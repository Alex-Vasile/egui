//! Tagged float content demo: `ImageData::Float` through `ui.image()`.
//!
//! Shows the two content↔surface pairing outcomes from a plain app's point
//! of view (plan §4):
//!
//! - an **sRGB-tagged** float gradient (values in [0, 1]) — uploads natively
//!   on every granted space (on a P3-referred surface it shifts exactly like
//!   the rest of the UI bytes);
//! - a **DisplayP3-tagged** pure-primary strip — uploads natively only when
//!   the surface was granted a Display P3 color space, where the strip shows
//!   primaries outside the sRGB gamut. On a fallback (sRGB) surface the
//!   upload is **loudly rejected**: the texture is never created, a warning
//!   logs once, and the placeholder renders — the empty bordered slot below.
//!
//! The surface decision is parsed from the `surface_target` diagnostics
//! string ([`egui::ViewportInfo::surface_target`], e.g.
//! "`Bgra8Unorm + DisplayP3`") — same channel as `hdr_wide_gamut`.
//!
//! Everything here stays SDR: the float values are in [0, 1] (1.0 = SDR
//! reference white). Displaying values above 1.0 is PR 3; this stage makes
//! wide-gamut *images* possible.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![expect(rustdoc::missing_crate_level_docs)] // it's an example

use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu::SurfacePreference;

fn main() -> Result<(), eframe::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,naga=warn,wgpu_core=warn,wgpu_hal=warn"),
    )
    .init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 420.0]),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // The fork opts in unconditionally (plan §2), so on a
            // P3-capable display the P3 strip gets its native pairing.
            surface_preference: SurfacePreference::WideGamut,
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "hdr_float_image",
        options,
        Box::new(|_cc| Ok(Box::new(FloatImageApp::default()) as Box<dyn eframe::App>)),
    )
}

#[derive(Default)]
struct FloatImageApp {
    srgb_gradient: Option<egui::TextureHandle>,
    p3_strip: Option<egui::TextureHandle>,
}

impl FloatImageApp {
    /// An sRGB-tagged float gradient, gamma-encoded, values in [0, 1]
    /// (1.0 = SDR reference white).
    fn load_srgb_gradient(&mut self, ctx: &egui::Context) {
        if self.srgb_gradient.is_some() {
            return;
        }
        let (width, height) = (256usize, 24usize);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for _ in 0..height {
            for x in 0..width {
                // The encoded level directly: already gamma-encoded per the
                // contract, fully opaque.
                let v = x as f32 / (width - 1) as f32;
                rgba.extend([v, v, v, 1.0]);
            }
        }
        let image = egui::FloatImage::from_rgba([width, height], egui::Primaries::Srgb, &rgba);
        self.srgb_gradient = Some(ctx.load_texture(
            "srgb_gradient",
            egui::ImageData::Float(Arc::new(image)),
            egui::TextureOptions::LINEAR,
        ));
    }

    /// A DisplayP3-tagged strip of the three pure P3 primaries (encoded in
    /// P3-referred gamma space, fully opaque).
    fn load_p3_strip(&mut self, ctx: &egui::Context) {
        if self.p3_strip.is_some() {
            return;
        }
        let (cell, height) = (64usize, 24usize);
        let primaries = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ];
        let mut rgba = Vec::with_capacity(cell * 3 * height * 4);
        for _ in 0..height {
            for primary in &primaries {
                for _ in 0..cell {
                    rgba.extend(*primary);
                }
            }
        }
        let image =
            egui::FloatImage::from_rgba([cell * 3, height], egui::Primaries::DisplayP3, &rgba);
        self.p3_strip = Some(ctx.load_texture(
            "p3_strip",
            egui::ImageData::Float(Arc::new(image)),
            egui::TextureOptions::LINEAR,
        ));
    }
}

impl eframe::App for FloatImageApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.load_srgb_gradient(&ctx);
        self.load_p3_strip(&ctx);

        let (status, target) = ctx.input(|input| {
            let info = input.raw.viewports.get(&egui::ViewportId::ROOT);
            (
                info.and_then(|v| v.surface_status),
                info.and_then(|v| v.surface_target.clone()),
            )
        });
        let p3_granted = target
            .as_deref()
            .and_then(|t| t.rsplit_once(" + ").map(|(_, space)| space))
            == Some("DisplayP3");

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Tagged float content (ImageData::Float)");
            ui.add_space(8.0);

            ui.label("sRGB-tagged gradient — native on every granted space:");
            bordered_slot(ui, [512.0, 24.0], |ui| {
                if let Some(texture) = self.srgb_gradient.as_ref() {
                    ui.image(texture);
                }
            });
            ui.add_space(8.0);

            ui.label("DisplayP3-tagged pure primaries — native only on a P3 surface:");
            bordered_slot(ui, [192.0, 24.0], |ui| {
                if let Some(texture) = self.p3_strip.as_ref() {
                    ui.image(texture);
                }
            });
            if p3_granted {
                ui.colored_label(
                    egui::Color32::LIGHT_GREEN,
                    "P3 surface granted: the strip shows primaries outside the sRGB gamut.",
                );
            } else {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    "sRGB surface: the strip's upload was loudly rejected — the slot above is the \
                     placeholder. A warning logged once in the console.",
                );
            }
            ui.add_space(8.0);

            ui.label(format!("Surface status: {status:?}"));
            ui.label(format!(
                "Surface target: {}",
                target.as_deref().unwrap_or("(unknown)")
            ));
            ui.label(
                "The gradient is gamma-encoded in its declared primaries, premultiplied, \
                 stored as f16 (Rgba16Float on the GPU). Values stay in [0,1] here — \
                 HDR brightness above SDR white is a later stage.",
            );
        });
    }
}

/// A fixed-size slot with a visible border, so a rejected upload renders as
/// an obviously-empty frame (the placeholder made visible).
fn bordered_slot(ui: &mut egui::Ui, size: [f32; 2], content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.set_min_size(egui::vec2(size[0], size[1]));
            content(ui);
        });
}
