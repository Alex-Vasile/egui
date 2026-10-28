//! Color picker widgets.

use crate::util::fixed_cache::FixedCache;
use crate::{
    Context, DragValue, Id, Painter, Popup, PopupCloseBehavior, Response, Role, Sense,
    TextureHandle, TextureId, Ui, Widget as _, WidgetInfo, epaint, lerp, remap_clamp,
};
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

/// How many cached swatch textures to keep before clearing the whole cache
/// (dropping the handles and with them the textures). There is no
/// individual eviction; the cap bounds the cache.
const HDR_SWATCH_CACHE_CAP: usize = 256;

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

    // Exposure in stops, −6..=+6 in ¼-stop steps. The gradient sweeps the
    // exposure over the current color, encoded + clamped: the ramp is u8,
    // so past SDR white it reads flat white even on an extended-range
    // surface (§6 reserves float display for the flat swatches) — the true
    // result is the preview swatch below, not the ramp.
    let min = HdrColor::MIN_EXPOSURE_STOPS;
    let max = HdrColor::MAX_EXPOSURE_STOPS;
    let mut exposure_norm = remap_clamp(hdr.exposure_stops, min..=max, 0.0..=1.0);
    let exposure = color_slider_1d(ui, &mut exposure_norm, |t| {
        let factor = lerp(min..=max, t).exp2();
        Color32::from_rgb(encode(r * factor), encode(g * factor), encode(b * factor))
    });
    exposure.widget_info(|| {
        // The value the screen reader announces is the edited quantity
        // (stops), not the slider's internal 0..1 normalization.
        WidgetInfo::slider(
            ui.is_enabled(),
            f64::from(lerp(min..=max, exposure_norm)),
            "Exposure (¼-stop)",
        )
    });
    if exposure
        .on_hover_text(format!(
            "Exposure: {:+.2} stops (the ramp itself clamps at SDR white — the preview swatch \
             below shows the true brightness)",
            clamp_stops(hdr.exposure_stops)
        ))
        .changed()
    {
        let stops = lerp(min..=max, exposure_norm);
        hdr.exposure_stops = (stops / 0.25).round() * 0.25; // ¼-stop steps
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
/// Cache policy: at [`HDR_SWATCH_CACHE_CAP`] entries the whole cache is
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
        if cache.len() >= HDR_SWATCH_CACHE_CAP {
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
        assert!(ids.len() > HDR_SWATCH_CACHE_CAP);
        assert!(ids.iter().all(|id| *id != first_id));

        use_hdr_swatch_cache(&ctx, |cache| {
            assert!(
                cache.len() <= HDR_SWATCH_CACHE_CAP,
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
