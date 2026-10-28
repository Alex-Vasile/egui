//! Surface policy: turns an app-level [`SurfacePreference`] into a concrete
//! per-viewport [`SurfaceTarget`] and validates it into a
//! [`wgpu::SurfaceConfiguration`].
//!
//! Everything here is GPU-free and pure: the decider only reads
//! [`wgpu::SurfaceCapabilities`], the builder only constructs the
//! configuration the caller then passes to [`wgpu::Surface::configure`].

use crate::SurfaceConfig;
use crate::WgpuError;

/// How far beyond plain SDR sRGB the surface should go, requested at create
/// time on [`crate::WgpuConfiguration`].
///
/// Each variant names a *capability*, not a color space: the decider
/// ([`decide_surface_target`]) maps it to the best concrete surface pairing
/// the platform actually reports. Ungrantable means plain SDR with a
/// [`SurfaceStatus::FellBack`] status, never an error and never a partial
/// grant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfacePreference {
    /// Today's behavior: a plain 8-bit sRGB surface. The rollout default.
    #[default]
    Off,

    /// Wider gamut (Display P3 today), standard dynamic range.
    ///
    /// Falls back to the [`SurfacePreference::Off`] pairing (with
    /// [`FallbackReason::NoPairedColorSpace`]) when no wide color space is
    /// reported on a supported format.
    WideGamut,

    /// HDR brightness with the sRGB gamut pinned: extended-range sRGB, no
    /// saturation claim.
    ///
    /// Falls back to the [`SurfacePreference::Off`] pairing (with
    /// [`FallbackReason::NoPairedColorSpace`]) when no extended-range color
    /// space is reported on a supported float format.
    Hdr,

    /// Wide gamut *and* HDR brightness, all-or-nothing: the combined pairing
    /// or plain SDR, never one axis alone.
    ///
    /// Falls back to the [`SurfacePreference::Off`] pairing (with
    /// [`FallbackReason::NoPairedColorSpace`]) when no extended-range wide
    /// color space is reported on a supported float format.
    WideGamutHdr,
}

/// The concrete surface pairing a viewport decided on.
///
/// Answers *what is configured*; pair it with [`SurfaceStatus`], which
/// answers *was the request honored*.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceTarget {
    /// The swapchain texture format.
    pub format: wgpu::TextureFormat,

    /// The color space the presentation engine interprets the surface values in.
    pub color_space: wgpu::SurfaceColorSpace,
}

impl SurfaceTarget {
    /// Can this surface display values above 1.0?
    ///
    /// True iff the color space is one of the extended-range spaces
    /// (`ExtendedSrgbLinear`, `ExtendedSrgb`, `ExtendedDisplayP3`). Derived,
    /// never stored. Wide-gamut SDR pairings (`Bgra8Unorm` +
    /// [`DisplayP3`](wgpu::SurfaceColorSpace::DisplayP3)) are *not* extended.
    #[must_use]
    pub const fn is_extended(&self) -> bool {
        matches!(
            self.color_space,
            wgpu::SurfaceColorSpace::ExtendedSrgbLinear
                | wgpu::SurfaceColorSpace::ExtendedSrgb
                | wgpu::SurfaceColorSpace::ExtendedDisplayP3
        )
    }

    /// The ITU-T H.273 cICP code points (color primaries, transfer function,
    /// matrix coefficients, full-range flag) a PNG dump of this surface's
    /// pixels must carry so readers interpret them the way the presentation
    /// engine did.
    ///
    /// `Auto` maps to the sRGB points it resolves to on every surface this
    /// crate configures. cICP cannot express *extended* range (values beyond
    /// `[0, 1]`): extended-range surfaces aren't 8-bit-capturable anyway (the
    /// u8 capture path's `read_screen_rgba` only accepts `Rgba8Unorm` /
    /// `Bgra8Unorm`; lossless inspection goes through
    /// `CaptureState::read_screen_floats`), so their points are the base
    /// spaces.
    ///
    /// BT.2020 PQ/HLG points are included for completeness; no BT.2020
    /// surface pairing is granted yet.
    #[must_use]
    pub const fn cicp_code_points(&self) -> [u8; 4] {
        let (primaries, transfer) = match self.color_space {
            wgpu::SurfaceColorSpace::Auto
            | wgpu::SurfaceColorSpace::Srgb
            | wgpu::SurfaceColorSpace::ExtendedSrgb => (1, 13),
            wgpu::SurfaceColorSpace::DisplayP3 | wgpu::SurfaceColorSpace::ExtendedDisplayP3 => {
                (12, 13)
            }
            wgpu::SurfaceColorSpace::ExtendedSrgbLinear => (1, 8),
            wgpu::SurfaceColorSpace::Bt2100Pq => (9, 16),
            wgpu::SurfaceColorSpace::Bt2100Hlg => (9, 18),
        };
        // Matrix coefficients: identity (RGB). Full range: 1.
        [primaries, transfer, 0, 1]
    }
}

impl core::fmt::Display for SurfaceTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self {
            format,
            color_space,
        } = self;
        write!(f, "{format:?} + {color_space:?}")
    }
}

/// Whether the requested [`SurfacePreference`] was honored for a viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceStatus {
    /// The preference was granted.
    Granted,

    /// The preference was ungrantable; the surface fell back.
    ///
    /// Always a working surface, never an error.
    FellBack {
        /// Why the preference could not be granted.
        reason: FallbackReason,
    },
}

/// Why a [`SurfacePreference`] could not be granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FallbackReason {
    /// No surface reported the color space the preference needs on a
    /// supported format. Carries the request so bug reports are
    /// self-describing.
    NoPairedColorSpace {
        /// The preference that could not be granted.
        requested: SurfacePreference,
    },

    /// There is no surface (headless rendering).
    NoSurface,
}

/// Convert a surface status into the reduced mirror that egui's
/// `ViewportInfo` carries.
///
/// The rich `requested` payload of
/// [`FallbackReason::NoPairedColorSpace`] is dropped on purpose: the
/// requesting app knows its own preference, so the surfaced status only
/// needs to carry the outcome.
#[cfg(any(feature = "capture", feature = "winit"))]
impl From<SurfaceStatus> for egui::SurfaceStatus {
    fn from(status: SurfaceStatus) -> Self {
        match status {
            SurfaceStatus::Granted => Self::Granted,
            SurfaceStatus::FellBack { reason } => Self::FellBack(match reason {
                FallbackReason::NoPairedColorSpace { .. } => {
                    egui::FallbackReason::NoPairedColorSpace
                }
                FallbackReason::NoSurface => egui::FallbackReason::NoSurface,
            }),
        }
    }
}

/// Priority-ordered wide-gamut color spaces the
/// [`SurfacePreference::WideGamut`] leg probes, widest first.
///
/// Table data: add one row per newly supported primaries.
pub const WIDE_GAMUT_ORDER: &[wgpu::SurfaceColorSpace] = &[wgpu::SurfaceColorSpace::DisplayP3];

/// Priority-ordered extended-range color spaces the [`SurfacePreference::Hdr`]
/// leg probes.
///
/// [`ExtendedSrgb`](wgpu::SurfaceColorSpace::ExtendedSrgb) first: HDR pins the
/// sRGB gamut where the platform offers it. [`ExtendedSrgbLinear`] second: the
/// scRGB pairing, the Windows/DX12 anchor (plan §7) — the platform dictates
/// the space, so a platform that only reports extended-linear still grants
/// HDR through it (the renderer's linear path handles the encoding; PR 4).
/// Table data: add one row per newly supported extended space.
pub const HDR_ORDER: &[wgpu::SurfaceColorSpace] = &[
    wgpu::SurfaceColorSpace::ExtendedSrgb,
    wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
];

/// Priority-ordered combined wide-gamut + extended-range color spaces the
/// [`SurfacePreference::WideGamutHdr`] leg probes.
///
/// [`ExtendedDisplayP3`](wgpu::SurfaceColorSpace::ExtendedDisplayP3) first
/// (both axes in one space); [`ExtendedSrgbLinear`] second — scRGB is
/// wide-gamut-*capable* rather than wide-gamut-tagged (its sRGB primaries are
/// basis vectors, not a boundary, plan §7), so it is the honest combined
/// pairing where the platform reports nothing else. Platform-divergent
/// grammar is accepted: what macOS grants as `ExtendedDisplayP3`, DX12 grants
/// as `ExtendedSrgbLinear`. Table data: add one row per newly supported
/// combined space.
pub const WIDE_GAMUT_HDR_ORDER: &[wgpu::SurfaceColorSpace] = &[
    wgpu::SurfaceColorSpace::ExtendedDisplayP3,
    wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
];

/// Decide the surface target for one viewport from its actual capabilities.
///
/// The [`SurfacePreference::Off`] leg delegates to
/// [`crate::preferred_framebuffer_format`] and resolves wgpu's `Auto` color
/// space explicitly from the reported capabilities (`ExtendedSrgbLinear` for
/// `Rgba16Float` when reported, `Srgb` otherwise), so the default path stays
/// byte-identical to today's while never leaving `Auto` in a configuration.
///
/// The [`SurfacePreference::WideGamut`], [`SurfacePreference::Hdr`] and
/// [`SurfacePreference::WideGamutHdr`] legs each walk their priority-ordered
/// table ([`WIDE_GAMUT_ORDER`], [`HDR_ORDER`],
/// [`WIDE_GAMUT_HDR_ORDER`]) and grant the first pairing their format class
/// reports (raw-view 8-bit / float / float respectively); if none is reported
/// they return the `Off` target with a [`SurfaceStatus::FellBack`] status
/// instead. The walks read reported capabilities only, so probe-gating is
/// inherent — a platform that doesn't report the extended pair is never
/// assumed to have it.
///
/// # Errors
/// Returns [`WgpuError::NoSurfaceFormatsAvailable`] if the surface has no
/// formats at all. Everything else — including an ungrantable preference —
/// is a status, never an error.
pub fn decide_surface_target(
    caps: &wgpu::SurfaceCapabilities,
    pref: SurfacePreference,
) -> Result<(SurfaceTarget, SurfaceStatus), WgpuError> {
    match pref {
        SurfacePreference::Off => {
            resolve_off_target(caps).map(|target| (target, SurfaceStatus::Granted))
        }
        SurfacePreference::WideGamut => grant_or_fall_back(caps, pref, wide_gamut_pairing(caps)),
        SurfacePreference::Hdr => grant_or_fall_back(caps, pref, hdr_pairing(caps)),
        SurfacePreference::WideGamutHdr => {
            grant_or_fall_back(caps, pref, wide_gamut_hdr_pairing(caps))
        }
    }
}

/// The granted pairing, or the `Off` target with a [`SurfaceStatus::FellBack`]
/// status when the preference has no pairing on this surface.
fn grant_or_fall_back(
    caps: &wgpu::SurfaceCapabilities,
    pref: SurfacePreference,
    pairing: Option<(wgpu::TextureFormat, wgpu::SurfaceColorSpace)>,
) -> Result<(SurfaceTarget, SurfaceStatus), WgpuError> {
    if let Some((format, color_space)) = pairing {
        return Ok((
            SurfaceTarget {
                format,
                color_space,
            },
            SurfaceStatus::Granted,
        ));
    }

    let target = resolve_off_target(caps)?;
    Ok((
        target,
        SurfaceStatus::FellBack {
            reason: FallbackReason::NoPairedColorSpace { requested: pref },
        },
    ))
}

/// The `Off` pairing: [`crate::preferred_framebuffer_format`]'s format with
/// wgpu's `Auto` color space resolved explicitly.
fn resolve_off_target(caps: &wgpu::SurfaceCapabilities) -> Result<SurfaceTarget, WgpuError> {
    let format = crate::preferred_framebuffer_format(&caps.formats)?;

    // Replicates wgpu 30's `Auto` resolution from the reported capabilities:
    // `ExtendedSrgbLinear` for `Rgba16Float` when the surface supports it for
    // that format, `Srgb` otherwise.
    let color_space = if format == wgpu::TextureFormat::Rgba16Float
        && reported(
            caps.color_spaces(format),
            wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
        ) {
        wgpu::SurfaceColorSpace::ExtendedSrgbLinear
    } else {
        wgpu::SurfaceColorSpace::Srgb
    };

    Ok(SurfaceTarget {
        format,
        color_space,
    })
}

/// The `WideGamut` pairing probe: first wide color space reported on a
/// supported raw-view 8-bit format.
fn wide_gamut_pairing(
    caps: &wgpu::SurfaceCapabilities,
) -> Option<(wgpu::TextureFormat, wgpu::SurfaceColorSpace)> {
    pairing(caps, WIDE_GAMUT_ORDER, is_u8)
}

/// The `Hdr` pairing probe: first extended-range color space reported on a
/// supported float format.
fn hdr_pairing(
    caps: &wgpu::SurfaceCapabilities,
) -> Option<(wgpu::TextureFormat, wgpu::SurfaceColorSpace)> {
    pairing(caps, HDR_ORDER, is_f16)
}

/// The `WideGamutHdr` pairing probe: first combined wide-gamut extended-range
/// color space reported on a supported float format.
fn wide_gamut_hdr_pairing(
    caps: &wgpu::SurfaceCapabilities,
) -> Option<(wgpu::TextureFormat, wgpu::SurfaceColorSpace)> {
    pairing(caps, WIDE_GAMUT_HDR_ORDER, is_f16)
}

/// The pairing walk shared by all three preference legs: the first color
/// space in `order` reported on the first `format_filter` format.
///
/// Raw views only (`!format.is_srgb()`): a `*Srgb` format would decode and
/// then re-interpret the color space (double encoding), whatever the space.
fn pairing(
    caps: &wgpu::SurfaceCapabilities,
    order: &[wgpu::SurfaceColorSpace],
    format_filter: impl Fn(wgpu::TextureFormat) -> bool,
) -> Option<(wgpu::TextureFormat, wgpu::SurfaceColorSpace)> {
    for &color_space in order {
        for cap in &caps.format_capabilities {
            let format = cap.format;
            if format_filter(format) && !format.is_srgb() && reported(cap.color_spaces, color_space)
            {
                return Some((format, color_space));
            }
        }
    }
    None
}

/// The 8-bit formats egui renders to natively.
///
/// Table data: extend when egui renders to more 8-bit formats.
fn is_u8(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
    )
}

/// The float formats egui renders to natively: the HDR pairing class.
///
/// Table data: extend when egui renders to more float formats. Must stay in
/// sync with `renderer.rs`'s `is_float_target` (the dither-off target class)
/// — enforced by
/// [`float_target_tables_agree`](super::tests::float_target_tables_agree).
pub(crate) fn is_f16(format: wgpu::TextureFormat) -> bool {
    matches!(format, wgpu::TextureFormat::Rgba16Float)
}

/// Does a reported color-space set include `color_space`?
fn reported(color_spaces: wgpu::SurfaceColorSpaces, color_space: wgpu::SurfaceColorSpace) -> bool {
    color_space
        .to_color_spaces()
        .is_some_and(|flags| color_spaces.contains(flags))
}

/// Is this pairing coherent — does it encode exactly once?
///
/// A surface must use exactly one grammar: `*Srgb` formats encode in
/// hardware and demand [`Srgb`](wgpu::SurfaceColorSpace::Srgb) values, raw
/// `*Unorm` formats store already-encoded values. `Auto` is unresolved, not
/// a grammar.
fn is_coherent(format: wgpu::TextureFormat, color_space: wgpu::SurfaceColorSpace) -> bool {
    match color_space {
        wgpu::SurfaceColorSpace::Auto => false,
        wgpu::SurfaceColorSpace::Srgb => true,
        _ => !format.is_srgb(),
    }
}

/// Build a validated [`wgpu::SurfaceConfiguration`] from a decided
/// [`SurfaceTarget`].
///
/// Pure: no device, no surface. Unlike wgpu's `get_default_config` (which
/// seeds `Auto`) every decision is explicit, so a color space can never
/// silently resolve behind egui's back.
///
/// # Panics
/// In debug builds if the target is incoherent (`Auto` color space, a
/// double-grammar pairing) or not reported by `caps`.
pub fn build_configuration(
    caps: &wgpu::SurfaceCapabilities,
    target: &SurfaceTarget,
    size: [u32; 2],
    surface: &SurfaceConfig,
    alpha_mode: wgpu::CompositeAlphaMode,
) -> wgpu::SurfaceConfiguration {
    debug_assert!(
        target.color_space != wgpu::SurfaceColorSpace::Auto,
        "surface color space must be resolved explicitly, never `Auto`"
    );
    debug_assert!(
        is_coherent(target.format, target.color_space),
        "incoherent surface pairing {:?} + {:?}: a surface must encode exactly once",
        target.format,
        target.color_space
    );
    debug_assert!(
        reported(caps.color_spaces(target.format), target.color_space),
        "surface target {:?} + {:?} is not reported by the given capabilities",
        target.format,
        target.color_space
    );

    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: target.format,
        color_space: target.color_space,
        width: size[0],
        height: size[1],
        present_mode: surface.present_mode,
        desired_maximum_frame_latency: surface.desired_maximum_frame_latency.unwrap_or(2),
        alpha_mode,
        view_formats: vec![target.format],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SurfaceConfig;

    fn caps(
        formats: &[wgpu::TextureFormat],
        format_capabilities: &[(wgpu::TextureFormat, wgpu::SurfaceColorSpaces)],
    ) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats: formats.to_vec(),
            format_capabilities: format_capabilities
                .iter()
                .map(|&(format, color_spaces)| wgpu::SurfaceFormatCapabilities {
                    format,
                    color_spaces,
                })
                .collect(),
            ..wgpu::SurfaceCapabilities::default()
        }
    }

    /// Metal: 8-bit surface reporting the wide-gamut P3 space, plus the fp16
    /// surface reporting the extended pair — an EDR-capable panel.
    fn metal_like() -> wgpu::SurfaceCapabilities {
        caps(
            &[
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Rgba16Float,
            ],
            &[
                (
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::DISPLAY_P3,
                ),
                (
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::SurfaceColorSpaces::SRGB
                        | wgpu::SurfaceColorSpaces::EXTENDED_SRGB
                        | wgpu::SurfaceColorSpaces::EXTENDED_DISPLAY_P3,
                ),
            ],
        )
    }

    /// Metal on an SDR panel: wide gamut on 8-bit, no extended spaces at all.
    fn metal_like_sdr() -> wgpu::SurfaceCapabilities {
        caps(
            &[wgpu::TextureFormat::Bgra8Unorm],
            &[(
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::DISPLAY_P3,
            )],
        )
    }

    /// DXGI: float surface reporting the extended-linear space.
    fn dxgi_like() -> wgpu::SurfaceCapabilities {
        caps(
            &[wgpu::TextureFormat::Rgba16Float],
            &[(
                wgpu::TextureFormat::Rgba16Float,
                wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
            )],
        )
    }

    /// DX12 on a Windows 11 22H2+ advanced-color machine: the 8-bit surface
    /// reports sRGB only, the float surface reports the scRGB-linear space
    /// (the only extended space DXGI offers; plan §7). This is the fixture
    /// the scRGB pairing was built for.
    fn dx12_hdr_like() -> wgpu::SurfaceCapabilities {
        caps(
            &[
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Rgba16Float,
            ],
            &[
                (
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::SurfaceColorSpaces::SRGB,
                ),
                (
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
                ),
            ],
        )
    }

    /// Float surface that reports no extended space.
    fn float_without_extended() -> wgpu::SurfaceCapabilities {
        caps(
            &[wgpu::TextureFormat::Rgba16Float],
            &[(
                wgpu::TextureFormat::Rgba16Float,
                wgpu::SurfaceColorSpaces::SRGB,
            )],
        )
    }

    /// `*Srgb` format advertised with a wide space: the double-grammar trap.
    fn srgb_grammar_p3() -> wgpu::SurfaceCapabilities {
        caps(
            &[wgpu::TextureFormat::Bgra8UnormSrgb],
            &[(
                wgpu::TextureFormat::Bgra8UnormSrgb,
                wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::DISPLAY_P3,
            )],
        )
    }

    fn empty() -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities::default()
    }

    #[test]
    fn off_leg_matches_preferred_framebuffer_format() {
        for fixture in [
            metal_like(),
            metal_like_sdr(),
            dxgi_like(),
            dx12_hdr_like(),
            float_without_extended(),
            srgb_grammar_p3(),
        ] {
            let expected = crate::preferred_framebuffer_format(&fixture.formats).unwrap();

            let (target, status) = decide_surface_target(&fixture, SurfacePreference::Off).unwrap();

            assert_eq!(target.format, expected);
            assert_eq!(status, SurfaceStatus::Granted);
        }
    }

    #[test]
    fn off_resolves_auto_explicitly() {
        let (target, status) = decide_surface_target(&dxgi_like(), SurfacePreference::Off).unwrap();
        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
            }
        );
        assert_eq!(status, SurfaceStatus::Granted);

        let (target, _) = decide_surface_target(&metal_like(), SurfacePreference::Off).unwrap();
        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Bgra8Unorm,
                color_space: wgpu::SurfaceColorSpace::Srgb,
            }
        );

        let (target, _) =
            decide_surface_target(&float_without_extended(), SurfacePreference::Off).unwrap();
        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::Srgb,
            }
        );
    }

    #[test]
    fn empty_caps_error_for_every_preference() {
        for pref in [
            SurfacePreference::Off,
            SurfacePreference::WideGamut,
            SurfacePreference::Hdr,
            SurfacePreference::WideGamutHdr,
        ] {
            let err = decide_surface_target(&empty(), pref).unwrap_err();
            assert!(matches!(err, WgpuError::NoSurfaceFormatsAvailable));
        }
    }

    #[test]
    fn wide_gamut_grants_display_p3_on_metal_like() {
        let (target, status) =
            decide_surface_target(&metal_like(), SurfacePreference::WideGamut).unwrap();

        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Bgra8Unorm,
                color_space: wgpu::SurfaceColorSpace::DisplayP3,
            }
        );
        assert_eq!(status, SurfaceStatus::Granted);
        // Wide gamut, SDR range.
        assert!(!target.is_extended());
    }

    #[test]
    fn wide_gamut_falls_back_to_off_target_without_pairing() {
        for fixture in [dxgi_like(), float_without_extended()] {
            let (off_target, _) = decide_surface_target(&fixture, SurfacePreference::Off).unwrap();

            let (target, status) =
                decide_surface_target(&fixture, SurfacePreference::WideGamut).unwrap();

            assert_eq!(target, off_target);
            assert_eq!(
                status,
                SurfaceStatus::FellBack {
                    reason: FallbackReason::NoPairedColorSpace {
                        requested: SurfacePreference::WideGamut,
                    }
                }
            );
        }
    }

    #[test]
    fn wide_gamut_skips_double_grammar_pairing() {
        let (target, status) =
            decide_surface_target(&srgb_grammar_p3(), SurfacePreference::WideGamut).unwrap();

        assert_eq!(
            status,
            SurfaceStatus::FellBack {
                reason: FallbackReason::NoPairedColorSpace {
                    requested: SurfacePreference::WideGamut,
                }
            }
        );
        assert_eq!(target.format, wgpu::TextureFormat::Bgra8UnormSrgb);
        assert_eq!(target.color_space, wgpu::SurfaceColorSpace::Srgb);
    }

    #[test]
    fn hdr_grants_extended_srgb_on_fp16() {
        let (target, status) =
            decide_surface_target(&metal_like(), SurfacePreference::Hdr).unwrap();

        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::ExtendedSrgb,
            }
        );
        assert_eq!(status, SurfaceStatus::Granted);
        assert!(target.is_extended());
    }

    #[test]
    fn wide_gamut_hdr_grants_extended_display_p3_on_fp16() {
        let (target, status) =
            decide_surface_target(&metal_like(), SurfacePreference::WideGamutHdr).unwrap();

        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::ExtendedDisplayP3,
            }
        );
        assert_eq!(status, SurfaceStatus::Granted);
        assert!(target.is_extended());
    }

    /// The scRGB pairing (PR 4, plan §7): on a DX12-like fixture whose fp16
    /// surface reports `ExtendedSrgbLinear`, both HDR legs grant
    /// `(Rgba16Float, ExtendedSrgbLinear)` — HDR brightness and a
    /// wide-gamut-*capable* surface, probe-gated on the reported caps.
    #[test]
    fn hdr_and_wide_gamut_hdr_grant_scrgb_linear_on_dx12_like() {
        for pref in [SurfacePreference::Hdr, SurfacePreference::WideGamutHdr] {
            let (target, status) = decide_surface_target(&dx12_hdr_like(), pref).unwrap();

            assert_eq!(
                target,
                SurfaceTarget {
                    format: wgpu::TextureFormat::Rgba16Float,
                    color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
                },
                "{pref:?}"
            );
            assert_eq!(status, SurfaceStatus::Granted, "{pref:?}");
            assert!(target.is_extended(), "{pref:?}");
        }

        // The minimal DXGI fixture reports the same space, so it grants too.
        let (target, status) = decide_surface_target(&dxgi_like(), SurfacePreference::Hdr).unwrap();
        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
            }
        );
        assert_eq!(status, SurfaceStatus::Granted);
    }

    /// The `WideGamut` leg stays 8-bit raw-view only (§7): it cannot take the
    /// fp16 scRGB pairing, so a DX12-like machine falls back to the plain SDR
    /// target — never a partial grant.
    #[test]
    fn wide_gamut_still_falls_back_on_dx12_like() {
        let (off_target, _) =
            decide_surface_target(&dx12_hdr_like(), SurfacePreference::Off).unwrap();

        let (target, status) =
            decide_surface_target(&dx12_hdr_like(), SurfacePreference::WideGamut).unwrap();

        assert_eq!(target, off_target);
        assert!(!target.is_extended());
        assert_eq!(
            status,
            SurfaceStatus::FellBack {
                reason: FallbackReason::NoPairedColorSpace {
                    requested: SurfacePreference::WideGamut,
                }
            }
        );
    }

    #[test]
    fn hdr_wide_gamut_hdr_fall_back_to_off_target_without_pairing() {
        // No extended space anywhere (wide-gamut-only panel, or float without
        // extended spaces): both HDR legs must land on the Off-identical SDR
        // target, never a partial grant.
        for fixture in [metal_like_sdr(), float_without_extended()] {
            let (off_target, _) = decide_surface_target(&fixture, SurfacePreference::Off).unwrap();

            for pref in [SurfacePreference::Hdr, SurfacePreference::WideGamutHdr] {
                let (target, status) = decide_surface_target(&fixture, pref).unwrap();

                assert_eq!(target, off_target, "{pref:?}");
                assert!(!target.is_extended(), "{pref:?}");
                assert_eq!(
                    status,
                    SurfaceStatus::FellBack {
                        reason: FallbackReason::NoPairedColorSpace { requested: pref },
                    },
                    "{pref:?}"
                );
            }
        }
    }

    #[test]
    fn hdr_does_not_take_the_wide_gamut_pairing_and_vice_versa() {
        // Axis separation: Hdr must not grant the wide-gamut *SDR* pairing
        // (u8 + DisplayP3), and WideGamut must not grant the extended *HDR*
        // pairing (fp16 + ExtendedDisplayP3) — each leg only walks its own
        // table over its own format class.
        let (_, status) = decide_surface_target(&metal_like_sdr(), SurfacePreference::Hdr).unwrap();
        assert!(matches!(status, SurfaceStatus::FellBack { .. }));

        let (target, status) =
            decide_surface_target(&metal_like(), SurfacePreference::WideGamut).unwrap();
        assert_eq!(
            target,
            SurfaceTarget {
                format: wgpu::TextureFormat::Bgra8Unorm,
                color_space: wgpu::SurfaceColorSpace::DisplayP3,
            }
        );
        assert_eq!(status, SurfaceStatus::Granted);
        assert!(
            !target.is_extended(),
            "WideGamut is SDR-range by definition"
        );
    }

    #[test]
    fn decider_never_returns_an_incoherent_target() {
        for fixture in [
            metal_like(),
            metal_like_sdr(),
            dxgi_like(),
            dx12_hdr_like(),
            float_without_extended(),
            srgb_grammar_p3(),
        ] {
            for pref in [
                SurfacePreference::Off,
                SurfacePreference::WideGamut,
                SurfacePreference::Hdr,
                SurfacePreference::WideGamutHdr,
            ] {
                let (target, _) = decide_surface_target(&fixture, pref).unwrap();

                // Never `Auto` (so `dxgi_like` never yields `(Rgba16Float, Auto)`),
                // never a double-grammar pairing, and always reported by the caps.
                assert!(is_coherent(target.format, target.color_space));
                assert!(reported(
                    fixture.color_spaces(target.format),
                    target.color_space
                ));
            }
        }
    }

    #[test]
    fn build_configuration_is_coherent_and_threads_everything() {
        let surface = SurfaceConfig {
            present_mode: wgpu::PresentMode::Immediate,
            desired_maximum_frame_latency: Some(1),
        };

        for fixture in [
            metal_like(),
            metal_like_sdr(),
            dxgi_like(),
            dx12_hdr_like(),
            float_without_extended(),
            srgb_grammar_p3(),
        ] {
            for pref in [
                SurfacePreference::Off,
                SurfacePreference::WideGamut,
                SurfacePreference::Hdr,
                SurfacePreference::WideGamutHdr,
            ] {
                let (target, _) = decide_surface_target(&fixture, pref).unwrap();

                let config = build_configuration(
                    &fixture,
                    &target,
                    [1280, 720],
                    &surface,
                    wgpu::CompositeAlphaMode::PostMultiplied,
                );

                assert_eq!(config.format, target.format);
                assert_eq!(config.color_space, target.color_space);
                assert_ne!(config.color_space, wgpu::SurfaceColorSpace::Auto);
                assert!(
                    !(config.format.is_srgb()
                        && config.color_space != wgpu::SurfaceColorSpace::Srgb)
                );
                assert_eq!(config.usage, wgpu::TextureUsages::RENDER_ATTACHMENT);
                assert_eq!(config.view_formats, vec![target.format]);
                assert_eq!((config.width, config.height), (1280, 720));
                assert_eq!(config.present_mode, wgpu::PresentMode::Immediate);
                assert_eq!(config.desired_maximum_frame_latency, 1);
                assert_eq!(config.alpha_mode, wgpu::CompositeAlphaMode::PostMultiplied);
            }
        }

        // `None` latency threads wgpu's default of 2 (what `get_default_config` seeds).
        let surface_none = SurfaceConfig {
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: None,
        };
        let (target, _) = decide_surface_target(&metal_like(), SurfacePreference::Off).unwrap();
        let config = build_configuration(
            &metal_like(),
            &target,
            [1, 1],
            &surface_none,
            wgpu::CompositeAlphaMode::Opaque,
        );
        assert_eq!(config.desired_maximum_frame_latency, 2);
        assert_eq!(config.present_mode, wgpu::PresentMode::Fifo);
    }

    #[test]
    #[should_panic]
    fn build_configuration_rejects_double_grammar_pairing() {
        // `Bgra8UnormSrgb` + `DisplayP3` decodes then re-interprets: double encoding.
        let fixture = srgb_grammar_p3();
        let target = SurfaceTarget {
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            color_space: wgpu::SurfaceColorSpace::DisplayP3,
        };
        build_configuration(
            &fixture,
            &target,
            [1, 1],
            &SurfaceConfig::LOW_LATENCY,
            wgpu::CompositeAlphaMode::Opaque,
        );
    }

    #[test]
    #[should_panic]
    fn build_configuration_rejects_target_not_reported_by_caps() {
        // Same fixture, but the wide pairing is not reported: membership must fire.
        let fixture = caps(
            &[wgpu::TextureFormat::Bgra8Unorm],
            &[(
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::SurfaceColorSpaces::SRGB,
            )],
        );
        let target = SurfaceTarget {
            format: wgpu::TextureFormat::Bgra8Unorm,
            color_space: wgpu::SurfaceColorSpace::DisplayP3,
        };
        build_configuration(
            &fixture,
            &target,
            [1, 1],
            &SurfaceConfig::LOW_LATENCY,
            wgpu::CompositeAlphaMode::Opaque,
        );
    }

    #[test]
    #[should_panic]
    fn build_configuration_rejects_auto_color_space() {
        let fixture = metal_like();
        let target = SurfaceTarget {
            format: wgpu::TextureFormat::Bgra8Unorm,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        build_configuration(
            &fixture,
            &target,
            [1, 1],
            &SurfaceConfig::LOW_LATENCY,
            wgpu::CompositeAlphaMode::Opaque,
        );
    }

    #[test]
    fn surface_target_display_is_format_plus_color_space() {
        for (target, expected) in [
            (
                SurfaceTarget {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    color_space: wgpu::SurfaceColorSpace::DisplayP3,
                },
                "Bgra8Unorm + DisplayP3",
            ),
            (
                SurfaceTarget {
                    format: wgpu::TextureFormat::Rgba16Float,
                    color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
                },
                "Rgba16Float + ExtendedSrgbLinear",
            ),
            (
                SurfaceTarget {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    color_space: wgpu::SurfaceColorSpace::Srgb,
                },
                "Bgra8Unorm + Srgb",
            ),
        ] {
            assert_eq!(target.to_string(), expected);
        }
    }

    /// The two float-target tables (the HDR pairing class in this module and
    /// the dither-off class in `renderer.rs`) must name the same formats: a
    /// pairing granted by one must be exactly the set whose pipelines dither
    /// not. Extend both together — this test fails if only one grows.
    #[test]
    fn float_target_tables_agree() {
        for format in [
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureFormat::Rg16Float,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgba8Uint,
            wgpu::TextureFormat::Depth32Float,
        ] {
            assert_eq!(
                is_f16(format),
                crate::renderer::is_float_target(format),
                "{format:?}: the HDR pairing class and the dither-off class diverged"
            );
        }
    }

    #[cfg(any(feature = "capture", feature = "winit"))]
    #[test]
    fn status_conversion_into_egui_mirror() {
        for (status, expected) in [
            (SurfaceStatus::Granted, egui::SurfaceStatus::Granted),
            (
                SurfaceStatus::FellBack {
                    reason: FallbackReason::NoPairedColorSpace {
                        requested: SurfacePreference::WideGamut,
                    },
                },
                egui::SurfaceStatus::FellBack(egui::FallbackReason::NoPairedColorSpace),
            ),
            (
                SurfaceStatus::FellBack {
                    reason: FallbackReason::NoSurface,
                },
                egui::SurfaceStatus::FellBack(egui::FallbackReason::NoSurface),
            ),
        ] {
            assert_eq!(egui::SurfaceStatus::from(status), expected);
        }
    }
}
