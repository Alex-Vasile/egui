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

/// Decide the surface target for one viewport from its actual capabilities.
///
/// The [`SurfacePreference::Off`] leg delegates to
/// [`crate::preferred_framebuffer_format`] and resolves wgpu's `Auto` color
/// space explicitly from the reported capabilities (`ExtendedSrgbLinear` for
/// `Rgba16Float` when reported, `Srgb` otherwise), so the default path stays
/// byte-identical to today's while never leaving `Auto` in a configuration.
///
/// The [`SurfacePreference::WideGamut`] leg walks [`WIDE_GAMUT_ORDER`] and
/// grants the first raw-view 8-bit format paired with a wide color space;
/// if none is reported it returns the `Off` target with a
/// [`SurfaceStatus::FellBack`] status instead.
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
        SurfacePreference::WideGamut => {
            if let Some((format, color_space)) = wide_gamut_pairing(caps) {
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
    }
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
    for &color_space in WIDE_GAMUT_ORDER {
        for cap in &caps.format_capabilities {
            let format = cap.format;
            // Raw-view 8-bit only: `*Srgb` formats would decode and then
            // re-interpret a wide color space (double encoding).
            if is_u8(format) && !format.is_srgb() && reported(cap.color_spaces, color_space) {
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

    /// Metal: 8-bit surface reporting the wide-gamut P3 space.
    fn metal_like() -> wgpu::SurfaceCapabilities {
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
            dxgi_like(),
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
        for pref in [SurfacePreference::Off, SurfacePreference::WideGamut] {
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
    fn decider_never_returns_an_incoherent_target() {
        for fixture in [
            metal_like(),
            dxgi_like(),
            float_without_extended(),
            srgb_grammar_p3(),
        ] {
            for pref in [SurfacePreference::Off, SurfacePreference::WideGamut] {
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
            dxgi_like(),
            float_without_extended(),
            srgb_grammar_p3(),
        ] {
            for pref in [SurfacePreference::Off, SurfacePreference::WideGamut] {
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
