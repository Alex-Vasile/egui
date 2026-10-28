//! The extended-range (`Rgba16Float`) offscreen target through the harness:
//! `WgpuTestRenderer::with_options_and_target` renders into fp16, the
//! readback decodes it via `egui_wgpu::readback`, and the u8 snapshot image
//! is the clamp-converted view (values above 1.0 saturate to white — the u8
//! domain is blind to HDR values; the numbers themselves are asserted by
//! egui-wgpu's float-readback tests).
//!
//! The canvas is 64 px wide — past the 32 px where a u8-pitch readback of an
//! fp16 target would fail validation — so a regression to the 8-bit pitch
//! cannot pass silently.

#![cfg(feature = "snapshot")]
#![cfg(feature = "wgpu")]

use egui::Vec2;
use egui_kittest::{Harness, wgpu};
use image::Rgba;

const OPAQUE_RED: Rgba<u8> = Rgba([255, 0, 0, 255]);

#[test]
fn fp16_offscreen_target_renders_and_reads_back() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(64.0, 64.0))
        .renderer(wgpu::WgpuTestRenderer::with_options_and_target(
            egui_wgpu::RendererOptions::PREDICTABLE,
            egui_wgpu::wgpu::TextureFormat::Rgba16Float,
            1,
        ))
        .build_ui(|ui| {
            ui.painter().rect_filled(
                ui.ctx().content_rect(),
                0.0,
                egui::Color32::from_rgb(255, 0, 0),
            );
        });

    let image = harness.render().expect("the fp16 render should succeed");

    // Every pixel is opaque red: 1.0 decodes exactly and maps to 255.
    for (x, y) in [(8, 8), (56, 8), (32, 32), (8, 56), (56, 56)] {
        assert_eq!(
            image.get_pixel(x, y),
            &OPAQUE_RED,
            "pixel ({x}, {y}) must be opaque red through the fp16 target + float readback"
        );
    }
}
