//! Kittest coverage for the HDR editing widgets (ladder row 5):
//! `color_edit_button_hdr` + `swatch_hdr` on the headless (sRGB) target.
//!
//! Appearance here is the SDR clamp (plan §7): float values above 1.0 clip
//! at SDR reference white in an 8-bit sRGB capture. The true >1.0
//! appearance (brightness above white on an extended-range surface) is the
//! manual EDR check in `MANUAL_TESTING.md` — 8-bit goldens cannot express it.

use egui::accesskit::Role;
use egui::{HdrColor, Vec2, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

/// An orange whose red channel is *at* SDR white and would be brighter
/// still on an extended-range surface (rgb 0.5 at +2 stops = 2.0 linear).
fn test_color() -> HdrColor {
    HdrColor::new([0.5, 0.25, 0.0625], 2.0) // effective linear [2.0, 1.0, 0.25]
}

/// Inject a display headroom into the harness's raw input, as a wgpu backend
/// would report it per frame via `ViewportInfo.hdr_headroom`. `RawInput::take`
/// preserves the viewport map between steps, so one injection serves the run.
fn inject_headroom(harness: &mut Harness<'_, HdrColor>, headroom: Option<f32>) {
    harness
        .input_mut()
        .viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .hdr_headroom = headroom;
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
    // The open picker keeps ~10 fps repaints alive (its headroom line), so an
    // unbounded run would never settle — step a fixed number of frames.
    harness.run_steps(3);

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

/// With a display headroom injected (what a wgpu backend reports per frame
/// on an extended-range surface), the opened picker shows the dashed
/// headroom line over the exposure ramp: at headroom 16.0 the boundary sits
/// at log2(16) = +4 stops, five sixths across the −6..=+6 ramp — clearly
/// right of the value marker at +2 stops. On this headless sRGB target the
/// ramp still renders clamped (float values > 1.0 clip in the 8-bit
/// capture); the golden pins the line's presence and position.
#[test]
fn hdr_edit_button_opened_shows_headroom_line() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(360.0, 180.0))
        .build_ui_state(
            |ui, hdr: &mut HdrColor| {
                ui.color_edit_button_hdr(hdr);
            },
            test_color(),
        );
    inject_headroom(&mut harness, Some(16.0));
    harness.run();

    harness
        .get_by_role_and_label(Role::ColorWell, "HDR Color")
        .click();
    // The open picker keeps ~10 fps repaints alive (its headroom line), so an
    // unbounded run would never settle — step a fixed number of frames.
    harness.run_steps(3);

    // The injected headroom is display state, not color state:
    assert_eq!(harness.state().exposure_stops, 2.0);
    assert_eq!(harness.state().rgb, [0.5, 0.25, 0.0625]);

    harness.snapshot("hdr_widgets/edit_button_opened_headroom_line");
}

/// Regression for settled-state repaint blindness: while the picker sits open
/// and idle, a headroom change must move the dashed line on the *next* frames
/// — the widget re-reads `hdr_headroom` every frame it draws (its own ~10 fps
/// repaint requests keep frames coming even though no window event fires).
#[test]
fn hdr_exposure_line_tracks_headroom_changes_while_idle() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(360.0, 180.0))
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
    harness.run_steps(3);

    let slider_rect = harness
        .get_by_role_and_label(Role::Slider, "Exposure (¼-stop)")
        .rect();

    // Headroom 1.0 = SDR white = 0 stops = the exact center of the ramp:
    inject_headroom(&mut harness, Some(1.0));
    harness.run_steps(2);
    let centered = harness.render().expect("render must succeed");

    // …and a changed headroom moves the line without any pointer event:
    inject_headroom(&mut harness, Some(16.0));
    harness.run_steps(2);
    let moved = harness.render().expect("render must succeed");

    let changed_columns: Vec<u32> = {
        let (w, _h) = centered.dimensions();
        let mut columns = std::collections::BTreeSet::new();
        for y in slider_rect.top() as u32..slider_rect.bottom() as u32 {
            for x in 0..w {
                if centered.get_pixel(x, y) != moved.get_pixel(x, y) {
                    columns.insert(x);
                }
            }
        }
        columns.into_iter().collect()
    };
    assert!(
        !changed_columns.is_empty(),
        "a headroom change must visibly move the line while the picker idles"
    );

    // The changed columns are the two line positions: fraction 0.5 (headroom
    // 1.0) and 5/6 (headroom 16.0) across the slider, within a few pixels for
    // the dash width and antialiasing.
    let expected = |fraction: f32| slider_rect.left() + fraction * slider_rect.width();
    for fraction in [0.5_f32, 16_f32.log2() / 12.0 + 0.5] {
        let want = expected(fraction);
        assert!(
            changed_columns
                .iter()
                .any(|&x| (x as f32 - want).abs() <= 3.0),
            "no changed column near the headroom fraction {fraction} (want x ≈ {want}), \
             changed columns at {changed_columns:?}"
        );
    }
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
    // The open picker keeps ~10 fps repaints alive (its headroom line), so an
    // unbounded run would never settle — step a fixed number of frames.
    harness.run_steps(3);

    // Drag the exposure ramp from 10% (≈ −4.8 stops) to 90% (≈ +4.8).
    let rect = harness
        .get_by_role_and_label(Role::Slider, "Exposure (¼-stop)")
        .rect();
    let grab = egui::pos2(rect.left() + rect.width() * 0.1, rect.center().y);
    let target = egui::pos2(rect.left() + rect.width() * 0.9, rect.center().y);
    harness.hover_at(grab);
    harness.run_steps(2);
    harness.drag_at(grab);
    harness.run_steps(2);
    harness.hover_at(target);
    harness.run_steps(2);
    harness.drop_at(target);
    harness.run_steps(2);

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
