//! The manual protocol's instrument (plan §8): one window that shows
//! everything `MANUAL_TESTING.md` reads — a live surface readout and a menu
//! of deterministic patterns, so a human can screenshot a pattern and
//! compare it against the expected readings.
//!
//! Requests [`SurfacePreference::WideGamutHdr`] (the most capable pairing;
//! falls back honestly everywhere) and never animates: every pattern is a
//! fixed fill/rect/image, so two screenshots of the same pattern on the same
//! setup are comparable. Snapshot-free by design — CI compile-checks this
//! binary; only a human can judge a panel.
//!
//! Patterns:
//! - **Primaries 2×3** — the wide-gamut check grid from `hdr_wide_gamut`,
//!   rebuilt as `FloatImage` cells (the tagged-content upload path, no
//!   custom pipeline): top row the sRGB primaries converted into the
//!   granted space, bottom row the granted space's native primaries.
//!   Identical rows mean no wide gamut.
//! - **SDR sweep** — a 0..255 grayscale ramp through the u8
//!   [`egui::ColorImage`] path: sanity that plain content is untouched.
//! - **Float gradient** — a linear-light 0..4.0 ramp through
//!   [`egui::FloatImage`]: on an extended surface the right three quarters
//!   render as brightness above SDR white; on an SDR surface everything
//!   above the ¼ mark clips at white.
//! - **Swatches** — a row of fixed `swatch_hdr` samples plus one live
//!   `color_edit_button_hdr` (the PR 5 widgets, interactive).
//! - **White 1.0 / White 4.0** — two labeled white patches, the EDR
//!   discriminator: on an EDR display with headroom the right patch is
//!   visibly brighter; on an SDR display they clip identically.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![expect(rustdoc::missing_crate_level_docs)] // it's an example

use eframe::egui;
use eframe::egui_wgpu::SurfacePreference;

fn main() -> Result<(), eframe::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,naga=warn,wgpu_core=warn,wgpu_hal=warn"),
    )
    .init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 560.0]),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // The fork opts in unconditionally (plan §2). This is the most
            // capable pairing: extended range + wide gamut where the
            // platform reports them, deterministic SDR fallback elsewhere —
            // the readout below shows which happened.
            surface_preference: SurfacePreference::WideGamutHdr,
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "hdr_test_app",
        options,
        Box::new(|_cc| Ok(Box::new(HdrTestApp::default()) as Box<dyn eframe::App>)),
    )
}

/// The deterministic pattern menu. All patterns are §6-compliant
/// (fills/rects/images — no HDR strokes or text) and static.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Pattern {
    /// The sRGB-converted vs native primaries check grid.
    #[default]
    Primaries,
    /// Grayscale 0..255 ramp (u8 path sanity).
    SdrSweep,
    /// Linear-light 0..4.0 gradient (float path).
    FloatGradient,
    /// Fixed `swatch_hdr` samples + one live `color_edit_button_hdr`.
    Swatches,
    /// The EDR discriminator: white at 1.0 vs 4.0 side by side.
    WhitePatches,
}

impl Pattern {
    const ALL: [Self; 5] = [
        Self::Primaries,
        Self::SdrSweep,
        Self::FloatGradient,
        Self::Swatches,
        Self::WhitePatches,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Primaries => "Primaries 2×3",
            Self::SdrSweep => "SDR sweep",
            Self::FloatGradient => "Float gradient",
            Self::Swatches => "Swatches",
            Self::WhitePatches => "White 1.0 / White 4.0",
        }
    }
}

/// The color space the surface was actually granted, parsed from the
/// app-visible diagnostics string
/// ([`egui::ViewportInfo::surface_target`], e.g. "`Rgba16Float +
/// ExtendedDisplayP3`") — the same channel as `hdr_wide_gamut` /
/// `hdr_float_image` (PR 1's structured per-viewport decision is not
/// app-reachable; the diagnostics string is pairing-table data whose suffix
/// matches [`wgpu::SurfaceColorSpace`]'s `Debug` name 1:1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GrantedSpace {
    /// The fallback: bytes are re-read as ordinary sRGB.
    #[default]
    Srgb,
    /// The wide-gamut SDR grant: bytes are read as Display P3.
    DisplayP3,
    /// Extended-range sRGB: values above 1.0 are brightness, gamut pinned.
    ExtendedSrgb,
    /// Extended-range Display P3: the full `WideGamutHdr` pairing.
    ExtendedDisplayP3,
    /// The scRGB-linear pairing (Windows/DX12, KWin): extended range in
    /// linear storage, sRGB gamut.
    ExtendedSrgbLinear,
    /// Anything else (or no string at all): shown loudly, never guessed.
    Unrecognized,
}

impl GrantedSpace {
    fn from_target(target: Option<&str>) -> Self {
        match target.and_then(|t| t.rsplit_once(" + ").map(|(_, space)| space)) {
            Some("Srgb") => Self::Srgb,
            Some("DisplayP3") => Self::DisplayP3,
            Some("ExtendedSrgb") => Self::ExtendedSrgb,
            Some("ExtendedDisplayP3") => Self::ExtendedDisplayP3,
            Some("ExtendedSrgbLinear") => Self::ExtendedSrgbLinear,
            _ => Self::Unrecognized,
        }
    }

    /// What the granted pairing means for the colors on screen: one
    /// sentence, driven by the parsed space (the parsed-space readout
    /// pattern from `hdr_wide_gamut`).
    fn interpretation(self) -> &'static str {
        match self {
            Self::Srgb => {
                "SDR sRGB surface: colors are ordinary sRGB; values above 1.0 clip at white."
            }
            Self::DisplayP3 => {
                "Wide-gamut SDR surface: bytes are read as Display P3, so primaries outside \
                 sRGB are visible; values above 1.0 clip at white."
            }
            Self::ExtendedSrgb => {
                "Extended-range sRGB surface: values above 1.0 render as brightness above SDR \
                 white; the gamut stays sRGB."
            }
            Self::ExtendedDisplayP3 => {
                "Extended-range wide-gamut surface: primaries outside sRGB are visible AND \
                 values above 1.0 render as brightness above SDR white."
            }
            Self::ExtendedSrgbLinear => {
                "Extended-range linear (scRGB) surface: values above 1.0 render as brightness \
                 above SDR white; the gamut stays sRGB."
            }
            Self::Unrecognized => {
                "Granted space not recognized — read the patterns as raw bytes; no color \
                 interpretation is claimed."
            }
        }
    }

    /// The primaries `FloatImage` content must be tagged with to upload
    /// natively on this space (plan §4's pairing table).
    fn content_primaries(self) -> egui::Primaries {
        match self {
            Self::DisplayP3 | Self::ExtendedDisplayP3 => egui::Primaries::DisplayP3,
            Self::Srgb | Self::ExtendedSrgb | Self::ExtendedSrgbLinear | Self::Unrecognized => {
                egui::Primaries::Srgb
            }
        }
    }

    /// Is this space's gamut wider than sRGB?
    fn is_wide_gamut(self) -> bool {
        matches!(self, Self::DisplayP3 | Self::ExtendedDisplayP3)
    }
}

/// Linear-light conversion from sRGB into Display P3's primaries, per
/// column: column j maps sRGB primary j (red (1, 0, 0), green (0, 1, 0),
/// blue (0, 0, 1)). Published verbatim in CSS Color 4
/// ("predefined-display-p3"); rows sum to 1 because both systems share the
/// D65 white. (The P3 primaries are *defined* by their xy chromaticities —
/// (0.680, 0.320) etc. Those are CIE chromaticity-diagram coordinates, not
/// colors.)
fn srgb_to_p3_linear() -> [[f32; 3]; 3] {
    [
        [0.822_461_97, 0.177_538_03, 0.0],
        [0.033_194_2, 0.966_805_8, 0.0],
        [0.017_082_63, 0.072_397_44, 0.910_519_93],
    ]
}

#[derive(Default)]
struct HdrTestApp {
    pattern: Pattern,
    /// The first-frame decision is logged once: the `MANUAL_TESTING`
    /// protocol reads the log line OR the on-screen readout, and the log
    /// survives where screenshots may not be permitted.
    logged_decision: bool,
    /// The live color of the `color_edit_button_hdr` in the Swatches
    /// pattern.
    hdr_color: egui::HdrColor,
    /// The primaries grid, cached with the content primaries it was tagged
    /// with — rebuilt when the granted space's gamut changes (the window
    /// may have moved to a different display).
    primaries: Option<(egui::Primaries, egui::TextureHandle)>,
    sdr_sweep: Option<egui::TextureHandle>,
    float_gradient: Option<egui::TextureHandle>,
    white_100: Option<egui::TextureHandle>,
    white_400: Option<egui::TextureHandle>,
}

impl HdrTestApp {
    /// The primaries check grid as `FloatImage` cells: the top row is the
    /// sRGB primaries converted into the tagged primaries' space, the bottom
    /// row the tagged primaries' natives as full-range cells — the
    /// `hdr_wide_gamut` shader demo's semantics through the tagged-content
    /// upload path (no custom pipeline).
    fn primaries_image(space: GrantedSpace) -> egui::FloatImage {
        let (cell, height) = (96usize, 64usize);
        let primaries = space.content_primaries();
        // Per column: the linear sRGB primary as seen in the content
        // primaries' space (identity when the space is sRGB-referred).
        let m = if space.is_wide_gamut() {
            srgb_to_p3_linear()
        } else {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        };
        let srgb_primaries = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

        // The six cell colors: top row the sRGB primaries converted into
        // the content space, bottom row the content space's own primaries
        // at full range — on a P3-referred surface that IS the P3 primary,
        // outside the sRGB gamut.
        // The contract: pixels are gamma-encoded in the declared primaries
        // (full-range cells stay ≤ 1.0 here), premultiplied with alpha 1.0.
        let encode = |c: f32| egui::ecolor::gamma_from_linear(c);
        let mut cells = [[0.0f32; 4]; 6];
        for row in 0..2 {
            for col in 0..3 {
                let p = srgb_primaries[col];
                let linear = if row == 0 {
                    [
                        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2],
                        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2],
                        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2],
                    ]
                } else {
                    p
                };
                let [r, g, b] = linear.map(encode);
                cells[row * 3 + col] = [r, g, b, 1.0];
            }
        }

        let width = cell * 3;
        let mut rgba = Vec::with_capacity(width * height * 2 * 4);
        for y in 0..height * 2 {
            let row = y / height;
            for x in 0..width {
                rgba.extend(cells[row * 3 + x / cell]);
            }
        }
        egui::FloatImage::from_rgba([width, height * 2], primaries, &rgba)
    }

    /// The 0..255 grayscale ramp through the u8 path.
    fn sdr_sweep_image() -> egui::ColorImage {
        let (width, height) = (256usize, 24usize);
        let mut gray = Vec::with_capacity(width * height);
        for _ in 0..height {
            for x in 0..width {
                gray.push(x as u8);
            }
        }
        egui::ColorImage::from_gray([width, height], &gray)
    }

    /// A linear-light gradient 0..4.0, gamma-encoded per the contract and
    /// tagged sRGB: the left quarter is ordinary SDR range, the rest is
    /// clamped on SDR surfaces and brightness on extended ones.
    fn float_gradient_image() -> egui::FloatImage {
        let (width, height) = (256usize, 24usize);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for _ in 0..height {
            for x in 0..width {
                let linear = 4.0 * x as f32 / (width - 1) as f32;
                let encoded = egui::ecolor::gamma_from_linear(linear);
                rgba.extend([encoded, encoded, encoded, 1.0]);
            }
        }
        egui::FloatImage::from_rgba([width, height], egui::Primaries::Srgb, &rgba)
    }

    fn load_textures(&mut self, ctx: &egui::Context, space: GrantedSpace) {
        match self.pattern {
            Pattern::Primaries => {
                let primaries = space.content_primaries();
                if self
                    .primaries
                    .as_ref()
                    .is_none_or(|(tagged, _)| *tagged != primaries)
                {
                    let image = Self::primaries_image(space);
                    self.primaries = Some((
                        primaries,
                        ctx.load_texture(
                            "primaries_grid",
                            egui::ImageData::Float(std::sync::Arc::new(image)),
                            egui::TextureOptions::LINEAR,
                        ),
                    ));
                }
            }
            Pattern::SdrSweep => {
                if self.sdr_sweep.is_none() {
                    self.sdr_sweep = Some(ctx.load_texture(
                        "sdr_sweep",
                        egui::ImageData::Color(std::sync::Arc::new(Self::sdr_sweep_image())),
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            Pattern::FloatGradient => {
                if self.float_gradient.is_none() {
                    self.float_gradient = Some(ctx.load_texture(
                        "float_gradient",
                        egui::ImageData::Float(std::sync::Arc::new(Self::float_gradient_image())),
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            Pattern::Swatches => {}
            Pattern::WhitePatches => {
                if self.white_100.is_none() {
                    // 1.0 linear = SDR reference white.
                    let image = egui::FloatImage::from_linear([128, 128], [1.0; 3], 0.0);
                    self.white_100 = Some(ctx.load_texture(
                        "white_100",
                        egui::ImageData::Float(std::sync::Arc::new(image)),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                if self.white_400.is_none() {
                    // 1.0 linear at +2 stops = 4.0 = four times SDR white.
                    let image = egui::FloatImage::from_linear([128, 128], [1.0; 3], 2.0);
                    self.white_400 = Some(ctx.load_texture(
                        "white_400",
                        egui::ImageData::Float(std::sync::Arc::new(image)),
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }
    }

    /// The always-visible live readout panel.
    fn readout_panel(
        ui: &mut egui::Ui,
        status: Option<egui::SurfaceStatus>,
        target: Option<&str>,
        headroom: Option<f32>,
    ) {
        egui::Frame::group(ui.style())
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.heading("Surface readout");
                ui.label(format!(
                    "Surface status: {}",
                    status
                        .as_ref()
                        .map_or_else(|| "(not reported yet)".to_owned(), |s| format!("{s:?}"))
                ));
                ui.label(format!("Surface target: {}", target.unwrap_or("(unknown)")));
                ui.label(headroom_line(headroom));
                let space = GrantedSpace::from_target(target);
                ui.label(space.interpretation());
            });
    }
}

/// The headroom line: "×" multiplier formatting, `None` shown as the
/// unknown state it is (never fabricated).
fn headroom_line(headroom: Option<f32>) -> String {
    match headroom {
        Some(h) if h.is_finite() && h > 0.0 => format!("HDR headroom: {h:.2}× SDR white"),
        Some(h) => format!("HDR headroom: (implausible report {h:?})"),
        None => "HDR headroom: unknown (no report from the backend)".to_owned(),
    }
}

/// The fixed swatch row of the Swatches pattern: primaries at SDR white,
/// then white at 2× and 4×.
const SWATCH_SAMPLES: [egui::HdrColor; 5] = [
    egui::HdrColor::new([1.0, 0.0, 0.0], 0.0),
    egui::HdrColor::new([0.0, 1.0, 0.0], 0.0),
    egui::HdrColor::new([0.0, 0.0, 1.0], 0.0),
    egui::HdrColor::new([1.0, 1.0, 1.0], 1.0),
    egui::HdrColor::new([1.0, 1.0, 1.0], 2.0),
];

impl eframe::App for HdrTestApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (status, target, headroom) = ctx.input(|input| {
            let info = input.raw.viewports.get(&egui::ViewportId::ROOT);
            (
                info.and_then(|v| v.surface_status),
                info.and_then(|v| v.surface_target.clone()),
                info.and_then(|v| v.hdr_headroom),
            )
        });
        let space = GrantedSpace::from_target(target.as_deref());
        self.load_textures(&ctx, space);

        if !self.logged_decision {
            self.logged_decision = true;
            log::info!(
                "hdr_test_app surface decision: {} ({:?}), {}",
                target.as_deref().unwrap_or("(unknown)"),
                status,
                headroom_line(headroom)
            );
        }

        egui::CentralPanel::default().show(ui, |ui| {
            Self::readout_panel(ui, status, target.as_deref(), headroom);
            ui.add_space(8.0);

            ui.heading("Patterns");
            ui.horizontal(|ui| {
                for pattern in Pattern::ALL {
                    ui.radio_value(&mut self.pattern, pattern, pattern.label());
                }
            });
            ui.separator();

            match self.pattern {
                Pattern::Primaries => {
                    ui.label(
                        "The check: identical rows mean no wide gamut — the surface re-reads \
                         the bytes as-is. On a P3-referred surface the top row reads as the \
                         familiar sRGB primaries while the bottom row is the Display P3 \
                         primaries, outside the sRGB gamut.",
                    );
                    if let Some((_, texture)) = self.primaries.as_ref() {
                        ui.image(texture);
                    }
                    ui.label(
                        egui::RichText::new(
                            "top row: sRGB primaries — bottom row: display primaries",
                        )
                        .strong(),
                    );
                }
                Pattern::SdrSweep => {
                    ui.label("Grayscale 0 (left) → 255 (right), u8 ColorImage path:");
                    if let Some(texture) = self.sdr_sweep.as_ref() {
                        ui.image(texture);
                    }
                    ui.label("Expected: a smooth ramp, identical on every surface type.");
                }
                Pattern::FloatGradient => {
                    ui.label("Linear-light gradient 0.0 (left) → 4.0 (right), tagged sRGB:");
                    if let Some(texture) = self.float_gradient.as_ref() {
                        ui.image(texture);
                    }
                    ui.label(
                        "The ¼ mark is 1.0 = SDR white. Left of it: the ordinary SDR ramp. \
                         Right of it: clipped at white on an SDR surface; visibly brighter \
                         than white on an extended-range surface.",
                    );
                }
                Pattern::Swatches => {
                    ui.label(
                        "Fixed samples — red, green, blue at SDR white, then white at 2× and 4×:",
                    );
                    ui.horizontal(|ui| {
                        for color in &SWATCH_SAMPLES {
                            ui.swatch_hdr(color);
                        }
                    });
                    ui.add_space(8.0);
                    ui.label("Live editing (the PR 5 widgets):");
                    ui.horizontal(|ui| {
                        if ui.color_edit_button_hdr(&mut self.hdr_color).changed() {
                            ui.ctx().request_repaint();
                        }
                        ui.swatch_hdr(&self.hdr_color);
                        let [r, g, b] = self.hdr_color.effective_linear();
                        let stops = self.hdr_color.exposure_stops;
                        ui.monospace(format!(
                            "effective linear: R {r:.3}, G {g:.3}, B {b:.3}   ({stops:+.2} stops)"
                        ));
                    });
                    ui.label(
                        "On an extended surface the swatches show true brightness above SDR \
                         white; on an SDR surface they clamp at white (the picker's clamped \
                         preview makes the clamp legible).",
                    );
                }
                Pattern::WhitePatches => {
                    ui.label("Two white patches — left at 1.0, right at 4.0 (2 stops):");
                    ui.horizontal(|ui| {
                        if let Some(texture) = self.white_100.as_ref() {
                            ui.vertical(|ui| {
                                ui.image(texture);
                                ui.label("White 1.0 (SDR white)");
                            });
                        }
                        if let Some(texture) = self.white_400.as_ref() {
                            ui.vertical(|ui| {
                                ui.image(texture);
                                ui.label("White 4.0 (4× SDR white)");
                            });
                        }
                    });
                    ui.label(
                        "THE EDR DISCRIMINATOR: on an EDR display with headroom the right \
                         patch is visibly brighter than the left; on an SDR display they \
                         clip identically.",
                    );
                }
            }
        });
    }
}

/// Construction checks for the pattern builders — the instrument's
/// determinism guarantee is that every pattern image builds, for every
/// granted space, with the exact resolution the contract asserts. Run with
/// `cargo test -p eframe --example hdr_test_app`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primaries_image_builds_for_every_granted_space() {
        for space in [
            GrantedSpace::Srgb,
            GrantedSpace::DisplayP3,
            GrantedSpace::ExtendedSrgb,
            GrantedSpace::ExtendedDisplayP3,
            GrantedSpace::ExtendedSrgbLinear,
            GrantedSpace::Unrecognized,
        ] {
            let image = HdrTestApp::primaries_image(space);
            assert_eq!(image.resolution(), [96 * 3, 64 * 2]);
        }
    }

    #[test]
    fn primaries_grid_rows_differ_only_when_wide_gamut() {
        // The fallback tell: on an sRGB-referred space the two rows are
        // byte-identical; on a P3-referred one the bottom row is the P3
        // primaries (outside the sRGB gamut) and must differ.
        let green_cell = |space, row: usize| {
            let image = HdrTestApp::primaries_image(space);
            // The middle pixel of the middle (green) cell in grid row
            // `row` (each grid row is half the image height).
            let [w, h] = image.resolution();
            let y = row * (h / 2) + h / 4;
            image.pixels()[y * w + w / 2]
        };
        assert_eq!(
            green_cell(GrantedSpace::Srgb, 0),
            green_cell(GrantedSpace::Srgb, 1),
            "sRGB-referred space: the rows must be identical (the fallback tell)"
        );
        assert_ne!(
            green_cell(GrantedSpace::DisplayP3, 0),
            green_cell(GrantedSpace::DisplayP3, 1),
            "P3-referred space: converted sRGB green and native P3 green must differ"
        );
    }

    #[test]
    fn sdr_sweep_and_gradient_and_patches_build() {
        assert_eq!(HdrTestApp::sdr_sweep_image().resolution(), [256, 24]);
        assert_eq!(HdrTestApp::float_gradient_image().resolution(), [256, 24]);
        // The EDR discriminator: the two patches differ above SDR white.
        let one = egui::FloatImage::from_linear([8, 8], [1.0; 3], 0.0);
        let four = egui::FloatImage::from_linear([8, 8], [1.0; 3], 2.0);
        assert_ne!(
            one.pixels()[0],
            four.pixels()[0],
            "4.0 white must not equal 1.0 white"
        );
    }
}
