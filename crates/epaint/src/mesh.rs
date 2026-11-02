use crate::{Color32, HdrVertexColor, TextureId, WHITE_UV, emath};
use emath::{Pos2, Rect, Rot2, TSTransform, Vec2};

/// The 2D vertex type.
///
/// Should be friendly to send to GPU as is.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg(any(not(feature = "unity"), feature = "_override_unity"))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Vertex {
    /// Logical pixel coordinates (points).
    /// (0,0) is the top left corner of the screen.
    pub pos: Pos2, // 64 bit

    /// Normalized texture coordinates.
    /// (0, 0) is the top left corner of the texture.
    /// (1, 1) is the bottom right corner of the texture.
    pub uv: Pos2, // 64 bit

    /// sRGBA with premultiplied alpha
    pub color: Color32, // 32 bit
}

impl Vertex {
    /// An untextured vertex
    #[inline]
    pub fn untextured(pos: Pos2, color: Color32) -> Self {
        Self {
            pos,
            uv: WHITE_UV,
            color,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg(all(feature = "unity", not(feature = "_override_unity")))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Vertex {
    /// Logical pixel coordinates (points).
    /// (0,0) is the top left corner of the screen.
    pub pos: Pos2, // 64 bit

    /// sRGBA with premultiplied alpha
    pub color: Color32, // 32 bit

    /// Normalized texture coordinates.
    /// (0, 0) is the top left corner of the texture.
    /// (1, 1) is the bottom right corner of the texture.
    pub uv: Pos2, // 64 bit
}

/// Textured triangles in two dimensions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Mesh {
    /// Draw as triangles (i.e. the length is always multiple of three).
    ///
    /// If you only support 16-bit indices you can use [`Mesh::split_to_u16`].
    ///
    /// egui is NOT consistent with what winding order it uses, so turn off backface culling.
    pub indices: Vec<u32>,

    /// The vertex data indexed by `indices`.
    pub vertices: Vec<Vertex>,

    /// The texture to use when drawing these triangles.
    pub texture_id: TextureId,

    /// Per-vertex HDR colors, or `None` for an all-SDR mesh.
    ///
    /// This is the *data plane* of the HDR vertex-color tier: `Some` means the
    /// vertices carry values that are not capped at 1.0, and the length equals
    /// `vertices.len()` (the invariant lives in [`Self::is_valid`]). The
    /// vertices' own [`Vertex::color`] then holds the legal SDR projection
    /// ([`HdrVertexColor::projected_color32`]) that SDR-only backends draw.
    ///
    /// `None` — the default, and every mesh egui's tessellator produces for
    /// SDR content — costs nothing and renders byte-identically to before the
    /// tier existed.
    ///
    /// Old serialized meshes load with `None` (the correct SDR semantics); a
    /// new mesh read by old code loses the HDR tier but still renders.
    #[cfg_attr(feature = "serde", serde(default))]
    pub hdr_colors: Option<Vec<HdrVertexColor>>,
    // TODO(emilk): bounding rectangle
}

impl Mesh {
    pub fn with_texture(texture_id: TextureId) -> Self {
        Self {
            texture_id,
            ..Default::default()
        }
    }

    /// Restore to default state, but without freeing memory.
    pub fn clear(&mut self) {
        self.indices.clear();
        self.vertices.clear();
        self.vertices = Default::default();
        self.hdr_colors = None;
    }

    /// Returns the amount of memory used by the vertices and indices.
    pub fn bytes_used(&self) -> usize {
        core::mem::size_of::<Self>()
            + self.vertices.len() * core::mem::size_of::<Vertex>()
            + self.indices.len() * core::mem::size_of::<u32>()
            + self.hdr_colors.as_ref().map_or(0, |colors| {
                colors.len() * core::mem::size_of::<HdrVertexColor>()
            })
    }

    /// Are all indices within the bounds of the contained vertices?
    pub fn is_valid(&self) -> bool {
        profiling::function_scope!();

        // The sidecar is per-vertex: a length mismatch is always a bug, not a
        // partial tier.
        if self
            .hdr_colors
            .as_ref()
            .is_some_and(|colors| colors.len() != self.vertices.len())
        {
            return false;
        }

        if let Ok(n) = u32::try_from(self.vertices.len()) {
            self.indices.iter().all(|&i| i < n)
        } else {
            false
        }
    }

    /// Does this mesh carry the HDR vertex-color tier?
    ///
    /// The predicate the tessellator's mesh-grouping key uses to keep HDR and
    /// SDR content in separate primitives.
    #[inline]
    #[must_use]
    pub fn is_hdr(&self) -> bool {
        self.hdr_colors.is_some()
    }

    /// Ensure this mesh carries per-vertex HDR colors, lifting the existing
    /// vertices' [`Color32`] exactly (per-component `/255`, well under half
    /// an 8-bit step) if it does not, and return the sidecar for appending.
    ///
    /// Idempotent: an already-HDR mesh is untouched and its sidecar returned.
    /// Promotion never loses information — an SDR color lifts into binary16
    /// within its precision — which is what makes
    /// [`Self::append`]/[`Self::append_ref`] able to merge the two tiers
    /// without dropping either side.
    pub fn promote_to_hdr(&mut self) -> &mut Vec<HdrVertexColor> {
        self.hdr_colors.get_or_insert_with(|| {
            self.vertices
                .iter()
                .map(|vertex| HdrVertexColor::from(vertex.color))
                .collect()
        })
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() && self.vertices.is_empty()
    }

    /// Iterate over the triangles of this mesh, returning vertex indices.
    pub fn triangles(&self) -> impl Iterator<Item = [u32; 3]> + '_ {
        self.indices
            .chunks_exact(3)
            .map(|chunk| [chunk[0], chunk[1], chunk[2]])
    }

    /// Calculate a bounding rectangle.
    pub fn calc_bounds(&self) -> Rect {
        let mut bounds = Rect::NOTHING;
        for v in &self.vertices {
            bounds.extend_with(v.pos);
        }
        bounds
    }

    /// Append all the indices and vertices of `other` to `self`.
    ///
    /// Panics when `other` mesh has a different texture.
    ///
    /// If either mesh carries HDR colors, the result does: an SDR side is
    /// promoted exactly ([`Self::promote_to_hdr`]). Merging never drops the
    /// HDR half and never clamps.
    pub fn append(&mut self, other: Self) {
        profiling::function_scope!();
        debug_assert!(other.is_valid(), "Other mesh is invalid");

        if self.is_empty() {
            *self = other;
        } else {
            self.append_ref(&other);
        }
    }

    /// Append all the indices and vertices of `other` to `self` without
    /// taking ownership.
    ///
    /// Panics when `other` mesh has a different texture.
    ///
    /// If `other` carries HDR colors and `self` does not, `self` is promoted;
    /// if `self` carries them and `other` does not, `other`'s colors are
    /// lifted into the extension iterator — `other` is never mutated, which
    /// `&Self` makes impossible anyway. See [`Self::append`] and
    /// [`Self::promote_to_hdr`].
    pub fn append_ref(&mut self, other: &Self) {
        debug_assert!(other.is_valid(), "Other mesh is invalid");

        if self.is_empty() {
            self.texture_id = other.texture_id;
        } else {
            assert_eq!(
                self.texture_id, other.texture_id,
                "Can't merge Mesh using different textures"
            );
        }

        if other.is_hdr() {
            self.promote_to_hdr();
        }

        if let Some(colors) = &mut self.hdr_colors {
            if let Some(other_colors) = &other.hdr_colors {
                colors.extend(other_colors.iter().copied());
            } else {
                // Lift into the extension iterator: no temporary `Vec`, and
                // `other` stays untouched.
                colors.extend(other.vertices.iter().map(|v| HdrVertexColor::from(v.color)));
            }
        }

        let index_offset = self.vertices.len() as u32;
        self.indices
            .extend(other.indices.iter().map(|index| index + index_offset));
        self.vertices.extend(other.vertices.iter());
    }

    /// Add a colored vertex.
    ///
    /// Panics when the mesh has assigned a texture.
    #[inline(always)]
    pub fn colored_vertex(&mut self, pos: Pos2, color: Color32) {
        debug_assert!(
            self.texture_id == TextureId::default(),
            "Mesh has an assigned texture"
        );
        self.vertices.push(Vertex::untextured(pos, color));
    }

    /// Add a triangle.
    #[inline(always)]
    pub fn add_triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Make room for this many additional triangles (will reserve 3x as many indices).
    /// See also `reserve_vertices`.
    #[inline(always)]
    pub fn reserve_triangles(&mut self, additional_triangles: usize) {
        self.indices.reserve(3 * additional_triangles);
    }

    /// Make room for this many additional vertices.
    /// See also `reserve_triangles`.
    #[inline(always)]
    pub fn reserve_vertices(&mut self, additional: usize) {
        self.vertices.reserve(additional);
    }

    /// Rectangle with a texture and color.
    #[inline(always)]
    pub fn add_rect_with_uv(&mut self, rect: Rect, uv: Rect, color: Color32) {
        #![expect(clippy::identity_op)]
        let idx = self.vertices.len() as u32;
        self.indices
            .extend_from_slice(&[idx + 0, idx + 1, idx + 2, idx + 2, idx + 1, idx + 3]);

        self.vertices.extend_from_slice(&[
            Vertex {
                pos: rect.left_top(),
                uv: uv.left_top(),
                color,
            },
            Vertex {
                pos: rect.right_top(),
                uv: uv.right_top(),
                color,
            },
            Vertex {
                pos: rect.left_bottom(),
                uv: uv.left_bottom(),
                color,
            },
            Vertex {
                pos: rect.right_bottom(),
                uv: uv.right_bottom(),
                color,
            },
        ]);
    }

    /// Uniformly colored rectangle.
    #[inline(always)]
    pub fn add_colored_rect(&mut self, rect: Rect, color: Color32) {
        debug_assert!(
            self.texture_id == TextureId::default(),
            "Mesh has an assigned texture"
        );
        self.add_rect_with_uv(rect, [WHITE_UV, WHITE_UV].into(), color);
    }

    /// Add a vertex with an HDR color.
    ///
    /// Promotes the mesh first, so vertices added before this call keep their
    /// colors (lifted exactly). The [`Vertex::color`] twin is the legal SDR
    /// projection, which is what an SDR-only backend draws.
    ///
    /// Panics when the mesh has assigned a texture.
    #[inline]
    pub fn colored_vertex_hdr(&mut self, pos: Pos2, color: HdrVertexColor) {
        debug_assert!(
            self.texture_id == TextureId::default(),
            "Mesh has an assigned texture"
        );
        self.promote_to_hdr().push(color);
        self.vertices
            .push(Vertex::untextured(pos, color.projected_color32()));
    }

    /// Rectangle with a texture and an HDR color.
    ///
    /// The four vertices get the legal SDR projection as their
    /// [`Vertex::color`], and `color` in the sidecar.
    #[inline]
    pub fn add_rect_with_uv_hdr(&mut self, rect: Rect, uv: Rect, color: HdrVertexColor) {
        self.promote_to_hdr().extend([color; 4]);
        self.add_rect_with_uv(rect, uv, color.projected_color32());
    }

    /// Uniformly HDR-colored rectangle.
    #[inline]
    pub fn add_colored_rect_hdr(&mut self, rect: Rect, color: HdrVertexColor) {
        debug_assert!(
            self.texture_id == TextureId::default(),
            "Mesh has an assigned texture"
        );
        self.add_rect_with_uv_hdr(rect, [WHITE_UV, WHITE_UV].into(), color);
    }

    /// This is for platforms that only support 16-bit index buffers.
    ///
    /// Splits this mesh into many smaller meshes (if needed)
    /// where the smaller meshes have 16-bit indices.
    pub fn split_to_u16(self) -> Vec<Mesh16> {
        debug_assert!(self.is_valid(), "Mesh is invalid");

        const MAX_SIZE: u32 = u16::MAX as u32;

        if self.vertices.len() <= MAX_SIZE as usize {
            // Common-case optimization:
            return vec![Mesh16 {
                indices: self.indices.iter().map(|&i| i as u16).collect(),
                vertices: self.vertices,
                texture_id: self.texture_id,
                hdr_colors: self.hdr_colors,
            }];
        }

        let mut output = vec![];
        let mut index_cursor = 0;

        while index_cursor < self.indices.len() {
            let span_start = index_cursor;
            let mut min_vindex = self.indices[index_cursor];
            let mut max_vindex = self.indices[index_cursor];

            while index_cursor < self.indices.len() {
                let (mut new_min, mut new_max) = (min_vindex, max_vindex);
                for i in 0..3 {
                    let idx = self.indices[index_cursor + i];
                    new_min = new_min.min(idx);
                    new_max = new_max.max(idx);
                }

                let new_span_size = new_max - new_min + 1; // plus one, because it is an inclusive range
                if new_span_size <= MAX_SIZE {
                    // Triangle fits
                    min_vindex = new_min;
                    max_vindex = new_max;
                    index_cursor += 3;
                } else {
                    break;
                }
            }

            assert!(
                index_cursor > span_start,
                "One triangle spanned more than {MAX_SIZE} vertices"
            );

            let mesh = Mesh16 {
                indices: self.indices[span_start..index_cursor]
                    .iter()
                    .map(|vi| {
                        #[expect(clippy::unwrap_used)]
                        {
                            u16::try_from(vi - min_vindex).unwrap()
                        }
                    })
                    .collect(),
                vertices: self.vertices[(min_vindex as usize)..=(max_vindex as usize)].to_vec(),
                texture_id: self.texture_id,
                hdr_colors: self
                    .hdr_colors
                    .as_ref()
                    .map(|colors| colors[(min_vindex as usize)..=(max_vindex as usize)].to_vec()),
            };
            debug_assert!(mesh.is_valid(), "Mesh is invalid");
            output.push(mesh);
        }
        output
    }

    /// Translate location by this much, in-place
    pub fn translate(&mut self, delta: Vec2) {
        for v in &mut self.vertices {
            v.pos += delta;
        }
    }

    /// Transform the mesh in-place with the given transform.
    pub fn transform(&mut self, transform: TSTransform) {
        for v in &mut self.vertices {
            v.pos = transform * v.pos;
        }
    }

    /// Rotate by some angle about an origin, in-place.
    ///
    /// Origin is a position in screen space.
    pub fn rotate(&mut self, rot: Rot2, origin: Pos2) {
        for v in &mut self.vertices {
            v.pos = origin + rot * (v.pos - origin);
        }
    }
}

// ----------------------------------------------------------------------------

/// A version of [`Mesh`] that uses 16-bit indices.
///
/// This is produced by [`Mesh::split_to_u16`] and is meant to be used for legacy render backends.
pub struct Mesh16 {
    /// Draw as triangles (i.e. the length is always multiple of three).
    ///
    /// egui is NOT consistent with what winding order it uses, so turn off backface culling.
    pub indices: Vec<u16>,

    /// The vertex data indexed by `indices`.
    pub vertices: Vec<Vertex>,

    /// The texture to use when drawing these triangles.
    pub texture_id: TextureId,

    /// Per-vertex HDR colors, or `None` for an all-SDR mesh.
    ///
    /// Mirrors [`Mesh::hdr_colors`] field-for-field.
    pub hdr_colors: Option<Vec<HdrVertexColor>>,
}

impl Mesh16 {
    /// Are all indices within the bounds of the contained vertices?
    pub fn is_valid(&self) -> bool {
        if self
            .hdr_colors
            .as_ref()
            .is_some_and(|colors| colors.len() != self.vertices.len())
        {
            return false;
        }

        if let Ok(n) = u16::try_from(self.vertices.len()) {
            self.indices.iter().all(|&i| i < n)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emath::pos2;

    fn rect() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0))
    }

    fn sdr_mesh() -> Mesh {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(rect(), Color32::from_rgb(10, 20, 30));
        mesh
    }

    fn hdr_mesh() -> Mesh {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect_hdr(
            rect(),
            HdrVertexColor::from_rgba_premultiplied([2.0, 1.0, 0.5, 1.0]),
        );
        mesh
    }

    #[test]
    fn sidecar_length_is_an_invariant() {
        let mut mesh = sdr_mesh();
        assert!(mesh.is_valid());
        assert!(!mesh.is_hdr());

        // A mismatched sidecar is invalid, which is the hook every mutation
        // path already calls.
        mesh.hdr_colors = Some(vec![HdrVertexColor::WHITE; mesh.vertices.len() + 1]);
        assert!(!mesh.is_valid());
        mesh.hdr_colors = Some(vec![HdrVertexColor::WHITE; mesh.vertices.len()]);
        assert!(mesh.is_valid());
        assert!(mesh.is_hdr());
    }

    #[test]
    fn promote_is_idempotent_and_exact() {
        let mut mesh = sdr_mesh();
        let colors = mesh.promote_to_hdr().clone();
        assert_eq!(mesh.hdr_colors.as_deref(), Some(colors.as_slice()));

        // Every lifted value is the exact `/255` rescale of its vertex color.
        for (color, vertex) in core::iter::zip(&colors, &mesh.vertices) {
            assert_eq!(*color, HdrVertexColor::from(vertex.color));
        }

        let vertex_count = mesh.vertices.len();
        let twice = mesh.promote_to_hdr();
        assert_eq!(twice.len(), vertex_count);
        assert_eq!(twice, &colors, "promotion must not change an HDR mesh");
    }

    #[test]
    fn append_promotes_whichever_side_is_sdr() {
        // HDR receiver + SDR argument: the sidecar is lifted in.
        let mut receiver = hdr_mesh();
        receiver.append(sdr_mesh());
        assert!(receiver.is_valid());
        assert_eq!(receiver.hdr_colors.as_ref().unwrap().len(), 8);
        assert_eq!(
            receiver.hdr_colors.as_ref().unwrap()[4..],
            [HdrVertexColor::from(Color32::from_rgb(10, 20, 30)); 4]
        );

        // SDR receiver + HDR argument: `append` owns the other side, so it can
        // promote the receiver (and would be allowed to promote other too).
        let mut receiver = sdr_mesh();
        receiver.append(hdr_mesh());
        assert!(receiver.is_valid());
        assert_eq!(receiver.hdr_colors.as_ref().unwrap().len(), 8);

        // Two SDR meshes stay SDR: no memory is invented.
        let mut receiver = sdr_mesh();
        receiver.append(sdr_mesh());
        assert!(!receiver.is_hdr());
    }

    #[test]
    fn append_ref_never_mutates_other() {
        let mut receiver = hdr_mesh();
        let other = sdr_mesh();
        let before = other.clone();
        receiver.append_ref(&other);
        assert_eq!(other, before, "append_ref must not promote its argument");
        assert_eq!(receiver.hdr_colors.as_ref().unwrap().len(), 8);

        // The reverse direction promotes the receiver.
        let mut receiver = sdr_mesh();
        let other = hdr_mesh();
        receiver.append_ref(&other);
        assert!(receiver.is_valid());
        assert_eq!(receiver.hdr_colors.as_ref().unwrap().len(), 8);
    }

    #[test]
    fn clear_drops_the_tier() {
        let mut mesh = hdr_mesh();
        mesh.clear();
        assert!(!mesh.is_hdr());
        assert!(!mesh.is_valid() || mesh.vertices.is_empty());
        assert_eq!(mesh.bytes_used(), core::mem::size_of::<Mesh>());
    }

    #[test]
    fn bytes_used_counts_the_sidecar() {
        let sdr = sdr_mesh();
        let hdr = hdr_mesh();
        assert_eq!(
            hdr.bytes_used() - sdr.bytes_used(),
            4 * core::mem::size_of::<HdrVertexColor>()
        );
    }

    /// §5 consequence 6: the sidecar may add a field but must not push either
    /// mesh over `PathShape`'s 56 B plus the shared `Mesh` header — the
    /// measured 88 B is the ceiling.
    #[test]
    fn mesh_size_is_unchanged() {
        assert_eq!(
            core::mem::size_of::<Mesh>(),
            88,
            "Mesh grew: the sidecar is the only added field, and it is an Option<Vec<_>>"
        );
        assert_eq!(
            core::mem::size_of::<Mesh16>(),
            88,
            "Mesh16 must stay the same size as Mesh"
        );
    }

    #[test]
    fn split_to_u16_carries_the_tier_on_both_paths() {
        // Fast path: under the u16 limit, the sidecar moves over whole.
        let mesh = hdr_mesh();
        let expected_colors = mesh.hdr_colors.clone().unwrap();
        let split = mesh.split_to_u16();
        assert_eq!(split.len(), 1);
        assert_eq!(
            split[0].hdr_colors.as_deref(),
            Some(expected_colors.as_slice())
        );
        assert!(split[0].is_valid());

        // Split path: two triangles far apart force two `Mesh16`s, and each
        // carries the sidecar slice matching its vertex slice.
        let n = u16::MAX as usize + 1000;
        let mesh = Mesh {
            vertices: (0..n)
                .map(|i| Vertex::untextured(pos2(i as f32, 0.0), Color32::WHITE))
                .collect(),
            indices: vec![
                0,
                30_000,
                60_000, //
                (n - 3) as u32,
                (n - 2) as u32,
                (n - 1) as u32,
            ],
            // Keep the marker exactly representable in binary16.
            hdr_colors: Some(
                (0..n)
                    .map(|i| {
                        HdrVertexColor::from_rgba_premultiplied([(i % 1024) as f32, 0.0, 0.0, 1.0])
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        assert!(mesh.is_valid());

        let split = mesh.split_to_u16();
        assert_eq!(split.len(), 2);
        for mesh16 in &split {
            assert!(mesh16.is_valid());
            let colors = mesh16.hdr_colors.as_ref().expect("tier carried");
            assert_eq!(colors.len(), mesh16.vertices.len());
            for (color, vertex) in core::iter::zip(colors, &mesh16.vertices) {
                assert_eq!(color.to_array()[0], (vertex.pos.x as usize % 1024) as f32);
            }
        }
        assert_eq!(split[0].hdr_colors.as_ref().unwrap().len(), 60_001);
        assert_eq!(split[1].hdr_colors.as_ref().unwrap().len(), 3);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trips_with_and_without_the_sidecar() {
        // SDR: the field is `None`, and an old payload without the key still
        // loads (the correct SDR semantics).
        let sdr = sdr_mesh();
        let json = serde_json::to_string(&sdr).expect("serialize SDR mesh");
        let back: Mesh = serde_json::from_str(&json).expect("deserialize SDR mesh");
        assert_eq!(back, sdr);
        assert!(!back.is_hdr());

        let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse");
        let object = value.as_object_mut().expect("mesh is an object");
        object.remove("hdr_colors");
        let old: Mesh = serde_json::from_value(value).expect("old payload must load");
        assert_eq!(old, sdr, "an old payload loads as the same SDR mesh");
        assert!(!old.is_hdr());

        // HDR: the sidecar survives byte-for-byte.
        let hdr = hdr_mesh();
        let json = serde_json::to_string(&hdr).expect("serialize HDR mesh");
        let back: Mesh = serde_json::from_str(&json).expect("deserialize HDR mesh");
        assert_eq!(back, hdr);
        assert!(back.is_hdr());
        assert_eq!(back.hdr_colors, hdr.hdr_colors);
    }

    #[test]
    fn mesh16_is_valid_checks_the_sidecar() {
        let mut mesh = hdr_mesh().split_to_u16().remove(0);
        assert!(mesh.is_valid());
        mesh.hdr_colors = Some(vec![]);
        assert!(!mesh.is_valid());
    }
}
