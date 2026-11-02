use alloc::sync::Arc;
use core::fmt::Debug;

use ecolor::Color32;
use emath::{Pos2, Rect};

use crate::HdrVertexColor;

/// How paths will be colored.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ColorMode {
    /// The entire path is one solid color, this is the default.
    Solid(Color32),

    /// Provide a callback which takes in the path's bounding box and a position and converts it to a color.
    /// When used with a path, the bounding box will have a margin of [`TessellationOptions::feathering_size_in_pixels`](`crate::tessellator::TessellationOptions::feathering_size_in_pixels`)
    ///
    /// **This cannot be serialized**
    #[cfg_attr(feature = "serde", serde(skip))]
    UV(Arc<dyn Fn(Rect, Pos2) -> Color32 + Send + Sync>),
}

impl Default for ColorMode {
    fn default() -> Self {
        Self::Solid(Color32::TRANSPARENT)
    }
}

impl Debug for ColorMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Solid(arg0) => f.debug_tuple("Solid").field(arg0).finish(),
            Self::UV(_arg0) => f.debug_tuple("UV").field(&"<closure>").finish(),
        }
    }
}

impl PartialEq for ColorMode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Solid(l0), Self::Solid(r0)) => l0 == r0,
            (Self::UV(_l0), Self::UV(_r0)) => false,
            _ => false,
        }
    }
}

impl ColorMode {
    pub const TRANSPARENT: Self = Self::Solid(Color32::TRANSPARENT);
}

/// How an HDR-colored shape ([`Shape::Hdr`](crate::Shape::Hdr)) evaluates its
/// color per vertex.
///
/// The HDR sibling of [`ColorMode`], with the same two-variant layout (measured
/// 16 B, so the wrapper payload stays under [`Shape`](crate::Shape)'s size
/// budget) and the same hand-written `Debug`/`PartialEq` treatment. There is
/// no serde impl: the wrapper never crosses a serialization boundary, because
/// [`Shape`](crate::Shape) is not serde-derived.
///
/// This is an *operator over coverage*: the shape's own alpha is the mask, and
/// the value here is what the mask is filled with. It is not the way to carry
/// per-vertex HDR data a caller already has — that is
/// [`Mesh::hdr_colors`](crate::Mesh::hdr_colors) on a hand-built mesh.
#[derive(Clone)]
pub enum HdrColorMode {
    /// One HDR value for the whole shape.
    Solid(HdrVertexColor),

    /// Provide a callback which takes in the shape's bounding box and a
    /// position and converts it to an HDR value.
    ///
    /// The bounding box has the same feathering margin [`ColorMode::UV`] gets
    /// (the shape's visual bounds expanded by the tessellator's feathering
    /// size).
    UV(Arc<dyn Fn(Rect, Pos2) -> HdrVertexColor + Send + Sync>),
}

impl Default for HdrColorMode {
    fn default() -> Self {
        Self::Solid(HdrVertexColor::TRANSPARENT)
    }
}

impl Debug for HdrColorMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Solid(arg0) => f.debug_tuple("Solid").field(arg0).finish(),
            Self::UV(_arg0) => f.debug_tuple("UV").field(&"<closure>").finish(),
        }
    }
}

impl PartialEq for HdrColorMode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Solid(l0), Self::Solid(r0)) => l0 == r0,
            // Like [`ColorMode::UV`]: two closures are never equal.
            (Self::UV(_l0), Self::UV(_r0)) => false,
            _ => false,
        }
    }
}

impl From<HdrVertexColor> for HdrColorMode {
    #[inline]
    fn from(color: HdrVertexColor) -> Self {
        Self::Solid(color)
    }
}

#[test]
fn hdr_color_mode_is_the_same_size_as_color_mode() {
    // The wrapper's payload must stay under `PathShape`'s size, so that
    // `Shape` does not grow. Both modes are the same fat-pointer niche layout.
    assert_eq!(core::mem::size_of::<ColorMode>(), 16);
    assert_eq!(core::mem::size_of::<HdrColorMode>(), 16);
}
