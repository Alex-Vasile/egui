//! Color picker widgets.

use crate::util::fixed_cache::FixedCache;
use crate::{
    Context, DragValue, Id, Painter, Popup, PopupCloseBehavior, Response, Role, Sense,
    TextureHandle, TextureId, Ui, Widget as _, WidgetInfo, epaint, lerp, remap_clamp,
};
use core::time::Duration;
use epaint::{
    ColorImage, CornerRadius, FloatImage, Rect, RectShape, RoundedRect, Shape, Stroke, StrokeKind,
    Vec2,
    ecolor::{
        Color32, Hsva, HsvaGamma, Rgba, gamma_from_linear, gamma_u8_from_linear_f32,
        linear_f32_from_linear_u8,
    },
    pos2,
    textures::TextureOptions,
    vec2,
};
use std::collections::HashMap;

fn contrast_color(color: impl Into<Rgba>) -> Color32 {
    if color.into().intensity() < 0.5 {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

/// Resolution of the color slider gradients: the textures have `N + 1` texels per dimension.
/// We need at least 6 for hues, and more for smooth 2D areas.
/// Should always be a multiple of 6 to hit the peak hues in HSV/HSL (every 60°).
const N: u32 = 6 * 6;

fn background_checkers(painter: &Painter, bounds: RoundedRect) {
    // Shrink slightly, so the dark checkers don't peek through
    // the antialiased edge of the color painted on top:
    let pixel_width = 1.0 / painter.ctx().pixels_per_point();
    let (rect, corner_radius) = bounds.shrink(pixel_width).into_parts();

    if !rect.is_positive() || !rect.is_finite() {
        return;
    }

    let dark_color = Color32::from_gray(32);
    let bright_color = Color32::from_gray(128);

    // A single repeating 2x2 checker tile, stretched so that
    // each checker is half the rect height, like before:
    let image = ColorImage::new(
        [2, 2],
        vec![bright_color, dark_color, dark_color, bright_color],
    );
    let texture = painter.ctx().load_texture_cached(
        "color_picker_checkers",
        Id::unique("color_picker_checkers"),
        image,
        TextureOptions::NEAREST_REPEAT,
    );

    // One tile (one uv unit) covers 2x2 checkers, i.e. the full rect height:
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(rect.width() / rect.height(), 1.0));
    painter
        .add(RectShape::filled(rect, corner_radius, Color32::WHITE).with_texture(texture.id(), uv));
}

/// Paint a gradient image over the given rounded rect, with an outline.
fn paint_gradient(ui: &Ui, id: Id, bounds: RoundedRect, outline: Stroke, image: ColorImage) {
    let (rect, corner_radius) = bounds.into_parts();
    let [width, height] = image.resolution();
    let texture = ui
        .ctx()
        .load_texture_cached("color_gradient", id, image, TextureOptions::LINEAR);

    // Inset the uv by half a texel, so the edges sample the exact edge colors:
    let inset = vec2(0.5 / width as f32, 0.5 / height as f32);
    let uv = Rect::from_min_max(inset.to_pos2(), pos2(1.0 - inset.x, 1.0 - inset.y));

    ui.painter()
        .add(RectShape::filled(rect, corner_radius, Color32::WHITE).with_texture(texture.id(), uv));

    // The outline must be a separate shape:
    // a textured `RectShape` cannot have a stroke,
    // since the stroke vertices would sample the same texture.
    ui.painter()
        .rect_stroke(rect, corner_radius, outline, StrokeKind::Inside);
}

/// Show a color with background checkers to demonstrate transparency (if any).
pub fn show_color(ui: &mut Ui, color: impl Into<Color32>, desired_size: Vec2) -> Response {
    show_color32(ui, color.into(), desired_size)
}

fn show_color32(ui: &mut Ui, color: Color32, desired_size: Vec2) -> Response {
    let (rect, response) = ui.allocate_at_least(desired_size, Sense::hover());
    if ui.is_rect_visible(rect) {
        show_color_at(ui.painter(), color, rect);
    }
    response
}

/// Show a color with background checkers to demonstrate transparency (if any).
pub fn show_color_at(painter: &Painter, color: Color32, rect: Rect) {
    if color.is_opaque() {
        painter.rect_filled(rect, 0.0, color);
    } else {
        // Transparent: how both the transparent and opaque versions of the color
        background_checkers(painter, rect.into());

        if color == Color32::TRANSPARENT {
            // There is no opaque version, so just show the background checkers
        } else {
            let left = Rect::from_min_max(rect.left_top(), rect.center_bottom());
            let right = Rect::from_min_max(rect.center_top(), rect.right_bottom());
            painter.rect_filled(left, 0.0, color);
            painter.rect_filled(right, 0.0, color.to_opaque());
        }
    }
}

/// Show a color with background checkers to demonstrate transparency (if any).
fn show_srgba_unmultiplied(ui: &mut Ui, srgba: [u8; 4], desired_size: Vec2) -> Response {
    let (rect, response) = ui.allocate_at_least(desired_size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let corner_radius = ui.visuals().widgets.noninteractive.corner_radius;
        show_srgba_unmultiplied_at(ui.painter(), srgba, RoundedRect::new(rect, corner_radius));
    }
    response
}

/// Show a color with background checkers to demonstrate transparency (if any).
fn show_srgba_unmultiplied_at(painter: &Painter, [r, g, b, a]: [u8; 4], bounds: RoundedRect) {
    let (rect, corner_radius) = bounds.into_parts();
    if a == 255 {
        painter.rect_filled(rect, corner_radius, Color32::from_rgb(r, g, b));
    } else {
        let left = Rect::from_min_max(rect.left_top(), rect.center_bottom());
        let right = Rect::from_min_max(rect.center_top(), rect.right_bottom());

        // Clamp to what the half-width rects can express,
        // so the checkers and both halves all round their corners the same:
        let corner_radius = corner_radius.at_most(0.5 * left.size().min_elem());

        background_checkers(painter, RoundedRect::new(rect, corner_radius));

        let left_corner_radius = corner_radius.with_east(0.0);
        let right_corner_radius = corner_radius.with_west(0.0);
        painter.rect_filled(
            left,
            left_corner_radius,
            Color32::from_rgba_unmultiplied(r, g, b, a),
        );
        painter.rect_filled(right, right_corner_radius, Color32::from_rgb(r, g, b));
    }
}

fn color_button(ui: &mut Ui, srgba: [u8; 4], open: bool) -> Response {
    let size = ui.spacing().interact_size;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(Role::ColorWell, ui.is_enabled(), "Color");
        let [r, g, b, a] = srgba;
        info.current_text_value = Some(format!("#{r:02x}{g:02x}{b:02x}{a:02x}"));
        info
    });

    if ui.is_rect_visible(rect) {
        let visuals = if open {
            &ui.visuals().widgets.open
        } else {
            ui.style().interact(&response)
        };
        let rect = rect.expand(visuals.expansion);

        let stroke_width = 1.0;
        let corner_radius = visuals.corner_radius;
        show_srgba_unmultiplied_at(
            ui.painter(),
            srgba,
            // Shrink both the rect and the corner radius,
            // so the fill arcs stay concentric with the inside stroke:
            RoundedRect::new(rect, corner_radius).shrink(stroke_width),
        );

        ui.painter().rect_stroke(
            rect,
            corner_radius,
            (stroke_width, visuals.bg_fill), // Using fill for stroke is intentional, because default style has no border
            StrokeKind::Inside,
        );
    }

    response
}

fn color_slider_1d(ui: &mut Ui, value: &mut f32, color_at: impl Fn(f32) -> Color32) -> Response {
    let desired_size = vec2(ui.spacing().slider_width, ui.spacing().interact_size.y);
    let (rect, mut response) = ui.allocate_at_least(desired_size, Sense::click_and_drag());

    if let Some(mpos) = response.interact_pointer_pos() {
        *value = remap_clamp(mpos.x, rect.left()..=rect.right(), 0.0..=1.0);
        // The value was written from the pointer: that IS a change. A bare
        // interact never derives `changed` on its own (unlike `Slider`,
        // nothing here compares old and new values), so callers gating on
        // `.changed()` would never see it.
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let corner_radius = visuals.corner_radius;
        let bounds = RoundedRect::new(rect, corner_radius);

        background_checkers(ui.painter(), bounds); // for alpha:

        {
            // fill color gradient:
            let width = N as usize + 1;
            let pixels = (0..width).map(|i| color_at(i as f32 / N as f32)).collect();
            let image = ColorImage::new([width, 1], pixels);
            paint_gradient(
                ui,
                response.id.with("gradient"),
                bounds,
                visuals.bg_stroke,
                image,
            );
        }

        {
            // Show where the slider is at:
            let x = lerp(rect.left()..=rect.right(), *value);
            let r = rect.height() / 4.0;
            let picked_color = color_at(*value);
            ui.painter().add(Shape::convex_polygon(
                vec![
                    pos2(x, rect.center().y),   // tip
                    pos2(x + r, rect.bottom()), // right bottom
                    pos2(x - r, rect.bottom()), // left bottom
                ],
                picked_color,
                Stroke::new(visuals.fg_stroke.width, contrast_color(picked_color)),
            ));
        }
    }

    response
}

/// The exposure slider of the HDR color picker: an HDR ramp swept over the
/// current color's exposure axis, with the dashed headroom-boundary line on top.
///
/// The slider body is the float texture from [`FloatImage::exposure_ramp`] (the
/// plan §6 carve-out covers this axis-aligned rect), painted across the slider
/// rect: on an extended-range surface the ramp is *brighter than SDR white*
/// where the display's headroom allows — the whole point of exposing the
/// exposure axis. The R/G/B channel sliders above it stay 8-bit `color_slider_1d`
/// gradients (their domains are 0..=1, nothing to show above white).
///
/// The texture is painted with the same mechanism as [`paint_gradient`] (a
/// textured [`RectShape`]) rather than the `Image` widget, so the ramp, the
/// interaction, and the accessibility node all share one rect: in a justified
/// layout the `Image` widget paints its aligned size but *reports* the full
/// justified rect, which would desync the pointer→stops mapping from the
/// visible ramp. It also keeps the ramp as wide as the R/G/B sliders beside it.
///
/// Interaction matches [`color_slider_1d`]: the pointer's x position maps to
/// stops across `MIN_EXPOSURE_STOPS..=MAX_EXPOSURE_STOPS`, the write marks the
/// response changed (a bare interact never derives `changed` on its own), and
/// the stored value is quantized to ¼ stops.
///
/// The headroom line is *realtime while the picker is open*: the value is
/// re-read from the current viewport's `ViewportInfo` every frame this widget
/// is drawn, and the widget schedules a ~10 fps repaint itself. That polling
/// is needed because PR 3's painter-side adaptive repaint only runs *on
/// paints*: once the headroom is steady the painter schedules nothing and egui
/// rests in `ControlFlow::Wait` — and a display-brightness change produces no
/// winit event, so without the widget's own requests the line would freeze
/// until an unrelated repaint. This widget only runs while the popup is open
/// ([`Popup::show`] runs its content only then), so the requests stop the
/// moment the picker closes and the app returns to its zero-cost rest; the
/// cost while editing is ~10 fps of a small popup window.
fn hdr_exposure_slider(ui: &mut Ui, hdr: &mut HdrColor) -> Response {
    let min = HdrColor::MIN_EXPOSURE_STOPS;
    let max = HdrColor::MAX_EXPOSURE_STOPS;
    let desired_size = vec2(ui.spacing().slider_width, ui.spacing().interact_size.y);
    let (rect, mut response) = ui.allocate_at_least(desired_size, Sense::click_and_drag());

    let mut exposure_norm = remap_clamp(hdr.exposure_stops, min..=max, 0.0..=1.0);
    if let Some(mpos) = response.interact_pointer_pos() {
        exposure_norm = remap_clamp(mpos.x, rect.left()..=rect.right(), 0.0..=1.0);
        let stops = lerp(min..=max, exposure_norm);
        hdr.exposure_stops = (stops / 0.25).round() * 0.25; // ¼-stop steps
        // The value was written from the pointer: that IS a change. A bare
        // interact never derives `changed` on its own (the 2a-era fix).
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);

        // The ramp: the cached float texture, linearly sampled across the
        // rect. It sweeps the current color over the exposure axis — it
        // depends on rgb but NOT on the current stops value, so the cached
        // texture is stable for the whole drag (no churn while dragging).
        // Same half-texel uv inset as `paint_gradient`, so the edges sample
        // the exact endpoint columns.
        let texture_id = exposure_ramp_texture_id(ui.ctx(), hdr.rgb, min, max);
        let inset_x = 0.5 / HDR_EXPOSURE_RAMP_WIDTH as f32;
        let uv = Rect::from_min_max(pos2(inset_x, 0.5), pos2(1.0 - inset_x, 0.5));
        ui.painter().add(
            RectShape::filled(rect, visuals.corner_radius, Color32::WHITE)
                .with_texture(texture_id, uv),
        );

        // The outline the other sliders have. A plain u8 stroke *over* a float
        // texture is fine — §6 forbids HDR-colored strokes, not strokes on top.
        // (A textured `RectShape` cannot carry its own stroke: the stroke
        // vertices would sample the same texture.)
        ui.painter().rect_stroke(
            rect,
            visuals.corner_radius,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );

        // Show where the slider is at (same marker as `color_slider_1d`, the
        // encoded color at the current stops — a vertex color caps at 1.0,
        // which is all a marker needs).
        let x = lerp(rect.left()..=rect.right(), exposure_norm);
        let factor = lerp(min..=max, exposure_norm).exp2();
        let picked_color = Color32::from_rgb(
            gamma_u8_from_linear_f32(hdr.rgb[0] * factor),
            gamma_u8_from_linear_f32(hdr.rgb[1] * factor),
            gamma_u8_from_linear_f32(hdr.rgb[2] * factor),
        );
        let r = rect.height() / 4.0;
        ui.painter().add(Shape::convex_polygon(
            vec![
                pos2(x, rect.center().y),   // tip
                pos2(x + r, rect.bottom()), // right bottom
                pos2(x - r, rect.bottom()), // left bottom
            ],
            picked_color,
            Stroke::new(visuals.fg_stroke.width, contrast_color(picked_color)),
        ));

        // The headroom boundary, live. `None` headroom (unknown) draws nothing.
        //
        // Keep frames flowing while the picker is open: the painter-side
        // adaptive repaint (PR 3) only polls *on paints*, so once the headroom
        // is steady nothing is scheduled and egui rests in `ControlFlow::Wait`
        // — and a brightness change arrives as no winit event at all. Without
        // this request the line would freeze until an unrelated repaint. This
        // code only runs while the popup is open, so the ~10 fps polling (and
        // its small cost) ends the moment the picker closes.
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        let headroom = ui.input(|input| input.viewport().hdr_headroom);
        if let Some(fraction) = headroom_line_fraction(headroom, min, max) {
            paint_headroom_line(ui, rect, fraction);
        }
    }

    response.widget_info(|| {
        // The value the screen reader announces is the edited quantity
        // (stops), not the slider's internal 0..1 normalization.
        WidgetInfo::slider(
            ui.is_enabled(),
            f64::from(lerp(min..=max, exposure_norm)),
            "Exposure (¼-stop)",
        )
    });

    response
}

/// The headroom boundary as a fraction across the exposure ramp
/// (`0.0` = left edge, `1.0` = right edge), or `None` for "no line".
///
/// `hdr_headroom` is the viewport's display headroom multiplier (1.0 = SDR
/// reference white). The ramp clips where its linear value exceeds the
/// headroom, i.e. at `stops = log2(headroom)`; this normalizes that stop over
/// the shown `stops_min..=stops_max` range.
///
/// Rules:
/// * `None` headroom means *unknown* (non-wgpu backend, no surface yet, or a
///   platform that reports nothing): no line — an unknown boundary is never
///   fabricated.
/// * A non-positive or NaN headroom is a broken report (never a valid
///   multiplier): clamped to the left edge — total over f32, never NaN.
/// * A boundary below the shown range (headroom ≤ `2^stops_min`) clamps to the
///   left edge: everything right of it clips, which is real information.
/// * A boundary at or above the shown range (headroom ≥ `2^stops_max`) hides the
///   line: nothing on the ramp clips, and a line hugging the right edge would
///   read as decoration rather than information. (Below the max the line is
///   clamped to 1.0, so it slides off the right edge as the headroom rises.)
pub(crate) fn headroom_line_fraction(
    hdr_headroom: Option<f32>,
    stops_min: f32,
    stops_max: f32,
) -> Option<f32> {
    let headroom = hdr_headroom?;
    if headroom.is_nan() || headroom <= 0.0 {
        // A non-positive (or NaN) headroom is a broken report, not an unknown:
        // `log2` of either is −inf/NaN, so clamp it to the left edge instead of
        // propagating NaN into draw coordinates.
        return Some(0.0);
    }
    if headroom >= stops_max.exp2() {
        return None;
    }
    Some(((headroom.log2() - stops_min) / (stops_max - stops_min)).clamp(0.0, 1.0))
}

/// The dashed headroom line over the ramp: a vertical line at `fraction` across
/// `rect`, drawn in two passes — a dark halo under a light core — so it reads
/// on both themes and over both the dim and the (potentially very bright) parts
/// of the ramp. Clipped to the ramp rect: the dashes never bleed outside.
fn paint_headroom_line(ui: &Ui, rect: Rect, fraction: f32) {
    let x = lerp(rect.left()..=rect.right(), fraction);
    let path = [pos2(x, rect.top()), pos2(x, rect.bottom())];
    let painter = ui.painter().with_clip_rect(rect);

    painter.extend(Shape::dashed_line(
        &path,
        Stroke::new(3.0, Color32::from_black_alpha(196)),
        4.0,
        4.0,
    ));
    painter.extend(Shape::dashed_line(
        &path,
        Stroke::new(1.0, Color32::WHITE),
        4.0,
        4.0,
    ));
}

/// # Arguments
/// * `x_value` - X axis, either saturation or value (0.0-1.0).
/// * `y_value` - Y axis, either saturation or value (0.0-1.0).
/// * `color_at` - A function that dictates how the mix of saturation and value will be displayed in the 2d slider.
///
/// e.g.: `|x_value, y_value| HsvaGamma { h: 1.0, s: x_value, v: y_value, a: 1.0 }.into()` displays the colors as follows:
/// * top-left: white `[s: 0.0, v: 1.0]`
/// * top-right: fully saturated color `[s: 1.0, v: 1.0]`
/// * bottom-right: black `[s: 0.0, v: 1.0].`
fn color_slider_2d(
    ui: &mut Ui,
    x_value: &mut f32,
    y_value: &mut f32,
    color_at: impl Fn(f32, f32) -> Color32,
) -> Response {
    let desired_size = Vec2::splat(ui.spacing().slider_width);
    let (rect, response) = ui.allocate_at_least(desired_size, Sense::click_and_drag());

    if let Some(mpos) = response.interact_pointer_pos() {
        *x_value = remap_clamp(mpos.x, rect.left()..=rect.right(), 0.0..=1.0);
        *y_value = remap_clamp(mpos.y, rect.bottom()..=rect.top(), 0.0..=1.0);
    }

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let corner_radius = visuals.corner_radius;

        {
            // fill color gradient:
            let width = N as usize + 1;
            let mut pixels = Vec::with_capacity(width * width);
            for yi in 0..=N {
                let yt = 1.0 - yi as f32 / (N as f32); // texel rows go from top to bottom
                for xi in 0..=N {
                    let xt = xi as f32 / (N as f32);
                    pixels.push(color_at(xt, yt));
                }
            }
            let image = ColorImage::new([width, width], pixels);
            paint_gradient(
                ui,
                response.id.with("gradient"),
                RoundedRect::new(rect, corner_radius),
                visuals.bg_stroke,
                image,
            );
        }

        // Show where the slider is at:
        let x = lerp(rect.left()..=rect.right(), *x_value);
        let y = lerp(rect.bottom()..=rect.top(), *y_value);
        let picked_color = color_at(*x_value, *y_value);
        ui.painter().add(epaint::CircleShape {
            center: pos2(x, y),
            radius: rect.width() / 12.0,
            fill: picked_color,
            stroke: Stroke::new(visuals.fg_stroke.width, contrast_color(picked_color)),
        });
    }

    response
}

/// We use a negative alpha for additive colors within this file (a bit ironic).
///
/// We use alpha=0 to mean "transparent".
fn is_additive_alpha(a: f32) -> bool {
    a < 0.0
}

/// What options to show for alpha
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Alpha {
    /// Set alpha to 1.0, and show no option for it.
    Opaque,

    /// Only show normal blend options for alpha.
    OnlyBlend,

    /// Show both blend and additive options.
    BlendOrAdditive,
}

fn color_picker_hsvag_2d(ui: &mut Ui, hsvag: &mut HsvaGamma, alpha: Alpha) {
    use crate::style::NumericColorSpace;

    let alpha_control = if is_additive_alpha(hsvag.a) {
        Alpha::Opaque // no alpha control for additive colors
    } else {
        alpha
    };

    match ui.style().visuals.numeric_color_space {
        NumericColorSpace::GammaByte => {
            let mut srgba_unmultiplied = Hsva::from(*hsvag).to_srgba_unmultiplied();
            let edited = srgba_edit_ui(ui, &mut srgba_unmultiplied, alpha_control);

            if edited.rgb {
                let alpha = hsvag.a;
                *hsvag = HsvaGamma::from(Hsva::from_srgb([
                    srgba_unmultiplied[0],
                    srgba_unmultiplied[1],
                    srgba_unmultiplied[2],
                ]));
                hsvag.a = alpha;
            }
            if edited.alpha {
                hsvag.a = linear_f32_from_linear_u8(srgba_unmultiplied[3]);
            }
        }

        NumericColorSpace::Linear => {
            let mut rgba_unmultiplied = Hsva::from(*hsvag).to_rgba_unmultiplied();
            let edited = rgba_edit_ui(ui, &mut rgba_unmultiplied, alpha_control);

            if edited.rgb {
                let alpha = hsvag.a;
                *hsvag = HsvaGamma::from(Hsva::from_rgb([
                    rgba_unmultiplied[0],
                    rgba_unmultiplied[1],
                    rgba_unmultiplied[2],
                ]));
                hsvag.a = alpha;
            }
            if edited.alpha {
                hsvag.a = rgba_unmultiplied[3];
            }
        }
    }

    let current_color_size = vec2(ui.spacing().slider_width, ui.spacing().interact_size.y);
    show_srgba_unmultiplied(
        ui,
        Hsva::from(*hsvag).to_srgba_unmultiplied(),
        current_color_size,
    )
    .on_hover_text("Selected color");

    if alpha == Alpha::BlendOrAdditive {
        let a = &mut hsvag.a;
        let mut additive = is_additive_alpha(*a);
        ui.horizontal(|ui| {
            ui.label("Blending:");
            ui.radio_value(&mut additive, false, "Normal");
            ui.radio_value(&mut additive, true, "Additive");

            if additive {
                *a = -a.abs();
            }

            if !additive {
                *a = a.abs();
            }
        });
    }

    let opaque = HsvaGamma { a: 1.0, ..*hsvag };

    let HsvaGamma { h, s, v, a: _ } = hsvag;

    if false {
        color_slider_1d(ui, s, |s| HsvaGamma { s, ..opaque }.into()).on_hover_text("Saturation");
    }

    if false {
        color_slider_1d(ui, v, |v| HsvaGamma { v, ..opaque }.into()).on_hover_text("Value");
    }

    color_slider_2d(ui, s, v, |s, v| HsvaGamma { s, v, ..opaque }.into());

    color_slider_1d(ui, h, |h| {
        HsvaGamma {
            h,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        }
        .into()
    })
    .on_hover_text("Hue");

    let additive = is_additive_alpha(hsvag.a);

    if alpha == Alpha::Opaque {
        hsvag.a = 1.0;
    } else {
        let a = &mut hsvag.a;

        if alpha == Alpha::OnlyBlend {
            if is_additive_alpha(*a) {
                *a = 0.5; // was additive, but isn't allowed to be
            }
            color_slider_1d(ui, a, |a| HsvaGamma { a, ..opaque }.into()).on_hover_text("Alpha");
        } else if !additive {
            color_slider_1d(ui, a, |a| HsvaGamma { a, ..opaque }.into()).on_hover_text("Alpha");
        }
    }
}

fn input_type_button_ui(ui: &mut Ui) {
    let mut input_type = ui.global_style().visuals.numeric_color_space;
    if input_type.toggle_button_ui(ui).changed() {
        ui.ctx().all_styles_mut(|s| {
            s.visuals.numeric_color_space = input_type;
        });
    }
}

/// Shows 4 `DragValue` widgets to be used to edit the RGBA u8 values.
/// Alpha's `DragValue` is hidden when `Alpha::Opaque`.
///
/// Returns which channels changed.
fn srgba_edit_ui(ui: &mut Ui, [r, g, b, a]: &mut [u8; 4], alpha: Alpha) -> ColorEditResult {
    let mut edited = ColorEditResult::default();

    ui.horizontal(|ui| {
        input_type_button_ui(ui);

        if ui
            .button("📋")
            .on_hover_text("Click to copy color values")
            .clicked()
        {
            if alpha == Alpha::Opaque {
                ui.copy_text(format!("{r}, {g}, {b}"));
            } else {
                ui.copy_text(format!("{r}, {g}, {b}, {a}"));
            }
        }
        edited.rgb |= DragValue::new(r).speed(0.5).prefix("R ").ui(ui).changed();
        edited.rgb |= DragValue::new(g).speed(0.5).prefix("G ").ui(ui).changed();
        edited.rgb |= DragValue::new(b).speed(0.5).prefix("B ").ui(ui).changed();
        if alpha != Alpha::Opaque {
            edited.alpha = DragValue::new(a).speed(0.5).prefix("A ").ui(ui).changed();
        }
    });

    edited
}

/// Shows 4 `DragValue` widgets to be used to edit the RGBA f32 values.
/// Alpha's `DragValue` is hidden when `Alpha::Opaque`.
///
/// Returns which channels changed.
fn rgba_edit_ui(ui: &mut Ui, [r, g, b, a]: &mut [f32; 4], alpha: Alpha) -> ColorEditResult {
    fn drag_value(ui: &mut Ui, prefix: &str, value: &mut f32) -> Response {
        DragValue::new(value)
            .speed(0.003)
            .prefix(prefix)
            .range(0.0..=1.0)
            .custom_formatter(|n, _| format!("{n:.03}"))
            .ui(ui)
    }

    let mut edited = ColorEditResult::default();

    ui.horizontal(|ui| {
        input_type_button_ui(ui);

        if ui
            .button("📋")
            .on_hover_text("Click to copy color values")
            .clicked()
        {
            if alpha == Alpha::Opaque {
                ui.copy_text(format!("{r:.03}, {g:.03}, {b:.03}"));
            } else {
                ui.copy_text(format!("{r:.03}, {g:.03}, {b:.03}, {a:.03}"));
            }
        }

        edited.rgb |= drag_value(ui, "R ", r).changed();
        edited.rgb |= drag_value(ui, "G ", g).changed();
        edited.rgb |= drag_value(ui, "B ", b).changed();
        if alpha != Alpha::Opaque {
            edited.alpha = drag_value(ui, "A ", a).changed();
        }
    });

    edited
}

#[derive(Default)]
struct ColorEditResult {
    rgb: bool,
    alpha: bool,
}

/// Shows a color picker where the user can change the given [`Hsva`] color.
///
/// Returns `true` on change.
pub fn color_picker_hsva_2d(ui: &mut Ui, hsva: &mut Hsva, alpha: Alpha) -> bool {
    let mut hsvag = HsvaGamma::from(*hsva);
    ui.vertical(|ui| {
        color_picker_hsvag_2d(ui, &mut hsvag, alpha);
    });
    let new_hasva = Hsva::from(hsvag);
    if *hsva == new_hasva {
        false
    } else {
        *hsva = new_hasva;
        true
    }
}

/// Shows a color picker where the user can change the given [`Color32`] color.
///
/// Returns `true` on change.
pub fn color_picker_color32(ui: &mut Ui, srgba: &mut Color32, alpha: Alpha) -> bool {
    let mut hsva = color_cache_get(ui.ctx(), *srgba);
    let changed = color_picker_hsva_2d(ui, &mut hsva, alpha);
    *srgba = Color32::from(hsva);
    color_cache_set(ui.ctx(), *srgba, hsva);
    changed
}

pub fn color_edit_button_hsva(ui: &mut Ui, hsva: &mut Hsva, alpha: Alpha) -> Response {
    let popup_id = ui.auto_id_with("popup");
    let open = Popup::is_id_open(ui.ctx(), popup_id);
    let mut button_response = color_button(ui, hsva.to_srgba_unmultiplied(), open);
    if ui.style().explanation_tooltips {
        button_response = button_response.on_hover_text("Click to edit color");
    }

    const COLOR_SLIDER_WIDTH: f32 = 275.0;

    Popup::menu(&button_response)
        .id(popup_id)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = COLOR_SLIDER_WIDTH;
            if color_picker_hsva_2d(ui, hsva, alpha) {
                button_response.mark_changed();
            }
        });

    button_response
}

/// Shows a button with the given color.
/// If the user clicks the button, a full color picker is shown.
pub fn color_edit_button_srgba(ui: &mut Ui, srgba: &mut Color32, alpha: Alpha) -> Response {
    let mut hsva = color_cache_get(ui.ctx(), *srgba);
    let response = color_edit_button_hsva(ui, &mut hsva, alpha);
    *srgba = Color32::from(hsva);
    color_cache_set(ui.ctx(), *srgba, hsva);
    response
}

/// Shows a button with the given color.
/// If the user clicks the button, a full color picker is shown.
/// The given color is in `sRGB` space.
pub fn color_edit_button_srgb(ui: &mut Ui, srgb: &mut [u8; 3]) -> Response {
    let mut srgba = Color32::from_rgb(srgb[0], srgb[1], srgb[2]);
    let response = color_edit_button_srgba(ui, &mut srgba, Alpha::Opaque);
    srgb[0] = srgba[0];
    srgb[1] = srgba[1];
    srgb[2] = srgba[2];
    response
}

/// Shows a button with the given color.
/// If the user clicks the button, a full color picker is shown.
pub fn color_edit_button_rgba(ui: &mut Ui, rgba: &mut Rgba, alpha: Alpha) -> Response {
    let mut hsva = color_cache_get(ui.ctx(), *rgba);
    let response = color_edit_button_hsva(ui, &mut hsva, alpha);
    *rgba = Rgba::from(hsva);
    color_cache_set(ui.ctx(), *rgba, hsva);
    response
}

/// Shows a button with the given color.
/// If the user clicks the button, a full color picker is shown.
pub fn color_edit_button_rgb(ui: &mut Ui, rgb: &mut [f32; 3]) -> Response {
    let mut rgba = Rgba::from_rgb(rgb[0], rgb[1], rgb[2]);
    let response = color_edit_button_rgba(ui, &mut rgba, Alpha::Opaque);
    rgb[0] = rgba[0];
    rgb[1] = rgba[1];
    rgb[2] = rgba[2];
    response
}

// To ensure we keep hue slider when `srgba` is gray we store the full [`Hsva`] in a cache:
fn color_cache_get(ctx: &Context, rgba: impl Into<Rgba>) -> Hsva {
    let rgba = rgba.into();
    use_color_cache(ctx, |cc| cc.get(&rgba).copied()).unwrap_or_else(|| Hsva::from(rgba))
}

// To ensure we keep hue slider when `srgba` is gray we store the full [`Hsva`] in a cache:
fn color_cache_set(ctx: &Context, rgba: impl Into<Rgba>, hsva: Hsva) {
    let rgba = rgba.into();
    use_color_cache(ctx, |cc| cc.set(rgba, hsva));
}

// To ensure we keep hue slider when `srgba` is gray we store the full [`Hsva`] in a cache:
fn use_color_cache<R>(ctx: &Context, f: impl FnOnce(&mut FixedCache<Rgba, Hsva>) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(Id::NULL)))
}

// ----------------------------------------------------------------------------

/// A linear-light RGB color plus an exposure in stops: egui's type for
/// editing and displaying colors **brighter than SDR reference white**.
///
/// The effective color is `rgb × 2^exposure_stops` per channel in linear
/// light, where `rgb` components live in `0..=1` (1.0 = SDR reference
/// white) and the exposure is clamped to
/// [`HdrColor::MAX_EXPOSURE_STOPS`] = ±6 (1.0 at +6 stops = 64× SDR white).
///
/// Display goes through the float-texture path ([`FloatImage`]): vertex
/// colors cap at 1.0 (plan §6 — stroked shapes and text cannot be HDR), so
/// a textured rectangle is the one §6-compliant way to show the true color.
/// On an extended-range surface it shows as brightness above SDR white; on
/// an SDR surface it clamps at white. Both are correct (plan §7).
///
/// Alpha is deliberately not part of this type: HDR translucency interacts
/// with the encoded-premultiply contract (the §4 attenuation), so HDR
/// colors are opaque in v1.
///
/// See [`color_edit_button_hdr`] and [`swatch_hdr`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct HdrColor {
    /// Linear-light components in `0..=1` (1.0 = SDR reference white).
    /// Values outside the range are clamped where they are consumed.
    pub rgb: [f32; 3],

    /// Exposure in stops (log₂ multiplier), clamped to ±6 when applied.
    pub exposure_stops: f32,
}

impl HdrColor {
    /// The largest exposure the widgets allow, in stops:
    /// 1.0 linear at `+MAX_EXPOSURE_STOPS` = 64× SDR reference white.
    pub const MAX_EXPOSURE_STOPS: f32 = 6.0;

    /// The smallest exposure the widgets allow, in stops.
    pub const MIN_EXPOSURE_STOPS: f32 = -Self::MAX_EXPOSURE_STOPS;

    pub const fn new(rgb: [f32; 3], exposure_stops: f32) -> Self {
        Self {
            rgb,
            exposure_stops,
        }
    }

    /// The effective linear-light color: `rgb × 2^exposure_stops`, with the
    /// exposure clamped to [`HdrColor::MIN_EXPOSURE_STOPS`]
    /// ..=[`HdrColor::MAX_EXPOSURE_STOPS`].
    pub fn effective_linear(&self) -> [f32; 3] {
        let stops = self
            .exposure_stops
            .clamp(Self::MIN_EXPOSURE_STOPS, Self::MAX_EXPOSURE_STOPS);
        let factor = stops.exp2();
        self.rgb.map(|c| c * factor)
    }

    /// The effective color as linear-light [`Rgba`] (alpha 1.0 —
    /// HDR colors are opaque; see the type documentation).
    pub fn effective_rgba(&self) -> Rgba {
        let [r, g, b] = self.effective_linear();
        Rgba::from_rgb(r, g, b)
    }

    /// The SDR appearance of this color: the effective linear value clamped
    /// to 1.0 per channel and gamma-encoded into [`Color32`] — what an SDR
    /// surface shows for it.
    pub fn clamped_color32(&self) -> Color32 {
        let [r, g, b] = self.effective_linear().map(|c| c.clamp(0.0, 1.0));
        Color32::from(Rgba::from_rgb(r, g, b))
    }
}

impl Default for HdrColor {
    fn default() -> Self {
        Self::new([0.0, 0.0, 0.0], 0.0)
    }
}

impl From<HdrColor> for epaint::HdrVertexColor {
    /// The HDR color as a vertex-borne value: `effective_linear` (exposure
    /// clamped to ±6 stops, `rgb` scaled, nothing per-channel clamped) fed
    /// through [`epaint::HdrVertexColor::from_linear`] with alpha exactly 1 —
    /// [`HdrColor`] is opaque by design.
    ///
    /// This is what makes `painter.rect_filled_hdr(rect, corner_radius,
    /// hdr_color)` and [`Ui::hdr_label`](crate::Ui::hdr_label) take a picker
    /// value without a hand-written conversion.
    fn from(color: HdrColor) -> Self {
        let [r, g, b] = color.effective_linear();
        Self::from_linear([r, g, b, 1.0])
    }
}

/// How many textures each HDR widget texture cache (the flat swatches and the
/// exposure ramps) keeps before the whole cache is cleared — dropping the
/// handles and with them the textures. There is no individual eviction; the
/// cap bounds the cache.
const HDR_TEXTURE_CACHE_CAP: usize = 256;

/// Texel width of a cached exposure ramp: fixed regardless of the on-screen
/// slider width (the texture is linearly sampled across whatever rect it is
/// stretched over), so a texture never churns with layout.
const HDR_EXPOSURE_RAMP_WIDTH: usize = 256;

/// Show a swatch of the true HDR color.
///
/// A rectangle of [`Ui::spacing`]'s interact size, displayed through the
/// float-texture path — on an extended-range surface this is the color
/// brighter than SDR reference white, on an SDR surface it clamps at white.
///
/// The tooltip shows the effective linear values.
///
/// See also [`color_edit_button_hdr`] for the editable version.
pub fn swatch_hdr(ui: &mut Ui, color: &HdrColor) -> Response {
    let size = vec2(ui.spacing().interact_size.x, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    // A distinct label from the edit button's: the swatch is not a second
    // copy of the same logical control (duplicate ColorWells with one label
    // would be indistinguishable to screen readers and ambiguous to query).
    response.widget_info(|| hdr_color_widget_info(ui, color, "HDR color swatch"));

    if ui.is_rect_visible(rect) {
        let corner_radius = ui.visuals().widgets.noninteractive.corner_radius;
        paint_hdr_swatch(ui, rect, corner_radius, color, Color32::WHITE);
    }

    let [r, g, b] = color.effective_linear();
    let stops = clamp_stops(color.exposure_stops);
    response.on_hover_text(format!(
        "Linear RGB: [{r:.4}, {g:.4}, {b:.4}] ({stops:+.2} stops, ×{mult:.2}; 1.0 = SDR white)",
        mult = stops.exp2(),
    ))
}

/// Shows a button with the given HDR color (the true color, displayed via
/// the float-texture path).
///
/// If the user clicks the button, an HDR color picker is shown:
/// linear-light R/G/B sliders, an exposure slider, a live numeric readout
/// of the effective color, and side-by-side swatches (true color vs the SDR
/// clamp) so the difference is visible on SDR surfaces too.
///
/// Edits are linear RGB (NOT gamma-encoded) in `0..=1` and exposure stops
/// in −6..=+6, on [`HdrColor`].
pub fn color_edit_button_hdr(ui: &mut Ui, hdr: &mut HdrColor) -> Response {
    let popup_id = ui.auto_id_with("popup");
    let open = Popup::is_id_open(ui.ctx(), popup_id);
    let mut button_response = hdr_color_button(ui, hdr, open);
    if ui.style().explanation_tooltips {
        button_response = button_response.on_hover_text("Click to edit HDR color");
    }

    const COLOR_SLIDER_WIDTH: f32 = 275.0;

    Popup::menu(&button_response)
        .id(popup_id)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = COLOR_SLIDER_WIDTH;
            if color_picker_hdr(ui, hdr) {
                button_response.mark_changed();
            }
        });

    button_response
}

fn hdr_color_widget_info(ui: &Ui, color: &HdrColor, label: &'static str) -> WidgetInfo {
    let [r, g, b] = color.effective_linear();
    let mut info = WidgetInfo::labeled(Role::ColorWell, ui.is_enabled(), label);
    info.current_text_value = Some(format!("R {r:.3}, G {g:.3}, B {b:.3} (linear)"));
    info
}

fn clamp_stops(stops: f32) -> f32 {
    stops.clamp(HdrColor::MIN_EXPOSURE_STOPS, HdrColor::MAX_EXPOSURE_STOPS)
}

fn hdr_color_button(ui: &mut Ui, color: &HdrColor, open: bool) -> Response {
    let size = ui.spacing().interact_size;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| hdr_color_widget_info(ui, color, "HDR Color"));

    if ui.is_rect_visible(rect) {
        let visuals = if open {
            &ui.visuals().widgets.open
        } else {
            ui.style().interact(&response)
        };
        let rect = rect.expand(visuals.expansion);

        let stroke_width = 1.0;
        let corner_radius = visuals.corner_radius;
        // Shrink the fill inside the inside-stroke,
        // so the fill arcs stay concentric with it (like `color_button`):
        let (fill_rect, fill_corner_radius) = RoundedRect::new(rect, corner_radius)
            .shrink(stroke_width)
            .into_parts();
        // While the popup is open, mix the fill halfway toward the
        // backdrop: the premultiplied `from_white_alpha` tint blends
        // `½·texel + ½·backdrop` in the encoded domain, a simple
        // de-emphasis cue while the picker covers the color. On an
        // extended-range surface a superwhite fill stays brighter than the
        // SDR chrome even when dimmed — the mix is not a tone-map.
        let tint = if open {
            Color32::from_white_alpha(128)
        } else {
            Color32::WHITE
        };
        paint_hdr_swatch(ui, fill_rect, fill_corner_radius, color, tint);

        ui.painter().rect_stroke(
            rect,
            corner_radius,
            (stroke_width, visuals.bg_fill), // Using fill for stroke is intentional, because default style has no border
            StrokeKind::Inside,
        );
    }

    response
}

/// The HDR color picker: linear R/G/B sliders (NOT gamma-encoded), an
/// exposure slider in ¼-stop steps, the live numeric readout of the
/// effective color, and the true-vs-clamped swatch preview.
///
/// Returns `true` on change.
fn color_picker_hdr(ui: &mut Ui, hdr: &mut HdrColor) -> bool {
    let mut changed = false;

    // Snapshot for the gradient closures (each slider edits one channel;
    // the gradients are encoded + clamped u8, so they cannot exceed SDR
    // white — the *values* being edited stay linear):
    let [r, g, b] = hdr.rgb;

    let encode = |l: f32| gamma_u8_from_linear_f32(l);

    let red = color_slider_1d(ui, &mut hdr.rgb[0], |t| {
        Color32::from_rgb(encode(t), encode(g), encode(b))
    });
    red.widget_info(|| WidgetInfo::slider(ui.is_enabled(), f64::from(hdr.rgb[0]), "Red (linear)"));
    red.on_hover_text("Red (linear)");
    let green = color_slider_1d(ui, &mut hdr.rgb[1], |t| {
        Color32::from_rgb(encode(r), encode(t), encode(b))
    });
    green.widget_info(|| {
        WidgetInfo::slider(ui.is_enabled(), f64::from(hdr.rgb[1]), "Green (linear)")
    });
    green.on_hover_text("Green (linear)");
    let blue = color_slider_1d(ui, &mut hdr.rgb[2], |t| {
        Color32::from_rgb(encode(r), encode(g), encode(t))
    });
    blue.widget_info(|| {
        WidgetInfo::slider(ui.is_enabled(), f64::from(hdr.rgb[2]), "Blue (linear)")
    });
    blue.on_hover_text("Blue (linear)");

    if hdr.rgb != [r, g, b] {
        changed = true;
    }

    // Exposure in stops, −6..=+6 in ¼-stop steps, as an HDR ramp: the slider
    // body is a float texture (`FloatImage::exposure_ramp` — §6's carve-out
    // covers this axis-aligned rect), so on an extended-range surface it
    // brightens past SDR white where the display's headroom allows, instead of
    // reading flat white. The dashed line over it is the headroom boundary,
    // re-read from the viewport's `ViewportInfo` every frame; the slider keeps
    // its own ~10 fps repaints going while the picker is open (a brightness
    // change is no winit event, so without them the line would freeze once the
    // value is steady). The true-vs-clamped preview below stays: the ramp
    // shows the *axis*, the preview swatches show the *current value*.
    let exposure = hdr_exposure_slider(ui, hdr);
    if exposure
        .on_hover_text(format!(
            "Exposure: {:+.2} stops (the ramp shows true brightness on an extended-range surface; \
             the dashed line marks the display's headroom boundary)",
            clamp_stops(hdr.exposure_stops)
        ))
        .changed()
    {
        changed = true;
    }

    // Side-by-side preview: the true color (float-texture path) and the SDR
    // clamp. On an extended-range surface the two differ above SDR white;
    // on an SDR surface they coincide at the clamp — the clamp, made
    // visible.
    let half_slider = vec2(
        ui.spacing().slider_width * 0.5 - 4.0,
        ui.spacing().interact_size.y,
    );
    let corner_radius = ui.visuals().widgets.noninteractive.corner_radius;
    let (true_response, clamp_response) = ui
        .horizontal(|ui| {
            let (true_rect, true_response) = ui.allocate_exact_size(half_slider, Sense::hover());
            if ui.is_rect_visible(true_rect) {
                paint_hdr_swatch(ui, true_rect, corner_radius, hdr, Color32::WHITE);
            }
            let (clamp_rect, clamp_response) = ui.allocate_exact_size(half_slider, Sense::hover());
            ui.painter()
                .rect_filled(clamp_rect, corner_radius, hdr.clamped_color32());
            (true_response, clamp_response)
        })
        .inner;

    true_response.on_hover_text("True color (float-texture path)");
    clamp_response.on_hover_text("SDR clamp (what an SDR surface shows)");

    let [er, eg, eb] = hdr.effective_linear();
    let stops = clamp_stops(hdr.exposure_stops);
    ui.monospace(format!(
        "effective linear: R {er:.3}, G {eg:.3}, B {eb:.3}   ({stops:+.2} stops, ×{mult:.2})",
        mult = stops.exp2(),
    ));

    changed
}

/// Paint the true HDR color over the given rect via the cached 1×1 float
/// texture (the §6-compliant display path: a textured, axis-aligned rect).
/// `tint` multiplies the texture (plain white = no change).
fn paint_hdr_swatch(
    ui: &Ui,
    rect: Rect,
    corner_radius: impl Into<CornerRadius>,
    color: &HdrColor,
    tint: Color32,
) {
    let texture_id = hdr_swatch_texture_id(ui.ctx(), color);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter()
        .add(RectShape::filled(rect, corner_radius, tint).with_texture(texture_id, uv));
}

/// The cached float texture for an HDR swatch.
///
/// Vertex colors cap at 1.0, so the true (>1.0) color is displayed through
/// a 1×1 [`FloatImage`] stretched over the swatch (bilinear-sampled flat
/// color). Creating one texture per displayed color per frame would churn
/// GPU memory, so textures are cached in egui memory, keyed by the
/// effective linear color quantized to 8 bits per channel — quantized in
/// the *encoded* domain, where buckets are perceptually uniform
/// (≈0.4% steps, well below the ~1% Weber just-noticeable difference).
/// Quantizing the linear values uniformly instead would put near-black and
/// 25%-gray in one bucket, which is not visual equivalence. Two colors in
/// the same bucket share a swatch: the texture shows whichever was
/// uploaded first (the stored texel itself is never quantized).
///
/// Cache policy: at [`HDR_TEXTURE_CACHE_CAP`] entries the whole cache is
/// cleared — the simplest honest policy. The cache owns the
/// [`TextureHandle`]s (the last live reference), which is what keeps each
/// cached texture alive between frames; the handle is what egui frees a
/// texture through, so entries must not be reduced to bare [`TextureId`]s
/// (the id would dangle, and freed ids are reused). There is no individual
/// eviction: a texture lives until the cache-wide clear frees it.
fn hdr_swatch_texture_id(ctx: &Context, color: &HdrColor) -> TextureId {
    let linear = color.effective_linear();
    let key = hdr_swatch_key(linear);

    if let Some(id) = use_hdr_swatch_cache(ctx, |cache| cache.get(&key).map(TextureHandle::id)) {
        return id;
    }

    // The exposure is already applied; `from_linear` stays the single
    // encode path (linear in, contract-layout f16 out).
    let [r, g, b] = key;
    let texture = ctx.load_texture(
        format!("hdr_swatch_{r:02x}{g:02x}{b:02x}"),
        FloatImage::from_linear([1, 1], linear, 0.0),
        TextureOptions::LINEAR,
    );
    let id = texture.id();
    use_hdr_swatch_cache(ctx, |cache| {
        if cache.len() >= HDR_TEXTURE_CACHE_CAP {
            cache.clear(); // drops the handles → frees the textures
        }
        cache.insert(key, texture);
    });
    id
}

/// The 8-bit-per-channel cache key: the effective linear color, encoded and
/// normalized so that 255 = the brightest expressible swatch (1.0 linear at
/// +6 stops = 64× SDR white). The stored texel is never quantized — this is
/// only the bucket a swatch texture is reused for.
fn hdr_swatch_key(linear: [f32; 3]) -> [u8; 3] {
    // The brightest expressible swatch: 1.0 linear at +6 stops.
    // Derived from `MAX_EXPOSURE_STOPS` so the two cannot drift apart.
    let max_linear = HdrColor::MAX_EXPOSURE_STOPS.exp2();
    let max_encoded = gamma_from_linear(max_linear);
    linear.map(|l| {
        let encoded = gamma_from_linear(l.clamp(0.0, max_linear));
        (encoded / max_encoded * 255.0) as u8 // saturating float→u8 cast
    })
}

fn use_hdr_swatch_cache<R>(
    ctx: &Context,
    f: impl FnOnce(&mut HashMap<[u8; 3], TextureHandle>) -> R,
) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(Id::unique("hdr_swatch_texture_cache"))))
}

/// The cache key for an exposure ramp: the channel color bucket ([`hdr_ramp_bucket`],
/// the color the exposure axis sweeps) plus the stops range as a tag. The ramp
/// depends on rgb but NOT on the current exposure value, so one texture serves
/// every stops position — a whole drag touches neither the map nor the GPU.
///
/// A separate map from the flat-swatch cache (a ramp texture is 256× the
/// memory of a 1×1 swatch, and its reuse pattern is different), but the same
/// policy: [`HDR_TEXTURE_CACHE_CAP`] entries, then a wholesale clear; the map
/// owns the [`TextureHandle`]s, never bare [`TextureId`]s.
type ExposureRampKey = ([u8; 3], u32, u32);

/// The cached float texture for the exposure slider's ramp
/// ([`FloatImage::exposure_ramp`] at [`HDR_EXPOSURE_RAMP_WIDTH`]×1, stretched
/// across the slider rect).
fn exposure_ramp_texture_id(
    ctx: &Context,
    rgb: [f32; 3],
    stops_min: f32,
    stops_max: f32,
) -> TextureId {
    let bucket = hdr_ramp_bucket(rgb);
    let key = (bucket, stops_min.to_bits(), stops_max.to_bits());

    if let Some(id) = use_exposure_ramp_cache(ctx, |cache| cache.get(&key).map(TextureHandle::id)) {
        return id;
    }

    let [r, g, b] = bucket;
    let texture = ctx.load_texture(
        format!("hdr_exposure_ramp_{r:02x}{g:02x}{b:02x}"),
        FloatImage::exposure_ramp([HDR_EXPOSURE_RAMP_WIDTH, 1], rgb, stops_min, stops_max),
        TextureOptions::LINEAR,
    );
    let id = texture.id();
    use_exposure_ramp_cache(ctx, |cache| {
        if cache.len() >= HDR_TEXTURE_CACHE_CAP {
            cache.clear(); // drops the handles → frees the textures
        }
        cache.insert(key, texture);
    });
    id
}

/// The 8-bit-per-channel ramp cache key: the channel color (0..=1 linear — the
/// ramp never sees exposure-applied values), encoded and normalized over the
/// ramp's own domain, so 255 = SDR reference white. Same perceptual rationale
/// as [`hdr_swatch_key`] — quantize in the encoded domain, where buckets are
/// perceptually uniform (≈0.4% steps, below the ~1% Weber just-noticeable
/// difference; the swatch key's 64×-normalization would waste four fifths of
/// the key range on colors a ramp never holds). The stored texels are never
/// quantized — this is only the bucket a ramp texture is reused for.
fn hdr_ramp_bucket(rgb: [f32; 3]) -> [u8; 3] {
    // The top of the ramp's domain: 1.0 linear (SDR reference white).
    let max_encoded = gamma_from_linear(1.0);
    rgb.map(|l| {
        let encoded = gamma_from_linear(l.clamp(0.0, 1.0));
        (encoded / max_encoded * 255.0) as u8 // saturating float→u8 cast
    })
}

fn use_exposure_ramp_cache<R>(
    ctx: &Context,
    f: impl FnOnce(&mut HashMap<ExposureRampKey, TextureHandle>) -> R,
) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(Id::unique("hdr_exposure_ramp_texture_cache"))))
}

#[cfg(test)]
mod hdr_swatch_tests {
    use super::*;
    use crate::ecolor::linear_from_gamma;

    /// The key is 8 bits per channel in the encoded domain: 0 at black,
    /// 255 at the brightest expressible swatch, clamped outside, and
    /// monotonic per channel.
    #[test]
    fn hdr_swatch_key_is_encoded_eight_bit_per_channel() {
        // The key's top of range is tied to the widgets' exposure clamp:
        assert_eq!(
            hdr_swatch_key([HdrColor::MAX_EXPOSURE_STOPS.exp2(); 3]),
            [255, 255, 255]
        );
        assert_eq!(hdr_swatch_key([0.0, 0.0, 0.0]), [0, 0, 0]);
        assert_eq!(hdr_swatch_key([-1.0, -0.5, 0.0]), [0, 0, 0]); // clamped at black

        let max = hdr_swatch_key([64.0, 64.0, 64.0]); // 1.0 at +6 stops
        assert_eq!(max, [255, 255, 255]);
        assert_eq!(hdr_swatch_key([65.0, 100.0, 1e9]), max); // clamped above

        // 1.0 (SDR white) sits well below the top of the range:
        let white = hdr_swatch_key([1.0, 1.0, 1.0]);
        assert!(white[0] > 0 && white[0] < 255, "{white:?}");

        // Independent channels, monotonic within a channel:
        assert_eq!(hdr_swatch_key([1.0, 0.0, 0.0]), [white[0], 0, 0]);
        assert!(hdr_swatch_key([0.25, 0.0, 0.0])[0] < white[0]);
        assert!(hdr_swatch_key([0.5, 0.0, 0.0])[0] < hdr_swatch_key([1.0, 0.0, 0.0])[0]);
    }

    /// Same bucket → same texture; different color → different texture;
    /// repeated lookups don't re-upload.
    #[test]
    fn hdr_swatch_texture_reused_within_bucket() {
        let ctx = Context::default();

        let base = HdrColor::new([0.5, 0.5, 0.5], 0.0);
        // A neighbor within the same 8-bit encoded bucket (the bucket is
        // ~0.4% of the encoded range; this is ~0.2% of the linear value):
        let neighbor = HdrColor::new([0.501, 0.5, 0.5], 0.0);
        assert_eq!(
            hdr_swatch_key(base.effective_linear()),
            hdr_swatch_key(neighbor.effective_linear()),
            "test assumption: the two colors share a bucket"
        );

        let id_base = hdr_swatch_texture_id(&ctx, &base);
        let id_neighbor = hdr_swatch_texture_id(&ctx, &neighbor);
        assert_eq!(id_base, id_neighbor, "same bucket must share a swatch");

        // Repeated lookup: no new texture.
        assert_eq!(hdr_swatch_texture_id(&ctx, &base), id_base);

        // A clearly different color: a different texture.
        let other = HdrColor::new([0.1, 0.1, 0.1], 0.0);
        assert_ne!(
            hdr_swatch_key(other.effective_linear()),
            hdr_swatch_key(base.effective_linear())
        );
        assert_ne!(hdr_swatch_texture_id(&ctx, &other), id_base);
    }

    /// At the cap the whole cache is cleared (the simplest honest policy):
    /// a color shown before the clear re-uploads as a new texture, and the
    /// map stays bounded.
    #[test]
    fn hdr_swatch_texture_cache_clears_at_cap() {
        let ctx = Context::default();

        // Distinct buckets, guaranteed: invert the key in the encoded
        // domain, two channels (a single channel only spans 256 buckets —
        // exactly the cap). Even steps so ±1 rounding can't merge colors.
        let lin = |k: u8| linear_from_gamma(k as f32 / 255.0 * gamma_from_linear(64.0));
        let color_for_key = |k0: u8, k1: u8| HdrColor::new([lin(k0), lin(k1), 0.0], 0.0);

        let first = color_for_key(0, 0);
        let first_id = hdr_swatch_texture_id(&ctx, &first);

        // More distinct buckets than the cap:
        let mut ids = Vec::new();
        for k1 in 0..4u8 {
            for c0 in 1..=127u8 {
                ids.push(hdr_swatch_texture_id(&ctx, &color_for_key(2 * c0, k1)));
            }
        }
        assert!(ids.len() > HDR_TEXTURE_CACHE_CAP);
        assert!(ids.iter().all(|id| *id != first_id));

        use_hdr_swatch_cache(&ctx, |cache| {
            assert!(
                cache.len() <= HDR_TEXTURE_CACHE_CAP,
                "cache stays capped, got {}",
                cache.len()
            );
        });

        // After the clear, the first color is a cache miss and re-uploads
        // (texture ids are never reused, so a re-upload is observable):
        let re_id = hdr_swatch_texture_id(&ctx, &first);
        assert_ne!(
            re_id, first_id,
            "cleared entry must re-upload, not serve a stale id"
        );
        // …and it is cached again:
        assert_eq!(hdr_swatch_texture_id(&ctx, &first), re_id);
    }
}

#[cfg(test)]
mod hdr_exposure_tests {
    use super::*;

    const MIN: f32 = HdrColor::MIN_EXPOSURE_STOPS; // −6
    const MAX: f32 = HdrColor::MAX_EXPOSURE_STOPS; // +6

    /// The line math: `None` headroom means no line; SDR white (×1.0) is the
    /// center of the −6..=+6 ramp; the boundaries clamp left and vanish at
    /// (and above) the right end.
    #[test]
    fn headroom_line_fraction_rules() {
        // Unknown headroom: no line, never a fabricated position.
        assert_eq!(headroom_line_fraction(None, MIN, MAX), None);

        // Some(1.0) = SDR white = 0 stops = the exact center:
        assert!((headroom_line_fraction(Some(1.0), MIN, MAX).unwrap() - 0.5).abs() < 1e-6);

        // Below the shown range: clamped to the left edge (everything right
        // of it clips — real information). 0.5 headroom is −1 stop, *inside*
        // the range; use values below 2^−6:
        assert_eq!(
            headroom_line_fraction(Some(MIN.exp2()), MIN, MAX),
            Some(0.0)
        );
        assert_eq!(headroom_line_fraction(Some(0.01), MIN, MAX), Some(0.0));
        assert_eq!(
            headroom_line_fraction(Some(f32::EPSILON), MIN, MAX),
            Some(0.0)
        );

        // Non-positive or NaN headroom is a broken report, not an unknown:
        // total over f32 — clamped to the left edge, never NaN:
        assert_eq!(headroom_line_fraction(Some(0.0), MIN, MAX), Some(0.0));
        assert_eq!(headroom_line_fraction(Some(-1.0), MIN, MAX), Some(0.0));
        assert_eq!(
            headroom_line_fraction(Some(f32::NEG_INFINITY), MIN, MAX),
            Some(0.0)
        );
        assert_eq!(headroom_line_fraction(Some(f32::NAN), MIN, MAX), Some(0.0));

        // At the top of the shown range (and above): hidden — nothing on the
        // ramp clips.
        assert_eq!(headroom_line_fraction(Some(MAX.exp2()), MIN, MAX), None);
        assert_eq!(
            headroom_line_fraction(Some(MAX.exp2() * 8.0), MIN, MAX),
            None
        );

        // Just below the top of the range: pinned to the right edge (the line
        // slides off the end as the headroom rises):
        let just_below = headroom_line_fraction(Some(MAX.exp2() * 0.999), MIN, MAX).unwrap();
        assert!((just_below - 1.0).abs() < 0.01, "{just_below}");
    }

    /// Monotonicity: a larger headroom never moves the line left, and the
    /// fraction stays in 0..=1 for every in-range headroom.
    #[test]
    fn headroom_line_fraction_is_monotone_and_bounded() {
        let mut prev = 0.0_f32;
        let mut headroom = MIN.exp2();
        while headroom < MAX.exp2() {
            let fraction = headroom_line_fraction(Some(headroom), MIN, MAX).unwrap();
            assert!((0.0..=1.0).contains(&fraction), "h {headroom}: {fraction}");
            assert!(fraction >= prev, "h {headroom}: {fraction} < {prev}");
            prev = fraction;
            headroom *= 1.25;
        }
    }

    /// The ramp bucket: full 0..=255 span for the ramp's 0..=1 domain, clamped
    /// outside, monotone — the ramp's top of range is SDR white, not the
    /// swatch's 64× (which would waste the key range above 1.0).
    #[test]
    fn hdr_ramp_bucket_spans_the_ramp_domain() {
        assert_eq!(hdr_ramp_bucket([0.0, 0.0, 0.0]), [0, 0, 0]);
        assert_eq!(hdr_ramp_bucket([-1.0, -0.5, 0.0]), [0, 0, 0]); // clamped at black
        assert_eq!(hdr_ramp_bucket([1.0, 1.0, 1.0]), [255, 255, 255]);
        assert_eq!(hdr_ramp_bucket([2.0, 100.0, 1e9]), [255, 255, 255]); // clamped above

        // Monotone per channel, independent channels:
        let white = hdr_ramp_bucket([1.0, 0.0, 0.0]);
        assert_eq!(white, [255, 0, 0]);
        assert!(hdr_ramp_bucket([0.5, 0.0, 0.0])[0] < 255);
        assert!(hdr_ramp_bucket([0.25, 0.0, 0.0])[0] < hdr_ramp_bucket([0.5, 0.0, 0.0])[0]);
    }

    /// The ramp texture depends on rgb but NOT on the current stops value:
    /// one texture per color bucket serves a whole drag (no churn), a
    /// different color is a different texture, and repeated lookups don't
    /// re-upload.
    #[test]
    fn exposure_ramp_texture_is_stable_across_a_drag() {
        let ctx = Context::default();
        let rgb = [0.5, 0.25, 0.0625];

        let at_min = exposure_ramp_texture_id(&ctx, rgb, MIN, MAX);
        // The current value moves from −4.8 to +4.75 mid-drag; the ramp must not:
        for stops in [-4.75_f32, 0.0, 2.5, 4.75] {
            assert_eq!(
                exposure_ramp_texture_id(&ctx, rgb, MIN, MAX),
                at_min,
                "stops {stops}: the ramp must not churn during a drag"
            );
        }

        // Repeated lookup: no new texture.
        assert_eq!(exposure_ramp_texture_id(&ctx, rgb, MIN, MAX), at_min);

        // A clearly different color: a different texture.
        let other = [0.1, 0.1, 0.1];
        assert_ne!(
            hdr_ramp_bucket(other),
            hdr_ramp_bucket(rgb),
            "test assumption: the two colors sit in different buckets"
        );
        assert_ne!(exposure_ramp_texture_id(&ctx, other, MIN, MAX), at_min);

        // The stops range is part of the key (a different axis is a different ramp):
        assert_ne!(exposure_ramp_texture_id(&ctx, rgb, MIN, 3.0), at_min);
    }

    /// Same shared policy as the swatch cache: at [`HDR_TEXTURE_CACHE_CAP`]
    /// entries the whole cache is cleared, and a cleared entry re-uploads
    /// (texture ids are never reused).
    #[test]
    fn exposure_ramp_cache_clears_at_cap() {
        let ctx = Context::default();
        let rgb = [0.5, 0.5, 0.5];

        let first = exposure_ramp_texture_id(&ctx, rgb, MIN, MAX);

        // Distinct keys via distinct stops ranges (same rgb bucket); i starts
        // at 1 — i = 0 would be the first key itself:
        let mut ids = Vec::new();
        for i in 1..(HDR_TEXTURE_CACHE_CAP + 32) {
            let max = MAX + i as f32; // distinct f32 bit patterns, valid ranges
            ids.push(exposure_ramp_texture_id(&ctx, rgb, MIN, max));
        }
        assert!(ids.len() > HDR_TEXTURE_CACHE_CAP);
        assert!(ids.iter().all(|id| *id != first));

        use_exposure_ramp_cache(&ctx, |cache| {
            assert!(
                cache.len() <= HDR_TEXTURE_CACHE_CAP,
                "cache stays capped, got {}",
                cache.len()
            );
        });

        // After the clear, the first key is a cache miss and re-uploads:
        let re_id = exposure_ramp_texture_id(&ctx, rgb, MIN, MAX);
        assert_ne!(
            re_id, first,
            "cleared entry must re-upload, not serve a stale id"
        );
        assert_eq!(exposure_ramp_texture_id(&ctx, rgb, MIN, MAX), re_id);
    }
}
