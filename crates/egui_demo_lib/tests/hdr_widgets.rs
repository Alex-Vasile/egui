//! Kittest coverage for the HDR editing widgets (ladder row 5):
//! `color_edit_button_hdr` + `swatch_hdr` on the headless (sRGB) target.
//!
//! Appearance here is the SDR clamp (plan §7): float values above 1.0 clip
//! at SDR reference white in an 8-bit sRGB capture. The true >1.0
//! appearance (brightness above white on an extended-range surface) is the
//! manual EDR check in `MANUAL_TESTING.md` — 8-bit goldens cannot express it.

use egui::accesskit::Role;
use egui::{HdrColor, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

/// An orange whose red channel is *at* SDR white and would be brighter
/// still on an extended-range surface (rgb 0.5 at +2 stops = 2.0 linear).
fn test_color() -> HdrColor {
    HdrColor::new([0.5, 0.25, 0.0625], 2.0) // effective linear [2.0, 1.0, 0.25]
}

/// The closed button shows the color through the float-texture path; on
/// this sRGB target the >1.0 channels clamp (R 2.0 → white level): a
/// clamped yellow, not the true color.
#[test]
fn hdr_edit_button_closed_shows_clamped_color() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(120.0, 48.0))
        .build_ui_state(
            |ui, hdr: &mut HdrColor| {
                ui.color_edit_button_hdr(hdr);
            },
            test_color(),
        );
    harness.run();
    harness.fit_contents();
    harness.snapshot("hdr_widgets/edit_button_closed");
}

/// Clicking the button opens the HDR picker: linear R/G/B sliders, the
/// exposure slider, the true-vs-clamped swatch preview and the live
/// numeric readout of the effective color.
#[test]
fn hdr_edit_button_opened_shows_picker() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(360.0, 180.0))
        .build_ui_state(
            |ui, hdr: &mut HdrColor| {
                ui.color_edit_button_hdr(hdr);
            },
            test_color(),
        );
    harness.run();

    // Kittest interacts: open the popup like a user would.
    harness
        .get_by_role_and_label(Role::ColorWell, "HDR Color")
        .click();
    harness.run();

    // The popup is open: its live readout is in the accessibility tree,
    // and opening alone did not change the color.
    assert!(
        harness
            .query_by_label_contains("effective linear")
            .is_some()
    );
    assert_eq!(harness.state().exposure_stops, 2.0);
    assert_eq!(harness.state().rgb, [0.5, 0.25, 0.0625]);

    harness.snapshot("hdr_widgets/edit_button_opened");
}

/// Dragging the exposure slider must move the ¼-stop-quantized store.
///
/// Regression: the slider response is a bare interact — `changed` is never
/// derived on its own — so the picker's exposure gate never fired, the store
/// stayed put, and every frame re-seeded the handle from the stale value
/// (the readout stayed at ×1.00 and the handle sprang back after every
/// drag).
#[test]
fn hdr_exposure_slider_drag_updates_stored_stops() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(360.0, 260.0))
        .build_ui_state(
            |ui, hdr: &mut HdrColor| {
                ui.color_edit_button_hdr(hdr);
            },
            test_color(),
        );
    harness.run();
    harness
        .get_by_role_and_label(Role::ColorWell, "HDR Color")
        .click();
    harness.run();

    // Drag the exposure ramp from 10% (≈ −4.8 stops) to 90% (≈ +4.8).
    let rect = harness
        .get_by_role_and_label(Role::Slider, "Exposure (¼-stop)")
        .rect();
    let grab = egui::pos2(rect.left() + rect.width() * 0.1, rect.center().y);
    let target = egui::pos2(rect.left() + rect.width() * 0.9, rect.center().y);
    harness.hover_at(grab);
    harness.run();
    harness.drag_at(grab);
    harness.run();
    harness.hover_at(target);
    harness.run();
    harness.drop_at(target);
    harness.run();

    // 90% of −6..=+6 is +4.8 stops → quantized to ¼-stop: +4.75. The
    // channel sliders must be untouched by the exposure drag.
    assert_eq!(harness.state().exposure_stops, 4.75);
    assert_eq!(harness.state().rgb, [0.5, 0.25, 0.0625]);
}

/// `swatch_hdr` on the headless sRGB target: the first swatch is entirely
/// within SDR range (linear [0.25, 0.125, 0.0625] — renders unchanged), the
/// second is 2× SDR white and clamps to white. This IS the clamp, in a
/// snapshot; on an EDR surface the second swatch would instead glow above
/// white.
#[test]
fn hdr_swatch_shows_clamped_color_on_sdr_target() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(120.0, 64.0))
        .build_ui(|ui| {
            ui.swatch_hdr(&HdrColor::new([0.25, 0.125, 0.0625], 0.0));
            ui.swatch_hdr(&HdrColor::new([0.5, 0.5, 0.5], 2.0)); // 2× SDR white
        });
    harness.run();

    // The swatches are their own accessibility nodes, labeled distinctly
    // from the edit button's "HDR Color" (both swatches share the label, so
    // query the plural form — the singular one asserts uniqueness).
    assert_eq!(
        harness
            .query_all_by_role_and_label(Role::ColorWell, "HDR color swatch")
            .count(),
        2
    );
    assert_eq!(
        harness
            .query_all_by_role_and_label(Role::ColorWell, "HDR Color")
            .count(),
        0
    );

    harness.run();
    harness.fit_contents();
    harness.snapshot("hdr_widgets/swatches_sdr_target");
}
