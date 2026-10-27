//! Kittest coverage for tagged float content (plan §8, row 2b): an
//! sRGB-tagged `FloatImage` renders through `ui.image()` on the headless
//! (sRGB) target, and a DisplayP3-tagged one is loudly rejected there — the
//! upload never happens, so the standard placeholder renders. SDR-region
//! appearance only: float *values* above 1.0 are guarded by the numeric
//! upload round-trip tests in `egui-wgpu`, not by u8 goldens.

use egui::TextureOptions;
use egui_kittest::Harness;

/// An 8×8 sRGB-tagged float gradient, gamma-encoded, values in [0, 1]
/// (1.0 = SDR reference white), fully opaque.
fn srgb_gradient() -> egui::FloatImage {
    let mut rgba = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8 {
        for x in 0..8 {
            let v = (x * 8 + y) as f32 / 63.0;
            rgba.extend([v, v, v, 1.0]);
        }
    }
    egui::FloatImage::from_rgba([8, 8], egui::Primaries::Srgb, &rgba)
}

/// A 4×4 DisplayP3-tagged strip of pure encoded P3 red — a tag the headless
/// sRGB target cannot honor.
fn p3_red() -> egui::FloatImage {
    egui::FloatImage::filled([4, 4], egui::Primaries::DisplayP3, [1.0, 0.0, 0.0, 1.0])
}

/// The sRGB-tagged float image uploads (native on the sRGB target) and
/// displays through the plain `ui.image()` path.
#[test]
fn hdr_float_image_srgb_tagged_renders() {
    let mut texture: Option<egui::TextureHandle> = None;
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(80.0, 48.0))
        .build_ui(move |ui| {
            let texture = texture.get_or_insert_with(|| {
                ui.ctx().load_texture(
                    "srgb_gradient",
                    egui::ImageData::Float(std::sync::Arc::new(srgb_gradient())),
                    TextureOptions::LINEAR,
                )
            });
            ui.image(&*texture);
        });
    harness.run();
    harness.fit_contents();
    harness.snapshot("hdr_float_image/srgb_tagged_renders");
}

/// The DisplayP3-tagged float image is loudly rejected on the headless sRGB
/// target: no texture exists, so egui's standard placeholder renders — the
/// image slot shows the white backing frame, visibly empty. This IS the
/// reject path, in a snapshot.
#[test]
fn hdr_float_image_p3_tagged_rejected_on_srgb_target() {
    let mut texture: Option<egui::TextureHandle> = None;
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(80.0, 48.0))
        .build_ui(move |ui| {
            let texture = texture.get_or_insert_with(|| {
                ui.ctx().load_texture(
                    "p3_red",
                    egui::ImageData::Float(std::sync::Arc::new(p3_red())),
                    TextureOptions::LINEAR,
                )
            });
            // A white backing frame: if (and only if) the upload is rejected,
            // the snapshot shows the empty frame instead of the red strip.
            egui::Frame::new()
                .fill(egui::Color32::WHITE)
                .inner_margin(4.0)
                .show(ui, |ui| {
                    ui.image(&*texture);
                });
        });
    harness.run();
    harness.fit_contents();
    harness.snapshot("hdr_float_image/p3_tagged_rejected_placeholder");
}
