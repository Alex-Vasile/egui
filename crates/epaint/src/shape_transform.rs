use alloc::sync::Arc;

use crate::{
    BandShape, CircleShape, Color32, ColorMode, CubicBezierShape, EllipseShape, HdrVertexColor,
    Mesh, PathShape, QuadraticBezierShape, RectShape, Shape, TextShape, color,
};

/// Remember to handle [`Color32::PLACEHOLDER`] specially!
///
/// `adjust_color` folds the SDR values (vertex colours, fills, strokes, galley
/// colours) and `adjust_hdr` folds the HDR values
/// ([`Mesh::hdr_colors`](crate::Mesh::hdr_colors)). One traversal, two value
/// spaces — the two are separate because an HDR value is not a `Color32` and a
/// conversion would lose the tier.
///
/// The three HDR levels dim differently, and the asymmetry is deliberate:
///
/// - [`Shape::Hdr`]: recurse into `inner` only. The wrapper's mask *is* the
///   inner alpha, so dimming the mode as well applies the factor twice.
/// - [`Shape::Mesh`]: apply `adjust_hdr` to every sidecar entry. Otherwise a
///   directly-built HDR mesh would ignore `multiply_opacity`/`fade_to_color`
///   on wgpu, where only the (SDR-twin) `Color32` fold is otherwise reached.
/// - [`Shape::Text`]: **do not** apply `adjust_hdr` to
///   [`TextShape::hdr_color`]. The text sidecar is derived as
///   `hdr_color × vertex alpha` at tessellation, and that alpha is exactly what
///   the `Color32` fold writes; dimming the field too would double-dim.
pub fn adjust_colors(
    shape: &mut Shape,
    adjust_color: impl Fn(&mut Color32) + Send + Sync + Copy + 'static,
    adjust_hdr: impl Fn(&mut HdrVertexColor) + Send + Sync + Copy + 'static,
) {
    #![expect(clippy::match_same_arms)]
    match shape {
        Shape::Noop => {}

        Shape::Vec(shapes) => {
            for shape in shapes {
                adjust_colors(shape, adjust_color, adjust_hdr);
            }
        }

        Shape::Hdr { inner, color: _ } => {
            // The HDR mode is not dimmed: the inner alpha is the single dimming
            // path for a wrapped shape.
            adjust_colors(Arc::make_mut(inner), adjust_color, adjust_hdr);
        }

        Shape::LineSegment { stroke, points: _ } => {
            adjust_color(&mut stroke.color);
        }

        Shape::Path(PathShape {
            points: _,
            closed: _,
            fill,
            stroke,
        })
        | Shape::QuadraticBezier(QuadraticBezierShape {
            points: _,
            closed: _,
            fill,
            stroke,
        })
        | Shape::CubicBezier(CubicBezierShape {
            points: _,
            closed: _,
            fill,
            stroke,
        }) => {
            adjust_color(fill);
            adjust_color_mode(&mut stroke.color, adjust_color);
        }

        Shape::Band(BandShape {
            points: _,
            fill,
            stroke,
            stroke_kind: _,
            angle: _,
        }) => {
            adjust_color(fill);
            adjust_color(&mut stroke.color);
        }

        Shape::Circle(CircleShape {
            center: _,
            radius: _,
            fill,
            stroke,
        })
        | Shape::Ellipse(EllipseShape {
            center: _,
            radius: _,
            fill,
            stroke,
            angle: _,
        })
        | Shape::Rect(RectShape {
            rect: _,
            corner_radius: _,
            fill,
            stroke,
            stroke_kind: _,
            round_to_pixels: _,
            blur_width: _,
            brush: _,
            angle: _,
        }) => {
            adjust_color(fill);
            adjust_color(&mut stroke.color);
        }

        Shape::Text(TextShape {
            pos: _,
            galley,
            underline,
            fallback_color,
            override_text_color,
            opacity_factor: _,
            angle: _,
            // Deliberately not dimmed here: the derived sidecar follows the
            // vertex alpha this fold writes, so dimming the field would apply
            // the factor twice.
            hdr_color: _,
        }) => {
            adjust_color(&mut underline.color);
            adjust_color(fallback_color);
            if let Some(override_text_color) = override_text_color {
                adjust_color(override_text_color);
            }

            if !galley.is_empty() {
                let galley = Arc::make_mut(galley);
                for placed_row in &mut galley.rows {
                    let row = Arc::make_mut(&mut placed_row.row);
                    for vertex in &mut row.visuals.mesh.vertices {
                        adjust_color(&mut vertex.color);
                    }
                }
            }
        }

        Shape::Mesh(mesh) => {
            let Mesh {
                indices: _,
                vertices,
                texture_id: _,
                hdr_colors,
            } = Arc::make_mut(mesh);

            for v in vertices {
                adjust_color(&mut v.color);
            }
            if let Some(hdr_colors) = hdr_colors {
                for color in hdr_colors {
                    adjust_hdr(color);
                }
            }
        }

        Shape::Callback(_) => {
            // Can't tint user callback code
        }
    }
}

fn adjust_color_mode(
    color_mode: &mut ColorMode,
    adjust_color: impl Fn(&mut Color32) + Send + Sync + Copy + 'static,
) {
    match color_mode {
        color::ColorMode::Solid(color) => adjust_color(color),
        color::ColorMode::UV(callback) => {
            let callback = Arc::clone(callback);
            *color_mode = color::ColorMode::UV(Arc::new(Box::new(move |rect, pos| {
                let mut color = callback(rect, pos);
                adjust_color(&mut color);
                color
            })));
        }
    }
}
