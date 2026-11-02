//! The different shapes that can be painted.

use alloc::sync::Arc;

use emath::{Align2, Pos2, Rangef, Rect, Rot2, TSTransform, Vec2, pos2};

use crate::{
    Color32, CornerRadius, Direction, HdrColorMode, HdrVertexColor, Mesh, Stroke, StrokeKind,
    TextureId, Vertex,
    stroke::PathStroke,
    text::{FontId, FontsView, Galley},
};

use super::{
    BandShape, CircleShape, CubicBezierShape, EllipseShape, PaintCallback, PathShape,
    QuadraticBezierShape, RectShape, TextShape,
};

/// A paint primitive such as a circle or a piece of text.
/// Coordinates are all screen space points (not physical pixels).
///
/// You should generally recreate your [`Shape`]s each frame,
/// but storing them should also be fine with one exception:
/// [`Shape::Text`] depends on the current `pixels_per_point` (dpi scale)
/// and so must be recreated every time `pixels_per_point` changes.
#[must_use = "Add a Shape to a Painter"]
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Paint nothing. This can be useful as a placeholder.
    Noop,

    /// Recursively nest more shapes - sometimes a convenience to be able to do.
    /// For performance reasons it is better to avoid it.
    Vec(Vec<Self>),

    /// Circle with optional outline and fill.
    Circle(CircleShape),

    /// Ellipse with optional outline and fill.
    Ellipse(EllipseShape),

    /// A line between two points.
    LineSegment { points: [Pos2; 2], stroke: Stroke },

    /// A series of lines between points.
    /// The path can have a stroke and/or fill (if closed).
    ///
    /// If you want a path of varying width, use [`Self::Band`] instead.
    Path(PathShape),

    /// A varying-width band along a direction.
    ///
    /// If you want a path of fixed width, use [`Self::Path`] instead.
    Band(BandShape),

    /// Rectangle with optional outline and fill.
    Rect(RectShape),

    /// Text.
    ///
    /// This needs to be recreated if `pixels_per_point` (dpi scale) changes.
    Text(TextShape),

    /// A general triangle mesh.
    ///
    /// Can be used to display images.
    ///
    /// Wrapped in an [`Arc`] to minimize the size of [`Shape`].
    Mesh(Arc<Mesh>),

    /// Paint an already-tessellated shape in an HDR color, using its
    /// **coverage** (alpha) as the mask.
    ///
    /// This is the operator that lets any shape — strokes, polylines, fills,
    /// images — be colored above SDR white without a per-shape field. The
    /// inner shape supplies the geometry and the alpha; the mode supplies the
    /// color. Build it with [`Shape::hdr`].
    ///
    /// **Uniform recolour.** A shape's fill and stroke collapse to one value,
    /// and an inner [`ColorMode::UV`](crate::ColorMode::UV) ramp is lost — use
    /// [`HdrColorMode::UV`](crate::HdrColorMode::UV),
    /// [`Shape::gradient_rect_hdr`], or a hand-built mesh with
    /// [`Mesh::hdr_colors`] for per-vertex variation. For textured content the
    /// wrapper is a tint: the texels keep multiplying underneath.
    Hdr {
        /// The shape whose coverage is recoloured.
        inner: Arc<Self>,

        /// How the color is evaluated per vertex.
        color: HdrColorMode,
    },

    /// A quadratic [Bézier Curve](https://en.wikipedia.org/wiki/B%C3%A9zier_curve).
    QuadraticBezier(QuadraticBezierShape),

    /// A cubic [Bézier Curve](https://en.wikipedia.org/wiki/B%C3%A9zier_curve).
    CubicBezier(CubicBezierShape),

    /// Backend-specific painting.
    Callback(PaintCallback),
}

#[test]
fn shape_size() {
    assert_eq!(
        core::mem::size_of::<Shape>(),
        64,
        "Shape changed size! If it shrank - good! Update this test. If it grew - bad! Try to find a way to avoid it."
    );
    assert!(
        core::mem::size_of::<Shape>() <= 64,
        "Shape is getting way too big!"
    );

    // The HDR tier's payloads must stay under `PathShape`'s 56 B ceiling, or
    // `Shape` grows. These are the measured numbers the design rests on.
    assert_eq!(
        core::mem::size_of::<HdrColorMode>(),
        16,
        "HdrColorMode is a two-variant fat-pointer niche layout; a third variant would be 24 B"
    );
    assert_eq!(
        core::mem::size_of::<(Arc<Shape>, HdrColorMode)>(),
        24,
        "the Shape::Hdr payload must stay at 24 B"
    );
    assert_eq!(
        core::mem::size_of::<crate::ColorMode>(),
        16,
        "ColorMode must not grow: it feeds PathStroke > PathShape > Shape"
    );
    assert_eq!(
        core::mem::size_of::<crate::Stroke>(),
        8,
        "Stroke must not grow: it feeds Shape::LineSegment"
    );
    assert_eq!(
        core::mem::size_of::<TextShape>(),
        56,
        "TextShape has room for exactly one Option<HdrVertexColor>"
    );
    assert_eq!(
        core::mem::size_of::<crate::Vertex>(),
        20,
        "Vertex must not grow: the sidecar is where HDR color lives"
    );
    assert_eq!(
        core::mem::size_of::<crate::PathStroke>(),
        24,
        "PathStroke must not grow: it feeds PathShape"
    );
    assert_eq!(core::mem::size_of::<crate::PathShape>(), 56);
    assert_eq!(core::mem::size_of::<crate::RectShape>(), 56);
}

// Compile-time check that `Shape` is `Send + Sync` on every target.
// Deliberately not a `#[test]`: tests never run on wasm, but `cargo check` does.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Shape>();
};

impl From<Vec<Self>> for Shape {
    #[inline(always)]
    fn from(shapes: Vec<Self>) -> Self {
        Self::Vec(shapes)
    }
}

impl From<Mesh> for Shape {
    #[inline(always)]
    fn from(mesh: Mesh) -> Self {
        Self::Mesh(mesh.into())
    }
}

impl From<Arc<Mesh>> for Shape {
    #[inline(always)]
    fn from(mesh: Arc<Mesh>) -> Self {
        Self::Mesh(mesh)
    }
}

/// ## Constructors
impl Shape {
    /// A line between two points.
    /// More efficient than calling [`Self::line`].
    #[inline]
    pub fn line_segment(points: [Pos2; 2], stroke: impl Into<Stroke>) -> Self {
        Self::LineSegment {
            points,
            stroke: stroke.into(),
        }
    }

    /// A horizontal line.
    pub fn hline(x: impl Into<Rangef>, y: f32, stroke: impl Into<Stroke>) -> Self {
        let x = x.into();
        Self::LineSegment {
            points: [pos2(x.min, y), pos2(x.max, y)],
            stroke: stroke.into(),
        }
    }

    /// A vertical line.
    pub fn vline(x: f32, y: impl Into<Rangef>, stroke: impl Into<Stroke>) -> Self {
        let y = y.into();
        Self::LineSegment {
            points: [pos2(x, y.min), pos2(x, y.max)],
            stroke: stroke.into(),
        }
    }

    /// A line through many points.
    ///
    /// Use [`Self::line_segment`] instead if your line only connects two points.
    #[inline]
    pub fn line(points: Vec<Pos2>, stroke: impl Into<PathStroke>) -> Self {
        Self::Path(PathShape::line(points, stroke))
    }

    /// A line that closes back to the start point again.
    #[inline]
    pub fn closed_line(points: Vec<Pos2>, stroke: impl Into<PathStroke>) -> Self {
        Self::Path(PathShape::closed_line(points, stroke))
    }

    /// Turn a line into equally spaced dots.
    pub fn dotted_line(
        path: &[Pos2],
        color: impl Into<Color32>,
        spacing: f32,
        radius: f32,
    ) -> Vec<Self> {
        let mut shapes = Vec::new();
        points_from_line(path, spacing, radius, color.into(), &mut shapes);
        shapes
    }

    /// Turn a line into dashes.
    pub fn dashed_line(
        path: &[Pos2],
        stroke: impl Into<Stroke>,
        dash_length: f32,
        gap_length: f32,
    ) -> Vec<Self> {
        let mut shapes = Vec::new();
        dashes_from_line(
            path,
            stroke.into(),
            &[dash_length],
            &[gap_length],
            &mut shapes,
            0.,
        );
        shapes
    }

    /// Turn a line into dashes with different dash/gap lengths and a start offset.
    pub fn dashed_line_with_offset(
        path: &[Pos2],
        stroke: impl Into<Stroke>,
        dash_lengths: &[f32],
        gap_lengths: &[f32],
        dash_offset: f32,
    ) -> Vec<Self> {
        let mut shapes = Vec::new();
        dashes_from_line(
            path,
            stroke.into(),
            dash_lengths,
            gap_lengths,
            &mut shapes,
            dash_offset,
        );
        shapes
    }

    /// Turn a line into dashes. If you need to create many dashed lines use this instead of
    /// [`Self::dashed_line`].
    pub fn dashed_line_many(
        points: &[Pos2],
        stroke: impl Into<Stroke>,
        dash_length: f32,
        gap_length: f32,
        shapes: &mut Vec<Self>,
    ) {
        dashes_from_line(
            points,
            stroke.into(),
            &[dash_length],
            &[gap_length],
            shapes,
            0.,
        );
    }

    /// Turn a line into dashes with different dash/gap lengths and a start offset. If you need to
    /// create many dashed lines use this instead of [`Self::dashed_line_with_offset`].
    pub fn dashed_line_many_with_offset(
        points: &[Pos2],
        stroke: impl Into<Stroke>,
        dash_lengths: &[f32],
        gap_lengths: &[f32],
        dash_offset: f32,
        shapes: &mut Vec<Self>,
    ) {
        dashes_from_line(
            points,
            stroke.into(),
            dash_lengths,
            gap_lengths,
            shapes,
            dash_offset,
        );
    }

    /// A convex polygon with a fill and optional stroke.
    ///
    /// The most performant winding order is clockwise.
    #[inline]
    pub fn convex_polygon(
        points: Vec<Pos2>,
        fill: impl Into<Color32>,
        stroke: impl Into<PathStroke>,
    ) -> Self {
        Self::Path(PathShape::convex_polygon(points, fill, stroke))
    }

    /// A filled triangle inscribed in `rect`, pointing down.
    ///
    /// `rotation` is in radians, and rotates the triangle around the center of `rect`:
    /// `0.0` points down, `TAU / 4.0` left, `TAU / 2.0` up, and `-TAU / 4.0` right.
    ///
    /// Useful to paint small arrow icons in the ui, like the on combo boxes or submenu buttons.
    pub fn rotated_triangle(rect: Rect, rotation: f32, fill: impl Into<Color32>) -> Self {
        let rotation = Rot2::from_angle(rotation);
        let points = [rect.left_top(), rect.right_top(), rect.center_bottom()]
            .into_iter()
            .map(|point| rect.center() + rotation * (point - rect.center()))
            .collect();
        Self::convex_polygon(points, fill, Stroke::NONE)
    }

    #[inline]
    pub fn circle_filled(center: Pos2, radius: f32, fill_color: impl Into<Color32>) -> Self {
        Self::Circle(CircleShape::filled(center, radius, fill_color))
    }

    #[inline]
    pub fn circle_stroke(center: Pos2, radius: f32, stroke: impl Into<Stroke>) -> Self {
        Self::Circle(CircleShape::stroke(center, radius, stroke))
    }

    #[inline]
    pub fn ellipse_filled(center: Pos2, radius: Vec2, fill_color: impl Into<Color32>) -> Self {
        Self::Ellipse(EllipseShape::filled(center, radius, fill_color))
    }

    #[inline]
    pub fn ellipse_stroke(center: Pos2, radius: Vec2, stroke: impl Into<Stroke>) -> Self {
        Self::Ellipse(EllipseShape::stroke(center, radius, stroke))
    }

    /// See also [`Self::rect_stroke`].
    #[inline]
    pub fn rect_filled(
        rect: Rect,
        corner_radius: impl Into<CornerRadius>,
        fill_color: impl Into<Color32>,
    ) -> Self {
        Self::Rect(RectShape::filled(rect, corner_radius, fill_color))
    }

    /// See also [`Self::rect_filled`].
    #[inline]
    pub fn rect_stroke(
        rect: Rect,
        corner_radius: impl Into<CornerRadius>,
        stroke: impl Into<Stroke>,
        stroke_kind: StrokeKind,
    ) -> Self {
        Self::Rect(RectShape::stroke(rect, corner_radius, stroke, stroke_kind))
    }

    /// Paints a gradient rectangle that transitions from `color_from` to `color_to`
    /// along the given `direction`.
    ///
    /// For example, [`Direction::TopDown`] paints `color_from` at the top edge fading
    /// to `color_to` at the bottom edge.
    #[inline]
    pub fn gradient_rect(rect: Rect, direction: Direction, [from, to]: [Color32; 2]) -> Self {
        let (left_top, right_top, left_bottom, right_bottom) = match direction {
            Direction::TopDown => (from, from, to, to),
            Direction::BottomUp => (to, to, from, from),
            Direction::LeftToRight => (from, to, from, to),
            Direction::RightToLeft => (to, from, to, from),
        };

        Self::from(Mesh {
            indices: vec![0, 1, 2, 2, 1, 3],
            vertices: vec![
                Vertex::untextured(rect.left_top(), left_top),
                Vertex::untextured(rect.right_top(), right_top),
                Vertex::untextured(rect.left_bottom(), left_bottom),
                Vertex::untextured(rect.right_bottom(), right_bottom),
            ],
            texture_id: Default::default(),
            hdr_colors: None,
        })
    }

    /// Paints a gradient rectangle whose colors are not capped at SDR white.
    ///
    /// The HDR mirror of [`Self::gradient_rect`]: same direction semantics,
    /// same corner assignment, plus the per-vertex sidecar. The four
    /// [`Vertex::color`] twins are the legal SDR projections
    /// ([`HdrVertexColor::projected_color32`]), so an SDR-only backend draws
    /// the range-reduced hue instead of discarding the rectangle.
    #[inline]
    pub fn gradient_rect_hdr(
        rect: Rect,
        direction: Direction,
        [from, to]: [HdrVertexColor; 2],
    ) -> Self {
        let (left_top, right_top, left_bottom, right_bottom) = match direction {
            Direction::TopDown => (from, from, to, to),
            Direction::BottomUp => (to, to, from, from),
            Direction::LeftToRight => (from, to, from, to),
            Direction::RightToLeft => (to, from, to, from),
        };

        Self::from(Mesh {
            indices: vec![0, 1, 2, 2, 1, 3],
            vertices: vec![
                Vertex::untextured(rect.left_top(), left_top.projected_color32()),
                Vertex::untextured(rect.right_top(), right_top.projected_color32()),
                Vertex::untextured(rect.left_bottom(), left_bottom.projected_color32()),
                Vertex::untextured(rect.right_bottom(), right_bottom.projected_color32()),
            ],
            texture_id: Default::default(),
            hdr_colors: Some(vec![left_top, right_top, left_bottom, right_bottom]),
        })
    }

    /// Paint `shape` in the given HDR color, using its coverage (alpha) as the
    /// mask.
    ///
    /// This is the generic HDR painting mechanism: every shape — a stroke, a
    /// polyline, a fill, an image — can be coloured above SDR white without a
    /// per-shape field, because the geometry and the alpha already exist and
    /// only the colour is replaced. See [`Shape::Hdr`] for the guarantees and
    /// limitations.
    ///
    /// `Shape::Vec` is flattened recursively, so each child keeps its own
    /// primitive split and its own tier decision, and a `Shape::Callback`
    /// passes through **uncoloured** (a callback owns its pipeline; there is
    /// nothing to recolour). Constructing a wrapper around an existing wrapper
    /// collapses to the outer colour.
    pub fn hdr(shape: impl Into<Self>, color: impl Into<HdrColorMode>) -> Self {
        let mut out = Vec::new();
        Self::wrap_into(shape.into(), color.into(), &mut out);
        if out.len() == 1 {
            out.pop().unwrap_or(Self::Noop)
        } else {
            Self::Vec(out)
        }
    }

    fn wrap_into(shape: Self, color: HdrColorMode, out: &mut Vec<Self>) {
        match shape {
            Self::Vec(shapes) => {
                for shape in shapes {
                    Self::wrap_into(shape, color.clone(), out);
                }
            }
            // A callback owns its pipeline: nothing to recolour, so it passes
            // through uncoloured (deliberately not a debug assertion).
            Self::Callback(callback) => out.push(Self::Callback(callback)),
            // The inner wrapper's colours are already baked into its alpha
            // mask, so a second wrapper collapses to the outer colour.
            Self::Hdr { inner, color: _ } => out.push(Self::Hdr { inner, color }),
            other => out.push(Self::Hdr {
                inner: Arc::new(other),
                color,
            }),
        }
    }

    /// Does this shape carry the HDR vertex-color tier?
    ///
    /// Total over the three producers: the [`Self::Hdr`] operator, the
    /// [`TextShape::hdr_color`] field, and a hand-built [`Mesh`] with a
    /// sidecar. The tessellator's mesh-grouping key uses this to keep HDR and
    /// SDR content in separate primitives, so this method has to be complete —
    /// a missed arm silently promotes the neighbouring SDR geometry.
    #[must_use]
    pub fn is_hdr(&self) -> bool {
        match self {
            Self::Hdr { .. } => true,
            Self::Text(text) => text.hdr_color.is_some(),
            Self::Mesh(mesh) => mesh.is_hdr(),
            _ => false,
        }
    }

    #[expect(clippy::needless_pass_by_value)]
    pub fn text(
        fonts: &mut FontsView<'_>,
        pos: Pos2,
        anchor: Align2,
        text: impl ToString,
        font_id: FontId,
        color: Color32,
    ) -> Self {
        let galley = fonts.layout_no_wrap(text.to_string(), font_id, color);
        let rect = anchor.anchor_size(pos, galley.size());
        Self::galley(rect.min, galley, color)
    }

    /// Any uncolored parts of the [`Galley`] (using [`Color32::PLACEHOLDER`]) will be replaced with the given color.
    ///
    /// Any non-placeholder color in the galley takes precedence over this fallback color.
    #[inline]
    pub fn galley(pos: Pos2, galley: Arc<Galley>, fallback_color: Color32) -> Self {
        TextShape::new(pos, galley, fallback_color).into()
    }

    /// All text color in the [`Galley`] will be replaced with the given color.
    #[inline]
    pub fn galley_with_override_text_color(
        pos: Pos2,
        galley: Arc<Galley>,
        text_color: Color32,
    ) -> Self {
        TextShape::new(pos, galley, text_color)
            .with_override_text_color(text_color)
            .into()
    }

    #[inline]
    pub fn mesh(mesh: impl Into<Arc<Mesh>>) -> Self {
        let mesh = mesh.into();
        debug_assert!(mesh.is_valid(), "Invalid mesh: {mesh:#?}");
        Self::Mesh(mesh)
    }

    /// An image at the given position.
    ///
    /// `uv` should normally be `Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))`
    /// unless you want to crop or flip the image.
    ///
    /// `tint` is a color multiplier. Use [`Color32::WHITE`] if you don't want to tint the image.
    pub fn image(texture_id: TextureId, rect: Rect, uv: Rect, tint: Color32) -> Self {
        let mut mesh = Mesh::with_texture(texture_id);
        mesh.add_rect_with_uv(rect, uv, tint);
        Self::mesh(mesh)
    }

    /// The visual bounding rectangle (includes stroke widths)
    pub fn visual_bounding_rect(&self) -> Rect {
        match self {
            Self::Noop => Rect::NOTHING,
            Self::Vec(shapes) => {
                let mut rect = Rect::NOTHING;
                for shape in shapes {
                    rect |= shape.visual_bounding_rect();
                }
                rect
            }
            Self::Circle(circle_shape) => circle_shape.visual_bounding_rect(),
            Self::Ellipse(ellipse_shape) => ellipse_shape.visual_bounding_rect(),
            Self::LineSegment { points, stroke } => {
                if stroke.is_empty() {
                    Rect::NOTHING
                } else {
                    Rect::from_two_pos(points[0], points[1]).expand(stroke.width * 0.5)
                }
            }
            Self::Path(path_shape) => path_shape.visual_bounding_rect(),
            Self::Band(band_shape) => band_shape.visual_bounding_rect(),
            Self::Rect(rect_shape) => rect_shape.visual_bounding_rect(),
            Self::Text(text_shape) => text_shape.visual_bounding_rect(),
            Self::Mesh(mesh) => mesh.calc_bounds(),
            // The wrapper changes colour only; the geometry is the inner shape's.
            Self::Hdr { inner, color: _ } => inner.visual_bounding_rect(),
            Self::QuadraticBezier(bezier) => bezier.visual_bounding_rect(),
            Self::CubicBezier(bezier) => bezier.visual_bounding_rect(),
            Self::Callback(custom) => custom.rect,
        }
    }
}

/// ## Inspection and transforms
impl Shape {
    #[inline(always)]
    pub fn texture_id(&self) -> crate::TextureId {
        if let Self::Mesh(mesh) = self {
            mesh.texture_id
        } else if let Self::Rect(rect_shape) = self {
            rect_shape.fill_texture_id()
        } else if let Self::Hdr { inner, color: _ } = self {
            // Delegation is mandatory: the tessellator's mesh-grouping key
            // reads this, and a wrapper over a textured mesh must start its own
            // primitive rather than be appended across textures.
            inner.texture_id()
        } else {
            crate::TextureId::default()
        }
    }

    /// Scale the shape by `factor`, in-place.
    ///
    /// A wrapper around [`Self::transform`].
    #[inline(always)]
    pub fn scale(&mut self, factor: f32) {
        self.transform(TSTransform::from_scaling(factor));
    }

    /// Move the shape by `delta`, in-place.
    ///
    /// A wrapper around [`Self::transform`].
    #[inline(always)]
    pub fn translate(&mut self, delta: Vec2) {
        self.transform(TSTransform::from_translation(delta));
    }

    /// Transform (move/scale) the shape in-place.
    ///
    /// If using a [`PaintCallback`], note that only the rect is scaled as opposed
    /// to other shapes where the stroke is also scaled.
    pub fn transform(&mut self, transform: TSTransform) {
        match self {
            Self::Noop => {}
            Self::Vec(shapes) => {
                for shape in shapes {
                    shape.transform(transform);
                }
            }
            Self::Circle(circle_shape) => {
                circle_shape.center = transform * circle_shape.center;
                circle_shape.radius *= transform.scaling;
                circle_shape.stroke.width *= transform.scaling;
            }
            Self::Ellipse(ellipse_shape) => {
                ellipse_shape.center = transform * ellipse_shape.center;
                ellipse_shape.radius *= transform.scaling;
                ellipse_shape.stroke.width *= transform.scaling;
            }
            Self::LineSegment { points, stroke } => {
                for p in points {
                    *p = transform * *p;
                }
                stroke.width *= transform.scaling;
            }
            Self::Path(path_shape) => {
                for p in &mut path_shape.points {
                    *p = transform * *p;
                }
                path_shape.stroke.width *= transform.scaling;
            }
            Self::Band(band_shape) => band_shape.transform(transform),
            Self::Rect(rect_shape) => {
                rect_shape.rect = transform * rect_shape.rect;
                rect_shape.corner_radius *= transform.scaling;
                rect_shape.stroke.width *= transform.scaling;
                rect_shape.blur_width *= transform.scaling;
            }
            Self::Text(text_shape) => {
                text_shape.transform(transform);
            }
            Self::Mesh(mesh) => {
                Arc::make_mut(mesh).transform(transform);
            }
            Self::Hdr { inner, color: _ } => {
                // Like `ColorMode::UV`, the colour source is defined against
                // the shape's recomputed bounds, so a ramp moves with the shape
                // instead of shearing. The sidecar holds colours only.
                Arc::make_mut(inner).transform(transform);
            }
            Self::QuadraticBezier(bezier) => {
                for p in &mut bezier.points {
                    *p = transform * *p;
                }
                bezier.stroke.width *= transform.scaling;
            }
            Self::CubicBezier(bezier) => {
                for p in &mut bezier.points {
                    *p = transform * *p;
                }
                bezier.stroke.width *= transform.scaling;
            }
            Self::Callback(shape) => {
                shape.rect = transform * shape.rect;
            }
        }
    }
}

// ----------------------------------------------------------------------------

/// Creates equally spaced filled circles from a line.
fn points_from_line(
    path: &[Pos2],
    spacing: f32,
    radius: f32,
    color: Color32,
    shapes: &mut Vec<Shape>,
) {
    let mut position_on_segment = 0.0;
    for &[start, end] in path.array_windows() {
        let vector = end - start;
        let segment_length = vector.length();
        while position_on_segment < segment_length {
            let new_point = start + vector * (position_on_segment / segment_length);
            shapes.push(Shape::circle_filled(new_point, radius, color));
            position_on_segment += spacing;
        }
        position_on_segment -= segment_length;
    }
}

/// Creates dashes from a line.
fn dashes_from_line(
    path: &[Pos2],
    stroke: Stroke,
    dash_lengths: &[f32],
    gap_lengths: &[f32],
    shapes: &mut Vec<Shape>,
    dash_offset: f32,
) {
    assert_eq!(
        dash_lengths.len(),
        gap_lengths.len(),
        "Mismatched dash and gap lengths, got dash_lengths: {}, gap_lengths: {}",
        dash_lengths.len(),
        gap_lengths.len()
    );
    let mut position_on_segment = dash_offset;
    let mut drawing_dash = false;
    let mut step = 0;
    let steps = dash_lengths.len();
    for &[start, end] in path.array_windows() {
        let vector = end - start;
        let segment_length = vector.length();

        let mut start_point = start;
        while position_on_segment < segment_length {
            let new_point = start + vector * (position_on_segment / segment_length);
            if drawing_dash {
                // This is the end point.
                shapes.push(Shape::line_segment([start_point, new_point], stroke));
                position_on_segment += gap_lengths[step];
                // Increment step counter
                step += 1;
                if step >= steps {
                    step = 0;
                }
            } else {
                // Start a new dash.
                start_point = new_point;
                position_on_segment += dash_lengths[step];
            }
            drawing_dash = !drawing_dash;
        }

        // If the segment ends and the dash is not finished, add the segment's end point.
        if drawing_dash {
            shapes.push(Shape::line_segment([start_point, end], stroke));
        }

        position_on_segment -= segment_length;
    }
}

#[cfg(test)]
mod hdr_tests {
    use super::*;
    use crate::{HdrColorMode, HdrVertexColor};
    use emath::pos2;

    fn callback_shape() -> Shape {
        Shape::Callback(PaintCallback {
            rect: Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            callback: Arc::new(()),
        })
    }

    fn wrap(shape: Shape, color: HdrVertexColor) -> Shape {
        Shape::hdr(shape, color)
    }

    fn solid(color: HdrVertexColor) -> HdrColorMode {
        HdrColorMode::Solid(color)
    }

    fn rect() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0))
    }

    #[test]
    fn hdr_wrapper_flattens_vec_and_passes_callbacks_through() {
        let color = HdrVertexColor::WHITE;
        let shape = wrap(
            Shape::Vec(vec![
                Shape::rect_filled(rect(), 0.0, Color32::WHITE),
                callback_shape(),
                Shape::Vec(vec![Shape::circle_filled(
                    pos2(5.0, 5.0),
                    3.0,
                    Color32::WHITE,
                )]),
            ]),
            color,
        );

        // One wrapper per non-callback child, nested `Vec`s flattened, and the
        // callback straight through (uncoloured).
        let Shape::Vec(children) = shape else {
            panic!("expected a Vec");
        };
        assert_eq!(children.len(), 3, "the callback is kept uncoloured");
        assert!(matches!(children[0], Shape::Hdr { .. }));
        assert!(matches!(children[1], Shape::Callback(_)));
        assert!(matches!(children[2], Shape::Hdr { .. }));
    }

    #[test]
    fn hdr_wrapper_collapses_to_the_outer_color() {
        let color = HdrVertexColor::WHITE;
        let inner = wrap(
            Shape::rect_filled(rect(), 0.0, Color32::WHITE),
            HdrVertexColor::TRANSPARENT,
        );
        let Shape::Hdr { inner, color: _ } = wrap(inner, color) else {
            panic!("expected a wrapper");
        };
        // The inner wrapper is gone: one layer, the outer colour.
        assert!(matches!(*inner, Shape::Rect(_)));
    }

    #[test]
    fn hdr_wrapper_delegates_texture_id() {
        let texture_id = TextureId::User(7);
        let mut mesh = Mesh::with_texture(texture_id);
        mesh.add_rect_with_uv(
            rect(),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let shape = wrap(Shape::mesh(mesh), HdrVertexColor::WHITE);
        assert_eq!(shape.texture_id(), texture_id);
        assert!(shape.is_hdr());
    }

    #[test]
    fn is_hdr_is_complete() {
        assert!(!Shape::Noop.is_hdr());
        assert!(!Shape::rect_filled(rect(), 0.0, Color32::WHITE).is_hdr());
        assert!(wrap(Shape::Noop, HdrVertexColor::WHITE).is_hdr());

        // A hand-built mesh with a sidecar is HDR without any wrapper.
        let mut mesh = Mesh::default();
        mesh.add_colored_rect_hdr(rect(), HdrVertexColor::WHITE);
        assert!(Shape::mesh(mesh.clone()).is_hdr());

        // `Shape::Hdr` transforms and bounds delegate to the inner shape.
        let mut shape = wrap(
            Shape::rect_filled(rect(), 0.0, Color32::WHITE),
            HdrVertexColor::WHITE,
        );
        assert!(shape.visual_bounding_rect().is_positive());
        shape.translate(Vec2::splat(3.0));
        assert_eq!(
            shape.visual_bounding_rect(),
            rect().translate(Vec2::splat(3.0))
        );
    }

    #[test]
    fn gradient_rect_hdr_mirrors_gradient_rect() {
        let colors = [
            Color32::from_rgba_unmultiplied(200, 100, 50, 200),
            Color32::from_rgba_unmultiplied(10, 20, 30, 100),
        ];
        for direction in [
            Direction::TopDown,
            Direction::BottomUp,
            Direction::LeftToRight,
            Direction::RightToLeft,
        ] {
            let sdr = Shape::gradient_rect(rect(), direction, colors);
            let hdr = Shape::gradient_rect_hdr(rect(), direction, colors.map(HdrVertexColor::from));

            let Shape::Mesh(sdr_mesh) = sdr else {
                panic!("gradient_rect must be a mesh");
            };
            let Shape::Mesh(hdr_mesh) = hdr else {
                panic!("gradient_rect_hdr must be a mesh");
            };
            assert_eq!(sdr_mesh.indices, hdr_mesh.indices);
            assert_eq!(sdr_mesh.texture_id, hdr_mesh.texture_id);
            assert_eq!(sdr_mesh.vertices.len(), hdr_mesh.vertices.len());
            assert!(hdr_mesh.hdr_colors.is_some());
            assert!(hdr_mesh.is_valid());

            for (sdr_vertex, hdr_vertex) in core::iter::zip(&sdr_mesh.vertices, &hdr_mesh.vertices)
            {
                assert_eq!(sdr_vertex.pos, hdr_vertex.pos);
                assert_eq!(sdr_vertex.uv, hdr_vertex.uv);
                for channel in 0..4 {
                    assert!(
                        sdr_vertex.color[channel].abs_diff(hdr_vertex.color[channel]) <= 1,
                        "SDR twin must match the SDR gradient: {sdr_vertex:?} vs {hdr_vertex:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn hdr_color_mode_is_solid_from_a_value() {
        let mode: HdrColorMode = HdrVertexColor::WHITE.into();
        assert!(matches!(mode, HdrColorMode::Solid(_)));
        assert_eq!(mode, solid(HdrVertexColor::WHITE));
    }
}
