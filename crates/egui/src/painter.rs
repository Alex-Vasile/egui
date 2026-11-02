use alloc::sync::Arc;

use emath::GuiRounding as _;
use epaint::{
    CircleShape, ClippedShape, CornerRadius, HdrColorMode, HdrVertexColor, PathStroke, RectShape,
    Shape, Stroke, StrokeKind,
    text::{FontsView, Galley, LayoutJob},
};

use crate::{
    Color32, Context, FontId,
    emath::{Align2, Pos2, Rangef, Rect, Vec2},
    layers::{LayerId, PaintList, ShapeIdx},
};

/// Helper to paint shapes and text to a specific region on a specific layer.
///
/// All coordinates are screen coordinates in the unit points (one point can consist of many physical pixels).
///
/// A [`Painter`] never outlive a single frame/pass.
#[derive(Clone)]
pub struct Painter {
    /// Source of fonts and destination of shapes
    ctx: Context,

    /// For quick access, without having to go via [`Context`].
    pixels_per_point: f32,

    /// Where we paint
    layer_id: LayerId,

    /// Everything painted in this [`Painter`] will be clipped against this.
    /// This means nothing outside of this rectangle will be visible on screen.
    clip_rect: Rect,

    /// If set, all shapes will have their colors modified to be closer to this.
    /// This is used to implement grayed out interfaces.
    fade_to_color: Option<Color32>,

    /// If set, all shapes will have their colors modified with [`Color32::gamma_multiply`] with
    /// this value as the factor.
    /// This is used to make interfaces semi-transparent.
    opacity_factor: f32,
}

impl Painter {
    /// Create a painter to a specific layer within a certain clip rectangle.
    pub fn new(ctx: Context, layer_id: LayerId, clip_rect: Rect) -> Self {
        let pixels_per_point = ctx.pixels_per_point();
        Self {
            ctx,
            pixels_per_point,
            layer_id,
            clip_rect,
            fade_to_color: None,
            opacity_factor: 1.0,
        }
    }

    /// Redirect where you are painting.
    #[must_use]
    #[inline]
    pub fn with_layer_id(mut self, layer_id: LayerId) -> Self {
        self.layer_id = layer_id;
        self
    }

    /// Create a painter for a sub-region of this [`Painter`].
    ///
    /// The clip-rect of the returned [`Painter`] will be the intersection
    /// of the given rectangle and the `clip_rect()` of the parent [`Painter`].
    pub fn with_clip_rect(&self, rect: Rect) -> Self {
        let mut new_self = self.clone();
        new_self.clip_rect = rect.intersect(self.clip_rect);
        new_self
    }

    /// Redirect where you are painting.
    ///
    /// It is undefined behavior to change the [`LayerId`]
    /// of [`crate::Ui::painter`].
    pub fn set_layer_id(&mut self, layer_id: LayerId) {
        self.layer_id = layer_id;
    }

    /// Set the opacity (alpha multiplier) of everything painted by this painter from this point forward.
    ///
    /// `opacity` must be between 0.0 and 1.0, where 0.0 means fully transparent (i.e., invisible)
    /// and 1.0 means fully opaque.
    ///
    /// See also: [`Self::opacity`] and [`Self::multiply_opacity`].
    pub fn set_opacity(&mut self, opacity: f32) {
        if opacity.is_finite() {
            self.opacity_factor = opacity.clamp(0.0, 1.0);
        }
    }

    /// Like [`Self::set_opacity`], but multiplies the given value with the current opacity.
    ///
    /// See also: [`Self::set_opacity`] and [`Self::opacity`].
    pub fn multiply_opacity(&mut self, opacity: f32) {
        if opacity.is_finite() {
            self.opacity_factor *= opacity.clamp(0.0, 1.0);
        }
    }

    /// Read the current opacity of the underlying painter.
    ///
    /// See also: [`Self::set_opacity`] and [`Self::multiply_opacity`].
    #[inline]
    pub fn opacity(&self) -> f32 {
        self.opacity_factor
    }

    /// If `false`, nothing you paint will show up.
    ///
    /// Also checks [`Context::will_discard`].
    pub fn is_visible(&self) -> bool {
        self.fade_to_color != Some(Color32::TRANSPARENT) && !self.ctx.will_discard()
    }

    /// If `false`, nothing added to the painter will be visible
    pub fn set_invisible(&mut self) {
        self.fade_to_color = Some(Color32::TRANSPARENT);
    }

    /// Get a reference to the parent [`Context`].
    #[inline]
    pub fn ctx(&self) -> &Context {
        &self.ctx
    }

    /// Number of physical pixels for each logical UI point.
    #[inline]
    pub fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    /// Read-only access to the shared [`FontsView`].
    ///
    /// See [`Context`] documentation for how locks work.
    #[inline]
    pub fn fonts<R>(&self, reader: impl FnOnce(&FontsView<'_>) -> R) -> R {
        self.ctx.fonts(reader)
    }

    /// Read-write access to the shared [`FontsView`].
    ///
    /// See [`Context`] documentation for how locks work.
    #[inline]
    pub fn fonts_mut<R>(&self, reader: impl FnOnce(&mut FontsView<'_>) -> R) -> R {
        self.ctx.fonts_mut(reader)
    }

    /// Where we paint
    #[inline]
    pub fn layer_id(&self) -> LayerId {
        self.layer_id
    }

    /// Everything painted in this [`Painter`] will be clipped against this.
    /// This means nothing outside of this rectangle will be visible on screen.
    #[inline]
    pub fn clip_rect(&self) -> Rect {
        self.clip_rect
    }

    /// Constrain the rectangle in which we can paint.
    ///
    /// Short for `painter.set_clip_rect(painter.clip_rect().intersect(new_clip_rect))`.
    ///
    /// See also: [`Self::clip_rect`] and [`Self::set_clip_rect`].
    #[inline]
    pub fn shrink_clip_rect(&mut self, new_clip_rect: Rect) {
        self.clip_rect = self.clip_rect.intersect(new_clip_rect);
    }

    /// Everything painted in this [`Painter`] will be clipped against this.
    /// This means nothing outside of this rectangle will be visible on screen.
    ///
    /// Warning: growing the clip rect might cause unexpected results!
    /// When in doubt, use [`Self::shrink_clip_rect`] instead.
    #[inline]
    pub fn set_clip_rect(&mut self, clip_rect: Rect) {
        self.clip_rect = clip_rect;
    }

    /// Useful for pixel-perfect rendering of lines that are one pixel wide (or any odd number of pixels).
    #[inline]
    pub fn round_to_pixel_center(&self, point: f32) -> f32 {
        point.round_to_pixel_center(self.pixels_per_point())
    }
}

/// ## Low level
impl Painter {
    #[inline]
    fn paint_list<R>(&self, writer: impl FnOnce(&mut PaintList) -> R) -> R {
        self.ctx.graphics_mut(|g| writer(g.entry(self.layer_id)))
    }

    fn transform_shape(&self, shape: &mut Shape) {
        if let Some(fade_to_color) = self.fade_to_color {
            tint_shape_towards(shape, fade_to_color);
        }
        if self.opacity_factor < 1.0 {
            multiply_opacity(shape, self.opacity_factor);
        }
    }

    /// It is up to the caller to make sure there is room for this.
    /// Can be used for free painting.
    /// NOTE: all coordinates are screen coordinates!
    pub fn add(&self, shape: impl Into<Shape>) -> ShapeIdx {
        if self.fade_to_color == Some(Color32::TRANSPARENT) || self.opacity_factor == 0.0 {
            self.paint_list(|l| l.add(self.clip_rect, Shape::Noop))
        } else {
            let mut shape = shape.into();
            self.transform_shape(&mut shape);
            self.paint_list(|l| l.add(self.clip_rect, shape))
        }
    }

    /// Add many shapes at once.
    ///
    /// Calling this once is generally faster than calling [`Self::add`] multiple times.
    pub fn extend<I: IntoIterator<Item = Shape>>(&self, shapes: I) {
        if self.fade_to_color == Some(Color32::TRANSPARENT) || self.opacity_factor == 0.0 {
            return;
        }
        if self.fade_to_color.is_some() || self.opacity_factor < 1.0 {
            let shapes = shapes.into_iter().map(|mut shape| {
                self.transform_shape(&mut shape);
                shape
            });
            self.paint_list(|l| l.extend(self.clip_rect, shapes));
        } else {
            self.paint_list(|l| l.extend(self.clip_rect, shapes));
        }
    }

    /// Modify an existing [`Shape`].
    pub fn set(&self, idx: ShapeIdx, shape: impl Into<Shape>) {
        if self.fade_to_color == Some(Color32::TRANSPARENT) {
            return;
        }
        let mut shape = shape.into();
        self.transform_shape(&mut shape);
        self.paint_list(|l| l.set(idx, self.clip_rect, shape));
    }

    /// Add a shape painted in an HDR color, using its coverage (alpha) as the
    /// mask.
    ///
    /// The generic escape hatch for painting anything above SDR white:
    /// strokes, polylines, circles, images, polygon fills — construct the shape
    /// exactly as the SDR API would (with any colour; only its alpha survives)
    /// and let the wrapper recolour it. The alpha channel is the coverage
    /// mask, so feathering, rounded corners and antialiasing keep working with
    /// no new geometry code.
    ///
    /// Limitations, stated rather than implied: the recolour is **uniform** (a
    /// shape with a differently-coloured fill and stroke collapses to one
    /// colour), and the inner shape's own positional colours are discarded (a
    /// [`ColorMode::UV`](epaint::ColorMode::UV) ramp is lost). Use
    /// [`Self::add_hdr_uv`], [`Shape::gradient_rect_hdr`], or a hand-built
    /// mesh with [`epaint::Mesh::hdr_colors`] for per-vertex variation.
    ///
    /// On a backend without HDR support (today: `egui_glow`) the shape still
    /// draws, range-reduced into the legal SDR range.
    pub fn add_hdr(&self, shape: impl Into<Shape>, color: impl Into<HdrColorMode>) -> ShapeIdx {
        self.add(Shape::hdr(shape, color))
    }

    /// Add a shape painted in an HDR color that varies with position.
    ///
    /// The callback receives the shape's visual bounding box (expanded by the
    /// tessellator's feathering) and a position, and returns the HDR value for
    /// it. This is the per-position ramp for wrapped geometry; it is called
    /// once per emitted vertex, so keep it cheap.
    pub fn add_hdr_uv(
        &self,
        shape: impl Into<Shape>,
        callback: impl Fn(Rect, Pos2) -> HdrVertexColor + Send + Sync + 'static,
    ) -> ShapeIdx {
        self.add(Shape::hdr(shape, HdrColorMode::UV(Arc::new(callback))))
    }

    /// Access all shapes added this frame.
    pub fn for_each_shape(&self, mut reader: impl FnMut(&ClippedShape)) {
        self.ctx.graphics(|g| {
            if let Some(list) = g.get(self.layer_id) {
                for c in list.all_entries() {
                    reader(c);
                }
            }
        });
    }
}

/// ## Debug painting
impl Painter {
    #[expect(clippy::needless_pass_by_value)]
    pub fn debug_rect(&self, rect: Rect, color: Color32, text: impl ToString) {
        self.rect(
            rect,
            0.0,
            color.additive().linear_multiply(0.015),
            (1.0, color),
            StrokeKind::Outside,
        );
        self.text(
            rect.min,
            Align2::LEFT_TOP,
            text.to_string(),
            FontId::monospace(12.0),
            color,
        );
    }

    pub fn error(&self, pos: Pos2, text: impl core::fmt::Display) -> Rect {
        let color = self.ctx.global_style().visuals.error_fg_color;
        self.debug_text(pos, Align2::LEFT_TOP, color, format!("🔥 {text}"))
    }

    /// Text with a background.
    ///
    /// See also [`Context::debug_text`].
    #[expect(clippy::needless_pass_by_value)]
    pub fn debug_text(
        &self,
        pos: Pos2,
        anchor: Align2,
        color: Color32,
        text: impl ToString,
    ) -> Rect {
        let galley = self.layout_no_wrap(text.to_string(), FontId::monospace(12.0), color);
        let rect = anchor.anchor_size(pos, galley.size());
        let frame_rect = rect.expand(2.0);

        let is_text_bright = color.is_additive() || epaint::Rgba::from(color).intensity() > 0.5;
        let bg_color = if is_text_bright {
            Color32::from_black_alpha(150)
        } else {
            Color32::from_white_alpha(150)
        };
        self.add(Shape::rect_filled(frame_rect, 0.0, bg_color));
        self.galley(rect.min, galley, color);
        frame_rect
    }
}

/// # Paint different primitives
impl Painter {
    /// Paints a line from the first point to the second.
    pub fn line_segment(&self, points: [Pos2; 2], stroke: impl Into<Stroke>) -> ShapeIdx {
        self.add(Shape::LineSegment {
            points,
            stroke: stroke.into(),
        })
    }

    /// Paints a line from the first point to the second, in an HDR color.
    ///
    /// [`Stroke`] is 8 bytes and cannot grow, so HDR segments go through the
    /// [`Shape::Hdr`] wrapper (the stroke's own colour supplies coverage only).
    pub fn line_segment_hdr(
        &self,
        points: [Pos2; 2],
        stroke: impl Into<Stroke>,
        color: impl Into<HdrVertexColor>,
    ) -> ShapeIdx {
        self.add_hdr(Shape::line_segment(points, stroke), color.into())
    }

    /// Paints a line connecting the points.
    /// NOTE: all coordinates are screen coordinates!
    pub fn line(&self, points: Vec<Pos2>, stroke: impl Into<PathStroke>) -> ShapeIdx {
        self.add(Shape::line(points, stroke))
    }

    /// Paints a horizontal line.
    pub fn hline(&self, x: impl Into<Rangef>, y: f32, stroke: impl Into<Stroke>) -> ShapeIdx {
        self.add(Shape::hline(x, y, stroke))
    }

    /// Paints a vertical line.
    pub fn vline(&self, x: f32, y: impl Into<Rangef>, stroke: impl Into<Stroke>) -> ShapeIdx {
        self.add(Shape::vline(x, y, stroke))
    }

    pub fn circle(
        &self,
        center: Pos2,
        radius: f32,
        fill_color: impl Into<Color32>,
        stroke: impl Into<Stroke>,
    ) -> ShapeIdx {
        self.add(CircleShape {
            center,
            radius,
            fill: fill_color.into(),
            stroke: stroke.into(),
        })
    }

    pub fn circle_filled(
        &self,
        center: Pos2,
        radius: f32,
        fill_color: impl Into<Color32>,
    ) -> ShapeIdx {
        self.add(CircleShape {
            center,
            radius,
            fill: fill_color.into(),
            stroke: Default::default(),
        })
    }

    pub fn circle_stroke(&self, center: Pos2, radius: f32, stroke: impl Into<Stroke>) -> ShapeIdx {
        self.add(CircleShape {
            center,
            radius,
            fill: Default::default(),
            stroke: stroke.into(),
        })
    }

    /// See also [`Self::rect_filled`] and [`Self::rect_stroke`].
    pub fn rect(
        &self,
        rect: Rect,
        corner_radius: impl Into<CornerRadius>,
        fill_color: impl Into<Color32>,
        stroke: impl Into<Stroke>,
        stroke_kind: StrokeKind,
    ) -> ShapeIdx {
        self.add(RectShape::new(
            rect,
            corner_radius,
            fill_color,
            stroke,
            stroke_kind,
        ))
    }

    pub fn rect_filled(
        &self,
        rect: Rect,
        corner_radius: impl Into<CornerRadius>,
        fill_color: impl Into<Color32>,
    ) -> ShapeIdx {
        self.add(RectShape::filled(rect, corner_radius, fill_color))
    }

    /// Paints a filled rectangle in an HDR color.
    ///
    /// The fill colour supplies coverage only: interior vertices are opaque and
    /// take the HDR value, feather vertices are transparent and stay
    /// transparent. This inherits rounded corners, stroke kinds, blur, brushes
    /// and every future [`RectShape`] feature instead of duplicating corner
    /// tessellation.
    pub fn rect_filled_hdr(
        &self,
        rect: Rect,
        corner_radius: impl Into<CornerRadius>,
        color: impl Into<HdrVertexColor>,
    ) -> ShapeIdx {
        self.add_hdr(
            RectShape::filled(rect, corner_radius, Color32::WHITE),
            color.into(),
        )
    }

    pub fn rect_stroke(
        &self,
        rect: Rect,
        corner_radius: impl Into<CornerRadius>,
        stroke: impl Into<Stroke>,
        stroke_kind: StrokeKind,
    ) -> ShapeIdx {
        self.add(RectShape::stroke(rect, corner_radius, stroke, stroke_kind))
    }

    /// Show an arrow starting at `origin` and going in the direction of `vec`, with the length `vec.length()`.
    pub fn arrow(&self, origin: Pos2, vec: Vec2, stroke: impl Into<Stroke>) {
        use crate::emath::Rot2;
        let rot = Rot2::from_angle(core::f32::consts::TAU / 10.0);
        let tip_length = vec.length() * 0.25;
        let tip = origin + vec;
        let dir = vec.normalized();
        let stroke = stroke.into();
        self.line_segment([origin, tip], stroke);
        self.line_segment([tip, tip - tip_length * (rot * dir)], stroke);
        self.line_segment([tip, tip - tip_length * (rot.inverse() * dir)], stroke);
    }

    /// Show an arrow in an HDR color.
    ///
    /// [`Self::arrow`] is a procedure (three `line_segment` calls), not a
    /// builder, so it cannot be wrapped after the fact — hence its own twin.
    /// The three segments are built with the same geometry as [`Self::arrow`]
    /// and flattened into the HDR wrapper together.
    pub fn arrow_hdr(
        &self,
        origin: Pos2,
        vec: Vec2,
        stroke: impl Into<Stroke>,
        color: impl Into<HdrVertexColor>,
    ) {
        use crate::emath::Rot2;
        let rot = Rot2::from_angle(core::f32::consts::TAU / 10.0);
        let tip_length = vec.length() * 0.25;
        let tip = origin + vec;
        let dir = vec.normalized();
        let stroke = stroke.into();
        let segments = Shape::Vec(vec![
            Shape::line_segment([origin, tip], stroke),
            Shape::line_segment([tip, tip - tip_length * (rot * dir)], stroke),
            Shape::line_segment([tip, tip - tip_length * (rot.inverse() * dir)], stroke),
        ]);
        self.add_hdr(segments, color.into());
    }

    /// An image at the given position.
    ///
    /// `uv` should normally be `Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))`
    /// unless you want to crop or flip the image.
    ///
    /// `tint` is a color multiplier. Use [`Color32::WHITE`] if you don't want to tint the image.
    ///
    /// Usually it is easier to use [`crate::Image::paint_at`] instead:
    ///
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// # let rect = egui::Rect::from_min_size(Default::default(), egui::Vec2::splat(100.0));
    /// egui::Image::new(egui::include_image!("../assets/ferris.png"))
    ///     .corner_radius(5)
    ///     .tint(egui::Color32::LIGHT_BLUE)
    ///     .paint_at(ui, rect);
    /// # });
    /// ```
    pub fn image(
        &self,
        texture_id: epaint::TextureId,
        rect: Rect,
        uv: Rect,
        tint: Color32,
    ) -> ShapeIdx {
        self.add(Shape::image(texture_id, rect, uv, tint))
    }
}

/// ## Text
impl Painter {
    /// Lay out and paint some text.
    ///
    /// To center the text at the given position, use `Align2::CENTER_CENTER`.
    ///
    /// To find out the size of text before painting it, use
    /// [`Self::layout`] or [`Self::layout_no_wrap`].
    ///
    /// Returns where the text ended up.
    #[expect(clippy::needless_pass_by_value)]
    pub fn text(
        &self,
        pos: Pos2,
        anchor: Align2,
        text: impl ToString,
        font_id: FontId,
        text_color: Color32,
    ) -> Rect {
        let galley = self.layout_no_wrap(text.to_string(), font_id, text_color);
        let rect = anchor.anchor_size(pos, galley.size());
        self.galley(rect.min, galley, text_color);
        rect
    }

    /// Will wrap text at the given width and line break at `\n`.
    ///
    /// Paint the results with [`Self::galley`].
    #[inline]
    #[must_use]
    pub fn layout(
        &self,
        text: String,
        font_id: FontId,
        color: crate::Color32,
        wrap_width: f32,
    ) -> Arc<Galley> {
        self.fonts_mut(|f| f.layout(text, font_id, color, wrap_width))
    }

    /// Will line break at `\n`.
    ///
    /// Paint the results with [`Self::galley`].
    #[inline]
    #[must_use]
    pub fn layout_no_wrap(
        &self,
        text: String,
        font_id: FontId,
        color: crate::Color32,
    ) -> Arc<Galley> {
        self.fonts_mut(|f| f.layout(text, font_id, color, f32::INFINITY))
    }

    /// Lay out this text layut job in a galley.
    ///
    /// Paint the results with [`Self::galley`].
    #[inline]
    #[must_use]
    pub fn layout_job(&self, layout_job: LayoutJob) -> Arc<Galley> {
        self.fonts_mut(|f| f.layout_job(layout_job))
    }

    /// Paint text that has already been laid out in a [`Galley`].
    ///
    /// You can create the [`Galley`] with [`Self::layout`] or [`Self::layout_job`].
    ///
    /// Any uncolored parts of the [`Galley`] (using [`Color32::PLACEHOLDER`]) will be replaced with the given color.
    ///
    /// Any non-placeholder color in the galley takes precedence over this fallback color.
    #[inline]
    pub fn galley(&self, pos: Pos2, galley: Arc<Galley>, fallback_color: Color32) {
        if !galley.is_empty() {
            self.add(Shape::galley(pos, galley, fallback_color));
        }
    }

    /// Paint text that has already been laid out in a [`Galley`].
    ///
    /// You can create the [`Galley`] with [`Self::layout`].
    ///
    /// All text color in the [`Galley`] will be replaced with the given color.
    #[inline]
    pub fn galley_with_override_text_color(
        &self,
        pos: Pos2,
        galley: Arc<Galley>,
        text_color: Color32,
    ) {
        if !galley.is_empty() {
            self.add(Shape::galley_with_override_text_color(
                pos, galley, text_color,
            ));
        }
    }
}

fn tint_shape_towards(shape: &mut Shape, target: Color32) {
    epaint::shape_transform::adjust_colors(
        shape,
        move |color| {
            if *color != Color32::PLACEHOLDER {
                *color = crate::ecolor::tint_color_towards(*color, target);
            }
        },
        move |hdr| tint_hdr_towards(hdr, target),
    );
}

fn multiply_opacity(shape: &mut Shape, opacity: f32) {
    epaint::shape_transform::adjust_colors(
        shape,
        move |color| {
            if *color != Color32::PLACEHOLDER {
                *color = color.gamma_multiply(opacity);
            }
        },
        move |hdr| {
            *hdr = hdr.scaled(opacity);
        },
    );
}

/// The encoded-space mirror of [`ecolor::tint_color_towards`] for a stored HDR
/// value.
///
/// It reproduces the three branches of the `Color32` function exactly, on the
/// sidecar's `0..=1` scale: the `a8 = round(a × 255)` threshold, the middle
/// branch's *integer* `2·255/a8` divisor, and the `a == 0` branch that halves
/// rgb without touching alpha. Reproducing them is what keeps an opaque HDR
/// mesh and its `Color32` twin from diverging on a fade.
///
/// Above 1.0 there is no twin to agree with, and this mirror *is* the
/// definition; below it, a twin-representable value lands on the same 8-bit
/// result up to rounding.
fn tint_hdr_towards(color: &mut HdrVertexColor, target: Color32) {
    let [r, g, b, a] = color.to_array();
    let a8 = (a * 255.0 + 0.5) as u8;
    let toward = |component: u8| f32::from(component) / 255.0;

    let [r, g, b, a] = if a8 == 0 {
        [r / 2.0, g / 2.0, b / 2.0, a]
    } else if a8 < 170 {
        // Same integer divisor as the `Color32` path: at a8 = 128 the integer
        // 510/128 = 3 pulls by a third, where a float `2/a` would pull by a
        // quarter.
        let div = (2 * 255 / i32::from(a8)) as u8;
        [
            r / 2.0 + toward(target.r() / div),
            g / 2.0 + toward(target.g() / div),
            b / 2.0 + toward(target.b() / div),
            a / 2.0,
        ]
    } else {
        [
            r / 2.0 + toward(target.r() / 2),
            g / 2.0 + toward(target.g() / 2),
            b / 2.0 + toward(target.b() / 2),
            a,
        ]
    };
    *color = HdrVertexColor::from_rgba_premultiplied([r, g, b, a]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use epaint::{
        Mesh, TessellationOptions, Tessellator, TextShape,
        text::{FontDefinitions, TextOptions},
    };

    fn four() -> HdrVertexColor {
        HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0])
    }

    fn new_painter() -> Painter {
        Painter::new(Context::default(), LayerId::background(), Rect::EVERYTHING)
    }

    /// The single shape a painter recorded, read back after `transform_shape`
    /// has run — exactly what the tessellator would receive.
    fn one_shape(painter: &Painter) -> Shape {
        let mut recorded = Vec::new();
        painter.for_each_shape(|clipped| recorded.push(clipped.shape.clone()));
        let mut shapes = recorded.into_iter();
        let shape = shapes.next().expect("expected one shape");
        assert!(shapes.next().is_none(), "expected exactly one shape");
        shape
    }

    /// `a8 >= 170`: only rgb moves, alpha is untouched.
    #[test]
    fn tint_hdr_towards_keeps_alpha_above_the_threshold() {
        let target = Color32::from_rgb(255, 0, 0);
        let mut color = HdrVertexColor::from_rgba_premultiplied([0.5, 0.5, 0.5, 1.0]);
        tint_hdr_towards(&mut color, target);
        let [r, g, b, a] = color.to_array();
        assert_eq!(a, 1.0, "a8 = 255 is the `a8 >= 170` branch");
        let toward = |component: u8| f32::from(component) / 255.0;
        assert!((r - (0.25 + toward(127))).abs() < 1e-3, "r = {r}");
        assert!((g - 0.25).abs() < 1e-3, "g = {g}");
        assert!((b - 0.25).abs() < 1e-3, "b = {b}");
    }

    /// `a8 < 170`: the middle branch halves alpha and pulls by the *integer*
    /// `2 * 255 / a8` divisor (3 at a8 = 128, not the float `2 / a`'s 4).
    #[test]
    fn tint_hdr_towards_uses_the_integer_divisor_below_the_threshold() {
        let target = Color32::from_rgb(255, 0, 0);
        let mut color = HdrVertexColor::from_rgba_premultiplied([0.5, 0.5, 0.5, 0.5]);
        assert_eq!((0.5_f32 * 255.0 + 0.5) as u8, 128, "a8 = 128");
        tint_hdr_towards(&mut color, target);
        let [r, g, b, a] = color.to_array();
        assert!((a - 0.25).abs() < 1e-3, "alpha halves: {a}");
        // 255 / 3 / 255 = 1/3, where a float `2 / a` divisor would give 1/4.
        assert!((r - (0.25 + 1.0 / 3.0)).abs() < 1e-3, "r = {r}");
        assert!((g - 0.25).abs() < 1e-3, "g = {g}");
        assert!((b - 0.25).abs() < 1e-3, "b = {b}");
    }

    /// `a8 == 0`: rgb halves and alpha stays zero, so a transparent value
    /// cannot acquire coverage on a fade.
    #[test]
    fn tint_hdr_towards_leaves_zero_alpha_at_zero() {
        let target = Color32::from_rgb(255, 0, 0);
        let mut color = HdrVertexColor::from_rgba_premultiplied([0.5, 0.5, 0.5, 0.0]);
        tint_hdr_towards(&mut color, target);
        let [r, g, b, a] = color.to_array();
        assert_eq!(a, 0.0);
        assert!((r - 0.25).abs() < 1e-3);
        assert!((g - 0.25).abs() < 1e-3);
        assert!((b - 0.25).abs() < 1e-3);
    }

    /// A wrapped shape dims through its inner alpha only: dimming the mode as
    /// well would apply the factor twice.
    #[test]
    fn opacity_dims_a_wrapped_shape_once() {
        let mut painter = new_painter();
        painter.multiply_opacity(0.5);
        painter.add(Shape::hdr(
            Shape::rect_filled(Rect::EVERYTHING, 0.0, Color32::WHITE),
            four(),
        ));

        let Shape::Hdr { inner, color } = one_shape(&painter) else {
            panic!("expected the wrapper");
        };
        assert_eq!(
            color,
            HdrColorMode::Solid(four()),
            "the wrapper's mask must not be dimmed"
        );
        let Shape::Rect(rect_shape) = &*inner else {
            panic!("expected the inner rect");
        };
        assert_eq!(
            rect_shape.fill,
            Color32::WHITE.gamma_multiply(0.5),
            "the inner fill is the single dimming path"
        );
    }

    /// A bare HDR mesh has no inner alpha to lean on, so `adjust_colors` dims
    /// every sidecar entry — once.
    #[test]
    fn opacity_dims_a_bare_hdr_mesh_once() {
        let mut painter = new_painter();
        painter.multiply_opacity(0.5);

        let mut mesh = Mesh::default();
        mesh.add_colored_rect_hdr(Rect::EVERYTHING, four());
        painter.add(Shape::Mesh(mesh.into()));

        let Shape::Mesh(mesh) = one_shape(&painter) else {
            panic!("expected the mesh");
        };
        let colors = mesh.hdr_colors.as_ref().expect("sidecar");
        assert_eq!(colors.len(), 4);
        for color in colors {
            assert_eq!(
                color.to_array(),
                [2.0, 2.0, 2.0, 0.5],
                "one halving, not two"
            );
        }
    }

    /// HDR text dims through the vertex alpha the `Color32` fold writes; the
    /// field is untouched, so the derived sidecar lands on exactly one factor.
    #[test]
    fn opacity_dims_hdr_text_once() {
        let mut fonts =
            epaint::text::Fonts::new(TextOptions::default(), FontDefinitions::default());
        let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
            "A".to_owned(),
            FontId::proportional(24.0),
            Color32::WHITE,
        );

        let mut painter = new_painter();
        painter.multiply_opacity(0.5);
        painter.add(TextShape::new(Pos2::ZERO, galley, Color32::WHITE).with_hdr_color(four()));

        let Shape::Text(text) = one_shape(&painter) else {
            panic!("expected the text");
        };
        assert_eq!(
            text.hdr_color,
            Some(four()),
            "the text field must not be dimmed; the vertex alpha is the single path"
        );

        // Tessellate what the painter recorded: the derived sidecar is the HDR
        // value at the already-dimmed coverage.
        let mut mesh = Mesh::default();
        let options = TessellationOptions {
            feathering: false,
            ..Default::default()
        };
        Tessellator::new(1.0, options, [1024, 1024], vec![]).tessellate_text(&text, &mut mesh);
        let colors = mesh.hdr_colors.as_ref().expect("sidecar");
        assert_eq!(colors.len(), mesh.vertices.len());
        let peak = colors
            .iter()
            .map(|color| color.to_array()[0])
            .fold(0.0_f32, f32::max);
        // 4.0 × 127/255 ≈ 1.99 for one halving; 4.0 × 0.25 would be a second.
        assert!(
            (1.9..=2.1).contains(&peak),
            "the sidecar must be dimmed exactly once, peak was {peak}"
        );
    }

    /// The fade path is the same traversal: a bare HDR mesh tints once, and
    /// the wrapper is not tinted twice.
    #[test]
    fn fade_tints_each_hdr_level_once() {
        let target = Color32::from_rgb(255, 0, 0);

        let mut painter = new_painter();
        painter.fade_to_color = Some(target);

        let mut mesh = Mesh::default();
        mesh.add_colored_rect_hdr(Rect::EVERYTHING, four());
        painter.add(Shape::Mesh(mesh.into()));
        painter.add(Shape::hdr(
            Shape::rect_filled(Rect::EVERYTHING, 0.0, Color32::WHITE),
            four(),
        ));

        let mut recorded = Vec::new();
        painter.for_each_shape(|clipped| recorded.push(clipped.shape.clone()));
        assert_eq!(recorded.len(), 2);

        let Shape::Mesh(mesh) = &recorded[0] else {
            panic!("expected the mesh first");
        };
        let mut wanted = four();
        tint_hdr_towards(&mut wanted, target);
        for color in mesh.hdr_colors.as_ref().expect("sidecar") {
            assert_eq!(*color, wanted, "the bare mesh's sidecar is tinted once");
        }

        let Shape::Hdr { color, .. } = &recorded[1] else {
            panic!("expected the wrapper second");
        };
        assert_eq!(
            *color,
            HdrColorMode::Solid(four()),
            "the wrapper's mask must not be tinted"
        );
    }
}
