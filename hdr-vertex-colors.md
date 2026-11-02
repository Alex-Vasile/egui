# HDR vertex colors — extension spec

**Status:** spec (no code) on `hdr-pr6-float-vertex-colors`, branched off
`hdr-side-items-test-app` — the top of the HDR stack (`hdr_enablement` → … → `hdr-pr5-hdr-widgets`
→ `hdr-side-items-test-app`).

**Relation to the main plan:** this document *extends* `hdr-egui-upstream.md`. That plan's §11
parked the vertex-color tier and §6 listed exactly what its absence costs; both are carried here in
full (§1), re-grounded against the tree (§4), and turned into a design space (§6), an API surface
(§8), and a PR ladder (§14). Nothing here changes a decision already shipped in PRs 1–5.

**Anchors:** re-verified against the working tree at `714f2a0cf`
(`hdr-side-items-test-app`, 2026-11-02), then re-checked in a second (house-style) pass on the same
day — the §4 line numbers and the §5 measurements below are the second pass's (Appendix C.3). Line
numbers are as of this branch.

---

## 0. Summary

Vertex-borne color is capped at 1.0 because `Vertex::color` is a `Color32`. Raising that ceiling is
the only way to color *geometry* above SDR white: strokes, text, gradients across arbitrary shapes.

Three separate questions, deliberately separated:

| Question | Answer in this spec | § |
|---|---|---|
| Where does the second color dtype live? | Per-mesh color sidecar, not in `Vertex` | 6 |
| How is it selected at draw time? | Pipeline variant keyed on a per-mesh tier bit | 6.3, 11 |
| What expresses it in the API? | `Mesh` (base tier) · `Shape::Hdr` recolor wrapper for every other shape, strokes and polylines included · `TextShape::hdr_color` for glyphs | 8 |
| What is the contract of the stored value? | The §4 contract, unchanged: gamma-encoded in the paint space, premultiplied in the encoded components, unclamped, odd-symmetric for negatives | 3 |
| Does it touch the default path? | No. `Vertex` stays 20 B, `Shape` stays 64 B, SDR staging is byte-identical | 5, 6.6 |

**Headline design decision (§6.7):** the ripple §11 feared — `Eq` cascade, serde repr of the color
field, every `Vertex` literal site, the `unity` layout, glow's stride — belongs to designs that put
the dtype *inside* `Vertex`. A per-mesh sidecar keeps `Vertex`, `Mesh`'s `Eq`, serde of vertices,
and glow's vertex path untouched, and costs nothing for the all-SDR default path. Accept the
sidecar's one new invariant (`hdr_colors.len() == vertices.len()`) and a second pipeline variant
instead.

Investigated 2026-11-02 (§6.6): a channel-dtype abstraction cannot collapse the two tiers — pipeline
count is a function of the vertex buffer layout, not of the Rust type — so B's two pipeline variants
are inherent, not incidental. Two of the stack's assumptions were corrected by the same
investigation: no `enable f16` is needed (WGSL receives `Float16x4` as `vec4<f32>`), and f16 vertex
fetch is core on every backend including the web.

Re-verified against the tree 2026-11-02 (style/measurement pass, §5 and Appendix C.3): three more
claims were corrected — `ColorMode` cannot take an HDR variant at any size (§5), the renderer has no
draw-record/batching layer to key on a tier (§11), and glow's warn-once convention is keyed on
`TextureId`, which a mesh does not have (§11). Each correction is recorded where it bites and
summarised in Appendix C.3; none of them weakens the three capabilities, the §3 contract, or the
SDR-is-untouched rule.

---

## 1. What was deferred (carried from `hdr-egui-upstream.md`)

### 1.1 The ladder entry (§11, verbatim intent)

`Vertex` color `u32` → `f16×4` ripples through every mesh consumer: egui-wgpu pipelines, egui_glow
(or explicit scoping-out), kittest, capture, mesh serde + the `Eq` cascade (`Vertex → Mesh → Shape`,
both cfg variants including `unity` — the packing trick must work in both field orders), and
`Context::tessellate` consumers. The layout fight (stride 20→24 vs dual-format meshes vs sidecar
buffers) is the sticky review thread.

> **Correction to the carry (2026-11-02):** the `Eq` cascade stops at `Mesh`. `Shape` derives
> `Clone, Debug, PartialEq` only (`epaint/src/shapes/shape.rs:25-27`) and cannot be `Eq`, because
> `ColorMode::UV` holds a closure whose manual `PartialEq` returns `false` for `UV == UV`
> (`epaint/src/color.rs:44-57`). Nothing in this spec can make `Shape` `Eq` fail; only `Mesh`'s
> derive is at stake.

Pre-E type decisions recorded then: the `Eq` drop, the serde repr of the color field (`Color32`
serializes as its own repr; `f16×4` needs a chosen repr via `half/serde`), and `half`'s `bytemuck`
feature as epaint's direct surface.

Trigger to revisit: animated HDR vector content (scopes at frame rate), HDR text as a first-class
need. The intermediate-linear render pass (§7 of the main plan) is the alternative for photometric
blending *without* the vertex-color tier.

### 1.2 What PR 5 cannot do (§6 of the main plan)

PR 5's float *textures* give HDR color as a function of position inside a rectangle. Float *vertex
colors* give HDR color as a property of geometry. Still impossible at PR 5:

1. **Stroked paths** — stroke color lives in vertices: no antialiased rotated geometry, no
   joins/caps, no polylines, no plot traces. (Rotated HDR *rects* work via `Image::rotate` —
   unantialiased.)
2. **Text** — glyph color is vertex-borne, capped at 1.0. The only stopgap is per-widget
   float-texture caching (a galley-keyed cache of atlas coverage × HDR color, ~16 KB fp16 for a
   readout label). That `hdr_text` helper is demo-app-local, never an upstream deliverable; it
   either validates deferring the tier or becomes the evidence it is needed.
3. **Gradients above 1.0 across arbitrary shapes** — per-vertex interpolation is the only masking
   mechanism the tessellator offers; textures fake axis-aligned only.

Who needs the tier: video scopes at frame rate, design tools with HDR gradient meshes, plot-heavy
UIs over HDR canvases, HDR text as a first-class need.

### 1.3 Constraints inherited from shipped work

- **Content contract (§4).** Float colors are gamma-encoded in the declared primaries,
  premultiplied in the encoded components, odd-symmetric for negatives, 1.0 = SDR reference white.
  `FloatImage`'s rustdoc (`epaint/src/image.rs:440-479`) is the canonical statement. Vertex colors
  adopt it verbatim (§3).
- **Primaries mechanism (§5).** The mechanism is primaries-generic: a primaries = enum variant +
  pairing row + conversion matrix, one PR, no design change. Vertex colors are **un**tagged content
  (§10), so they ride the surface's granted primaries the way `Color32` chrome does.
- **Chrome-shift numbers (§5).** A P3-referred surface reinterprets sRGB-referred bytes: neutrals
  shift 0.0, saturated accents 2.2–6.8 CIEDE2000. Applies to vertex colors identically, and is
  amplified by HDR values.
- **Pipeline cache (§2/PR 1).** Format-keyed and already extended by PR 4 to
  `(format, OutputEncoding)`; adding a third key component is mechanical, not a redesign.
- **Testing infrastructure.** kittest fp16 readback exists (`egui_kittest/src/texture_to_image.rs:16-30`),
  `Rgba16Float` capture decode exists (`egui-wgpu/src/capture.rs:91,207-215,271-320`), MSAA-on-fp16
  is covered by PR 3's tests, and PNG dumps are cICP-tagged.
- **Binding rule.** Every PR ships consumed, testable functionality — nothing dead, nothing
  untestable at merge.

---

## 2. The gap, concretely

| Scenario | Today | After this tier |
|---|---|---|
| Plot trace over an HDR canvas at 4× white | clamped to 1.0 (or faked with a texture strip) | antialiased polyline at 4.0 |
| Axis label on the same canvas | clamped | glyph coverage × HDR color |
| Gradient across a rounded shape, 1.0 → 8.0 | axis-aligned texture fake only | per-vertex interpolation |
| Scope trace at frame rate | per-frame texture upload per shape | one mesh, no upload |
| SDR chrome, every app | unchanged | **unchanged** (byte-identical) |

---

## 3. Contract for vertex-borne color

Adopt §4 exactly, so a float-textured swatch and an HDR vertex at the same value are
indistinguishable:

- **Stored values are encoded** (gamma curve of the paint space), never linear. The shader's
  existing multiply `in.color * tex_gamma` (`egui.wgsl:180-200`) stays valid at any magnitude.
- **Premultiplied in the encoded components**: `stored = a × enc(L)`, `Color32`'s convention.
- **Unclamped above 1.0**; 1.0 = SDR reference white (identical on macOS EDR, where the surface is
  display-referred, and on scRGB, where PR 4 scales by `sdr_white_nits/80`).
- **Odd-symmetric for negatives** — preserved through the linear (scRGB) fragment entry point
  because PR 2a made `linear_from_gamma` odd-symmetric.
- **Alpha is coverage, always `0..=1`.** Feathering multiplies all four components, exactly
  `mul_color`'s `gamma_multiply` (`epaint/src/tessellator.rs:1408-1412`), which keeps
  premultiplication intact at any magnitude.
- **Blending stays `One/OneMinusSrcAlpha` in encoded space** — egui's house style
  (`egui/src/lib.rs:147`); no change, and no new decode/encode round trip in the gamma path.

Precision: `f16` has an 11-bit mantissa; for values in `[0,1]` that is strictly finer than `u8`
(worst-case relative error ~5e-4 vs 4e-3, absolute floor 6e-8 vs 4e-3). Lifting an existing
`Color32` into the tier is therefore visually exact, not merely close (§8.1 has the assertion).

Range: `f16` tops out at 65504; the sRGB transfer of a 10⁶ linear value is ≈ 316. No realistic
content approaches the ceiling.

---

## 4. Code map (verified at `714f2a0cf`)

**The vertex and the mesh**

- `epaint/src/mesh.rs:8-24` — `Vertex` (non-unity): `pos`, `uv`, `color: Color32`; derives
  `Clone, Copy, Debug, Default, PartialEq, Eq`, serde, `bytemuck::Pod`; `#[repr(C)]`.
- `epaint/src/mesh.rs:39-55` — the `unity` cfg variant, same fields in a different order
  (`features unity` / `_override_unity`, `epaint/Cargo.toml:76-81`).
- `epaint/src/mesh.rs:26-33` — `Vertex::untextured`; `:165-175` — `Mesh::colored_vertex`.
- `epaint/src/mesh.rs:58-60` — `Mesh` derives `PartialEq, Eq` (must survive any field change);
  `:352-360` `Mesh16::is_valid` (no derives at all on `Mesh16`, so no serde obligation there).
- `epaint/src/mesh.rs:85-96` — `clear` (must also clear a sidecar) and `bytes_used`
  (`size_of::<Vertex>()` arithmetic); `:99-108` `is_valid` — the existing `debug_assert` hook every
  mutation path already calls.
- `epaint/src/mesh.rs:132-165` — `append` / `append_ref`: where the tier-mixing rule lives.
- `epaint/src/mesh.rs:199-227` — `add_rect_with_uv` (four literal `Vertex` constructions).
- `epaint/src/mesh.rs:337-348` — `Mesh16` (the u16-index split; mirrors `Mesh` field-for-field).
- `epaint/src/mesh.rs:242-303` — `split_to_u16`.

**Every `Vertex` literal site** (all would need the new field under a design that touches `Vertex`):
`mesh.rs:206,211,216,221` · `tessellator.rs:988,995,1008,2356` · `text_layout.rs:1311,1316,1321,1326`
· `egui-wgpu/src/renderer.rs:3334` (test).

**Color plumbing that must learn about HDR values**

- `epaint/src/color.rs:8-20` — `ColorMode { Solid(Color32), UV(Arc<dyn Fn(Rect, Pos2) -> Color32>) }`,
  16 B: the path-color abstraction. **Not** the seam for HDR path color: a third variant measures
  24 B and the growth propagates to `Shape` (§5, §8.3).
- `epaint/src/stroke.rs:113-160` — `PathStroke { width, color: ColorMode }`; `:198` `is_empty`.
- `epaint/src/stroke.rs:14-16` — `Stroke { width: f32, color: Color32 }`, 8 B — **cannot** grow
  without tripping the `Shape` size guard (§5). `Shape::LineSegment { points, stroke }` therefore
  stays SDR; HDR line segments go through `Mesh`.
- `epaint/src/tessellator.rs:1072-1130` — stroke tessellation; `get_color(&ColorMode, Pos2)`.
- `epaint/src/tessellator.rs:1408-1412` — `mul_color` → `gamma_multiply`.
- `epaint/src/tessellator.rs:854-857` — opacity folded into existing vertices.
- `epaint/src/tessellator.rs:2328-2358` — text vertex propagation: `override_text_color`,
  `PLACEHOLDER` fallback, color-glyph anti-tint, opacity factor.
- `epaint/src/shape_transform.rs:88-111,132-147` — color adjustment for placed rows and
  `ColorMode` callbacks (must gain the HDR arm). `:114-120` destructures
  `Mesh { indices: _, vertices, texture_id: _ }` **exhaustively** — it stops compiling the moment
  `Mesh` gains a field, which is the compiler doing the work of finding every such site.
- `epaint/src/shapes/shape.rs:437-445` — `Shape::texture_id()`: an `if let` chain, **not** an
  exhaustive match, so the compiler will *not* point at it when `Shape` gains a variant. It drives
  the tessellator's mesh-grouping (`tessellator.rs:1490-1535`), so a variant that wraps geometry
  must delegate here or the tessellator appends across textures and panics on
  `tessellator.rs:952-956`'s `assert_eq!`.
- `epaint/src/shapes/shape.rs:403-433` — `visual_bounding_rect` (exhaustive, compiler-visible) and
  `transform` (`:467`).
- `epaint/src/stats.rs:192-230` — `PaintStats::add` (exhaustive; a new `Shape` variant needs an
  arm), `:160-176` its fields, `:232-245` `with_clipped_primitives`. `AllocInfo` (`:13-19`) is a
  bare four-field aggregator and is *not* where a counter goes: the sidecar is counted by extending
  `AllocInfo::from_mesh` (`:93-95`) — and only if it deserves its own row, by a new `PaintStats`
  field. `egui/src/introspection.rs:78-120` destructures `PaintStats` exhaustively and renders one
  row per field, so a new field also gains its row there.
- `epaint/src/text/text_layout.rs:1284-1330` — glyph vertex emission.
- `epaint/src/shapes/rect_shape.rs:24,30,56` — `fill: Color32`, `stroke: Stroke`, `brush`.
- `epaint/src/shapes/band_shape.rs:59-71`, `shapes/shape.rs:27-78` — shape payloads.
- `egui/src/visuals.rs:87-89` — text color override writes `vertex.color` directly.
- `egui/src/text_selection/label_text_selection.rs:294-298` — selection clears `vertex.color`.

**GPU side**

- `egui-wgpu/src/renderer.rs:597-624` — `ensure_pipeline`: one pipeline per `(format, encoding)`,
  created eagerly from `update_buffers` (`:1262`), so `render()` can only *look up*.
- `egui-wgpu/src/renderer.rs:643-661` — `render()` looks the one pipeline up front and
  `.expect(…)`s it; `:686` sets it once per pass and only again after a paint callback.
- `egui-wgpu/src/renderer.rs:694-710` — the zero-size-clip skip: advances the index *and* vertex
  slice iterators by hand; `:714-735` the per-mesh draw (`set_vertex_buffer(0, …)` at `:728`,
  `draw_indexed` at `:734`).
- `egui-wgpu/src/renderer.rs:1761-1771` — the vertex layout:
  `entry_point: Some("vs_main")`, `array_stride: 5 * 4`,
  `vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32]`.
- `egui-wgpu/src/renderer.rs:1642-1690` — `OutputEncoding::of(format, color_space)`, the total table;
  `:1630-1632` `is_float_target`.
- `egui-wgpu/src/renderer.rs:1796-1805` — entry-point selection
  (`fs_main_linear_framebuffer` vs `fs_main_gamma_framebuffer`).
- `egui-wgpu/src/renderer.rs:551-552,1373-1410` — vertex staging: one contiguous buffer,
  `bytemuck::cast_slice(&mesh.vertices)`, per-mesh byte ranges (the `SlicedBuffer` at `:207-211`).
- `egui-wgpu/src/egui.wgsl:1-5` — `VertexOutput{tex_coord, color (gamma), position}`;
  `:60-70` `unpack_color`; `:96-110` `vs_main`; `:180-215` the two fragment entry points
  (`DITHER_CAPABLE` is an existing spliced constant — the precedent for compile-time *branching*;
  note the tier deliberately does **not** follow it, see the pipeline-matrix bullet below).
- `egui-wgpu/src/egui_transfer.wgsl` — transfer fns shared by production and the round-trip test.
- `egui-wgpu/src/renderer.rs:1452-1570` — `PairingRejection` for tagged float content (textures).
- `egui-wgpu/src/surface_policy.rs:328-380` — pairing walk; `is_u8` `:348`, `is_f16` `:361`.
- `egui_glow/src/painter.rs:222-252` — VAO layout: `a_srgba` = 4 × `UNSIGNED_BYTE` at
  `offset_of!(Vertex, color)`, stride `size_of::<Vertex>()`; `:480-486` raw `cast_slice` upload.
- `egui_glow/src/shader/vertex.glsl:24-36` — `v_rgba_in_gamma = a_srgba / 255.0`.
- `egui_glow/src/shader/fragment.glsl:56-70` — gamma-space multiply + dither.

**Testing / capture**

- `egui_kittest/src/texture_to_image.rs:16-30` — `Rgba16Float` readback returning floats.
- `egui-wgpu/src/capture.rs:91,207-215,271-320` — per-format texel size, fp16 decode, u8-path
  refusal to guess.
- `egui_kittest/src/wgpu.rs:92-101,158-186` — `with_render_options` (format override, MSAA samples).
- `epaint/src/stats.rs:93-95,192-230` — vertex byte accounting (`AllocInfo`, `PaintStats::add`).

**Public HDR surface that must not fragment**

- `epaint/src/image.rs:427-436` — `Primaries { Srgb, DisplayP3 }`; `:470-479` `FloatImage`.
- `egui/src/widgets/color_picker.rs:860-900` — `HdrColor { rgb: [f32;3] linear, exposure_stops }`.
- `egui/src/ui.rs:2263-2280` — `color_edit_button_hdr`, `swatch_hdr`.

---

## 5. Sizing budget (measured on this branch)

`cargo run -p epaint --example size_probe` (throwaway probe, removed) on aarch64 macOS. Re-run
2026-11-02 for the *proposed* layouts as well as the shipped ones — the proposed columns are the
ones that decide §8, and one early claim did not survive the re-run (consequence 3 below).

| Type | Bytes | Headroom |
|---|---|---|
| `Vertex` | 20 | — |
| `Mesh` / `Mesh16` | 64 / 64 | `Mesh` is `Arc`-ed inside `Shape`; growth is cheap |
| `Shape` | **64** | **has a hard guard** (`shapes/shape.rs:80-91`) |
| `PathShape` | 56 | inside a 64 B `Shape`: effectively none (the tag rides the remainder) |
| `RectShape` | 56 | effectively none |
| `TextShape` | 48 | 8 B — exactly one `Option<HdrVertexColor>` (§8.6) |
| `BandShape` | 48 | ~8 B |
| `CircleShape` | 24 | generous |
| `PathStroke` | 24 | none (feeds `PathShape`) |
| `Stroke` | 8 | none (feeds `Shape::LineSegment`) |
| `ColorMode` | 16 | **none** — any third variant measures 24 B (see "Correction" below) |

Proposed types, measured the same way:

| Proposed | Bytes | Effect on `Shape` |
|---|---|---|
| `HdrVertexColor` (8, `[u16; 4]`) | 8 | — |
| `Option<HdrVertexColor>` | 10 | `TextShape` 48 → 56; `Shape` unchanged (payload ceiling is `PathShape`'s 56) |
| `Shape::Hdr { inner: Arc<Shape>, color: HdrColorMode }` payload | 24 | `Shape` unchanged (24 ≤ 56) |
| `Mesh` + `Option<Vec<HdrVertexColor>>` / `Option<Box<[… ]>>` | 88 / 80 | unchanged (`Arc`-ed) |
| `ColorMode` with one or two extra variants | **24** | `PathStroke` 32, `PathShape` 64 → **`Shape` grows** |

Consequences, decided here rather than discovered in review:

1. **`Vertex` must not grow by 8 B "for free".** A dual field is `pos(8) + uv(8) + color(4) +
   hdr(8) = 28 B` (+40% on every vertex of every app). See §6.
2. **`PathShape`/`RectShape` fills cannot grow.** HDR *fills* must ride the mesh tier or a new
   `Shape` variant, never a new field on these structs.
3. **`ColorMode` cannot grow — corrected 2026-11-02.** The earlier entry here said an 8-byte
   payload fits "in the existing fat-pointer variant"; it does not. Measured: the *two-variant*
   enum is 16 B because `Arc<dyn Fn(..)>`'s non-null niche carries the discriminant, and the niche
   is exhausted at three variants — `{Solid, HdrSolid, UV}` and `{Solid, Hdr(payload), UV}` both
   measure **24 B** (the payload type is irrelevant). Verified independently with a standalone
   `rustc` probe: 2 variants → 16 B, 3 variants → 24 B, and adding a fourth does not grow it
   further. The *reason* is rustc's layout choice rather than a property of the enum, so treat
   `ColorMode` as **full at 16 B** and do not build a design on a future re-packing. 24 B `ColorMode` → `PathStroke` 32 → `PathShape` 64 → `Shape`
   past its guard. So HDR path colour does **not** ride `ColorMode`; it rides the `Shape::Hdr`
   wrapper, whose 24 B payload fits under `PathShape`'s 56 B ceiling (§8.3). This is a measured
   reversal of the §5-era assumption, not a preference.
4. **`TextShape` has room for exactly one `Option<HdrVertexColor>`** (48 → 56, measured), and
   `Shape` stays 64 because `PathShape` sets the payload ceiling. Anything wider than 8 B of
   payload there needs the boxed/`add_hdr` fallback (§8.6).
5. **`Stroke` stays 8 B** → `Shape::LineSegment` stays SDR. HDR segments are meshes or wrapped
   shapes.
6. Every PR that touches `Shape` sub-structs must keep `shape_size` green, and must add a guard for
   anything new that is size-sensitive (`PathStroke`, `ColorMode`, `Shape::Hdr`'s payload,
   `TextShape`, `Mesh`'s field set). The guard is a `#[test]`, not a `const` assert
   (`shapes/shape.rs:81-91`), so it runs in the normal test job.

`Mesh` is 64 B as `indices: Vec` (24) + `vertices: Vec` (24) + `texture_id: TextureId` (16)
— the surprising 16 is `TextureId`'s two-variant `u64` enum (`epaint/src/lib.rs:105-116`), not
something this work should fix. The sidecar is `Option<Vec<HdrVertexColor>>`: 24 B → `Mesh` 88,
`Arc`-ed inside `Shape`, and append-friendly. `Option<Box<[_]>>` (16 B → `Mesh` 80) buys 8 B and
loses push: the tessellator appends to `vertices` and the sidecar in lockstep, and `Mesh::append`/
`promote_to_hdr`/`split_to_u16` all grow it, so a boxed slice would reallocate and copy on every
pushed vertex. Resolved here (it was §16 question 6) rather than deferred: **`Vec`**.

---

## 6. Design space: where the second dtype lives

### 6.1 A — dtype inside `Vertex` (dual field, 28 B)

`Vertex { pos, uv, color: Color32, hdr: [u16; 4] }`, stride 28; shader picks one field per mesh tier
(pipeline variant) or per vertex (sentinel, see 6.2).

- **For:** one array, one geometry code path (`translate`/`rotate`/`append`/`split_to_u16` just work),
  one staging buffer, no length invariant.
- **Against:** +40% vertex memory and vertex-fetch bandwidth for **every** app, including the ~all-SDR
  majority; touches every `Vertex` literal (12 sites), the `unity` variant, `Vertex`'s serde shape,
  and any external code that constructs `Vertex` by literal (breaking change).
- **Note:** storing the field as `[u16; 4]` (f16 *bits*) rather than `[half::f16; 4]` preserves
  `Eq` and the serde derive with no new trait surface — the §11 "`Eq` drop" concern dissolves for
  this design too. The memory cost is the deal-breaker, not the traits.

### 6.2 A′ — dtype inside `Vertex`, selected per vertex (sentinel)

Same layout; one pipeline; the shader decides `hdr.a != 0 ? hdr : unpack(color)`. Sound in practice
(a fully transparent HDR vertex contributes nothing, so falling back to the u8 field cannot change
the image) but relies on a subtle invariant no reviewer will want to reason about, and it forfeits
the "loud, explicit tier" property the rest of the HDR stack is built on. Rejected as the primary;
recorded because it is the cheapest possible variant-matrix.

### 6.3 B — per-mesh color sidecar (**recommended**)

```rust
// epaint
pub struct Mesh {
    pub indices: Vec<u32>,
    pub vertices: Vec<Vertex>,          // unchanged, 20 B, glow-compatible
    pub texture_id: TextureId,
    pub hdr_colors: Option<Vec<HdrVertexColor>>,  // Some ⇒ tier = HDR; len == vertices.len()
}
```

- **SDR path:** `None` — no bytes, no staging branch, no pipeline change. Byte-identical to today.
- **HDR path:** +8 B/vertex (+40% for *those meshes only*), a second vertex buffer with
  `@location(3) Float16x4`, stride 8, and one pipeline variant.
- **For:** zero regression for every existing app; the §11 ripple list collapses to `Mesh` (one new
  `Option` field, `Eq` preserved since `[u16;4]` is `Eq`), the wgpu renderer, and one WGSL entry
  point; `Vertex`, glow's `Vertex` path, and `size_of::<Vertex>()` are untouched; sparse HDR content
  (a scope overlay over SDR chrome) costs nothing for the SDR meshes it is *not* in — per contiguous
  same-texture run, because the tier joins the tessellator's mesh-grouping key (§9).
- **Against:** one new invariant (`hdr_colors.len() == vertices.len()`, checked in `is_valid`,
  §8.2), a second `SlicedBuffer` and a per-primitive pipeline switch in the renderer (there is no
  batching layer to key on — §11), and a tier-mixing rule (§8.2: promote on append).
- **Pipeline matrix:** SDR and HDR meshes need different pipeline state because vertex *layout* is
  pipeline state (a used attribute must be backed by a bound buffer of sufficient length — a shared
  zero buffer cannot satisfy per-vertex fetch). Two variants keyed
  `(format, OutputEncoding, VertexColorTier)` in the existing cache, where `VertexColorTier` is a
  second derived-quantity enum next to `OutputEncoding` (`renderer.rs:1642-1669`), same
  `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]` and same "derived, never stored next to its
  inputs" doc rule:

  ```rust
  pub enum VertexColorTier { Sdr, Hdr }   // Sdr ⇔ Mesh::hdr_colors.is_none()
  ```

  **Selection is a second `@vertex` entry point in the same module (`vs_main_hdr`), not a spliced
  `const`** — investigated 2026-11-02: `VertexFormat::Float16x4` is consumed as **`vec4<f32>`** in
  WGSL (`wgpu-core-30.0.1/src/validation.rs:894-906`, `naga-types-30.0.1/src/lib.rs:265-266`), so the
  HDR path needs no `enable f16` and no `Features::SHADER_F16` on any backend, including web. A
  spliced const would double the module count from 2 to 4 (`renderer.rs:383-397,437-444` already
  has the `DITHER_CAPABLE` pair); an entry point doubles nothing.

### 6.4 C — mesh-level uniform HDR color (no per-vertex data)

`Mesh::hdr_color: Option<HdrVertexColor>`, multiplied into the u8 vertex color (which then only
supplies per-vertex alpha and tint).

- **For:** 0 B/vertex; covers solid HDR fills and solid strokes.
- **Against:** **does not unblock any of §1.2's three items** — no per-vertex gradient, no per-vertex
  HDR variation along a path. It also needs per-draw uniform state (dynamic offsets or push
  constants), which is a *bigger* renderer change than B. Keep as an optional cheap add-on if a
  use case appears; not the tier.

### 6.5 D — dual vertex types (`Vertex` vs `HdrVertex`)

`Mesh` holds either `Vec<Vertex>` (20 B: pos, uv, `Color32`) or `Vec<HdrVertex>` (24 B: pos, uv,
f16×4 bits), with `Shape::Mesh(Arc<Mesh>)` unchanged and the tier implied by which vector is
non-empty (or an explicit tag).

- **For:** the cheapest HDR representation of any design (24 B, no u8 color duplicated), one
  pipeline-variant difference, and no sidecar length invariant.
- **Against:** every geometry operation forks into two bodies — `translate`, `transform`, `rotate`,
  `calc_bounds`, `append`, `split_to_u16`, `is_valid`, `bytes_used`, `clear` — and every consumer
  that reads positions must match on the tier. The tessellator would emit into two different arrays,
  so its vertex-emission helper threads an enum instead of an `Option`. `Mesh`'s serde becomes a
  tagged representation, and `Mesh16` doubles the fork.
- **Verdict:** rejected. B reaches the same 28 B/vertex for HDR meshes with one array, one set of
  geometry ops, and one invariant; the 4 B/vertex D saves is not worth doubling the mesh surface.
  D becomes the right answer only if per-vertex bytes ever bind *and* HDR content dominates — the
  opposite of B's design target (HDR islands over SDR chrome).

### 6.6 E — channel-dtype abstraction (**investigated 2026-11-02; does not change B**)

Question asked: can one struct represent a vertex colour channel independently of its dtype, so that
epaint has one vertex-colour concept and one pipeline?

**Verdict: no — and not for want of a cleverer type.** The pipeline count is a function of the
*vertex buffer layout*, not of the Rust type. wgpu fixes `VertexBufferLayout` per pipeline
(`wgpu-core-30.0.1/src/device/resource.rs:4268-4334`) and validates at draw time that every
vertex-rate buffer covers the vertex count (`.../command/render.rs:446-534,3120-3121`), so an SDR
draw cannot be serviced by a pipeline that declares an f16 colour attribute, and cannot borrow a
shared dummy buffer either. Any one-pipeline design must therefore give **every vertex the same
layout** — i.e. pay the colour bytes on the SDR path — or move HDR colour out of the vertex layout
altogether (uniform/SSBO), which is §6.4's design, not a dtype abstraction.

| Candidate | What it buys | Why it does not change the verdict |
|---|---|---|
| Const-generic `Vertex<C>` / `Mesh<C>` | one Rust code path | `Shape::Mesh(Arc<Mesh>)` must stay non-generic → erase inside `Mesh` → the match reappears everywhere; the layout is still per-tier → still two pipelines, plus a public-API generic for nothing |
| Canonical-bits `Channel16(u16)` with per-pipeline interpretation (`Unorm16x4` vs `Float16x4` over the same bytes) | one Rust dtype, one width | the same bytes cannot decode correctly under both interpretations unless SDR stores unorm16 (`round(v*257)` decodes back to exactly `v/255`) — which is **8 B of colour per vertex for every vertex**, i.e. design A's memory cost with extra cleverness; two pipelines remain; wgpu 30 does expose `Unorm16x4`, so nothing is blocked, nothing is gained |
| f16-as-superset (A2) | one layout, **one** pipeline | blocked by glow, and for a different reason than first written: not `unpackHalf2x16` (unnecessary — drivers convert `GL_HALF_FLOAT` attributes to float), but **`GL_HALF_FLOAT` is not a legal `glVertexAttribPointer` type on GL 1.20 / ES 1.00**, which `egui_glow` explicitly supports (`egui_glow/src/shader_version.rs:24-43,57-62`). It would also need a per-frame CPU conversion of every vertex on the GL backend |
| Trait at the staging boundary (`trait VertexColor`) | tidier renderer code | no effect on pipeline count; optional, internal to 6a |

Two results from the same investigation are folded into §6.3 and §11 rather than repeated here:

- **No `enable f16` / `Features::SHADER_F16` is needed anywhere.** `VertexFormat::Float16x4` is
  consumed in WGSL as **`vec4<f32>`** (`wgpu-core-30.0.1/src/validation.rs:894-906`,
  `naga-types-30.0.1/src/lib.rs:265-266`) — WGSL never sees a 16-bit float.
- **`Float16x4` vertex fetch is core on every backend, including both web paths** (Metal via naga's
  vertex-pulling unpack, Vulkan `R16G16B16A16_SFLOAT`, DX12 `DXGI_FORMAT_R16G16B16A16_FLOAT`, GLES 3
  / WebGL2 `HALF_FLOAT`), and wgpu 30 has **no per-`VertexFormat` capability query** at all — the
  only per-format query is texture-shaped. A gate would be a false gate. See §11.

### 6.7 Comparison and recommendation

| | SDR bytes/vertex | HDR bytes/vertex | Vertex literals touched | glow | `Vertex` serde | new invariants | pipelines |
|---|---|---|---|---|---|---|---|
| **A** dual field in `Vertex` | 28 (+40%) | 28 | 12 sites + `unity` | untouched | changed | none | 2 (or 1 with A′) |
| **B** sidecar (**recommended**) | **20** | 28 | none | untouched | unchanged | 1 (`len` match) | 2 |
| **C** mesh uniform | 20 | 20 | none | untouched/ignored | unchanged | none | 1 + per-draw uniform (new machinery) |
| **D** dual vertex types | 20 | 24 | new type, parallel | two paths | two reprs | two geometry impls | 2 |
| **E** channel abstraction | 28 (if one layout) | 28 | any | unchanged only if u8 kept | depends | depends | 2 — **no gain; rejected** |

**Decision: B.** The default path is untouchable (egui's own perf contract: vertex memory is the
hottest allocation in the library, and every app pays it), the requested capability set is fully
covered by B + the API tier in §8, and B confines the change to code that already has a
format-keyed cache and a staging loop. Re-open A only if the second staging buffer or the variant
bit proves genuinely unmanageable — and note that A's memory cost is the argument upstream will
reject first.

---

## 7. Serialization, `Eq`, `unity`

Because B leaves `Vertex` alone:

- `Vertex`'s serde output, `Eq`, `Pod`, and both `unity` field orders are unchanged — the entire
  "pre-E decision" cluster from §11 (`Eq` drop, f16 serde repr, `half` feature surface) is
  **mooted**, not answered. Record that as a win.
- `Mesh` gains `#[cfg_attr(feature = "serde", serde(default))] hdr_colors:
  Option<Vec<HdrVertexColor>>` — the *gated* form every optional serde attribute in this tree uses
  (`tessellator.rs:665`, `style.rs:243`, …); a bare `#[serde(default)]` does not compile without
  the feature. `Mesh16` has no derives at all (`mesh.rs:337-348`), so it takes the field and no
  serde obligation. Old serialized meshes load as SDR (the correct semantic). New meshes read by old
  code lose the HDR tier — acceptable, and documented.
- `HdrVertexColor` stores `[u16; 4]` (f16 bit patterns) so `Eq`/`PartialEq`/serde derive without a
  NaN caveat: bit equality is the right semantics for a *render list* (a different NaN payload is a
  different byte pattern, and `-0.0 ≠ 0.0` is harmless). This is a deliberate deviation from
  `FloatImage`, which keeps `[half::f16; 4]` at rest and only turns it into bits at the serde
  boundary (`image.rs:746-800`): `FloatImage` is `PartialEq`-only, `Mesh` is `Eq`, and keeping
  `Mesh: Eq` is the cheaper half of the §11 trade. `half` stays an internal conversion detail.
  Document the representation in the type's rustdoc, and provide the accessors below so no user
  handles bits.
- The `unity` cfg variant needs no change at all (it only reorders `Vertex`), which is a second
  reason to prefer B: the "packing trick must work in both field orders" problem disappears.

---

## 8. API surface

### 8.1 `epaint::HdrVertexColor` (new, `epaint/src/hdr_color.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
#[repr(C)]
pub struct HdrVertexColor { bits: [u16; 4] }   // f16 patterns, §3 contract
```

Placed like every other epaint type: `mod hdr_color;` plus one line in `lib.rs`'s `pub use self::{…}`
block (`lib.rs:53-77`), so the public path is `epaint::HdrVertexColor` and the module stays private —
the same shape as `mesh`, `stroke`, `corner_radius`. Its rustdoc carries the §4 contract verbatim
(as `FloatImage`'s does, `image.rs:440-479`), links `FloatImage`, and states the bit representation.

Constructors/accessors follow `ecolor`'s value-type naming, not a private dialect:

- `from_rgba_premultiplied([f32; 4]) -> Self` — the contract boundary: encoded, premultiplied,
  unclamped, odd-symmetric; rounds f32 → f16 (`Color32::from_rgba_premultiplied` /
  `Rgba::from_rgba_premultiplied` set the name, `color32.rs:122`, `rgba.rs:60`; the image-side twin
  is `FloatImage::from_rgba`, `image.rs:519`).
- `to_array() -> [f32; 4]` — back, lossy (`Color32::to_array` / `Rgba::to_array`, `color32.rs:231`,
  `rgba.rs:188`). Not `encoded()`: every accessor in `ecolor` that returns components is `to_*`/`r()`.
- `from_linear([f32; 4]) -> Self` — the ergonomic constructor. Input is **straight (unmultiplied)**
  linear RGBA: each component is encoded with `ecolor::gamma_from_linear` (odd-symmetric since PR 2a)
  and then premultiplied, `stored = a × enc(L)`; `a` is the coverage and is not encoded. Say so in
  the rustdoc — "linear" alone does not settle premultiplication, and the two conventions differ for
  every translucent value.
- `impl From<Color32>` — the lift, a per-component `/255` rescale exactly as
  `FloatImage::from_color_image` (`image.rs:634-650`), not a transfer-function round trip. Test:
  absolute error `< 1/512` (< half an 8-bit step) for all 256 values per component. A `From` impl
  rather than an inherent `from_color32`, matching `impl From<Color32> for Rgba`.
- `TRANSPARENT` (`[0.0; 4]`), `WHITE`, `from_white_alpha(f32)` — the `Color32` const/method set,
  names included (`TRANSPARENT` at `color32.rs:60`, `from_white_alpha` at `:171`). `ZERO`/`from_alpha`
  would be a second vocabulary for the same two values.
- `From<HdrColor>` lives in `egui` (epaint must not depend on the picker type) and goes through
  `HdrColor::effective_linear` → `from_linear([r, g, b, 1.0])`; `egui` may also provide
  `impl From<Rgba>`.

Not `Copy`-detectable from `Color32` at the type level: the tier is a property of the mesh, not of a
value, which is what keeps the SDR path free.

### 8.2 `Mesh` / `Mesh16`

- `hdr_colors: Option<Vec<HdrVertexColor>>` on both (serde-gated, §7).
- `is_hdr(&self) -> bool`, and the length invariant lives in **`Mesh::is_valid`**
  (`mesh.rs:99-107`) / `Mesh16::is_valid` (`:352-359`): `hdr_colors.is_none() ||
  hdr_colors.len() == vertices.len()`. That is the existing hook — `append`, `append_ref`,
  `split_to_u16` and `Shape::mesh` already `debug_assert!(mesh.is_valid())`, so one added clause
  covers every mutation path that matters and no new "is_valid-adjacent" convention is invented.
  The renderer's staging loop asserts it too (it is the last place a desync would silently read
  out of bounds of the sidecar buffer).
- `colored_vertex_hdr(pos, color: HdrVertexColor)`, `add_colored_rect_hdr(rect, color)`,
  `add_rect_with_uv_hdr(rect, uv, color)` — the last one builds the four sidecar entries alongside
  the existing four `Vertex` literals (`mesh.rs:199-227`); the other two delegate to it the way
  `add_colored_rect` delegates today (`mesh.rs:229-237`).
- **Tier mixing (`append`/`append_ref`):** if exactly one side is HDR, `promote_to_hdr()` the SDR
  side — lifting every `Color32` via `HdrVertexColor::from(Color32)` preserves appearance within
  f16's worst-case relative error (~2^-11 ≈ 4.9e-4, i.e. well under half an 8-bit step, the
  `< 1/512` assertion §8.1 carries). Nothing in the merge is clamped. Never mix by silently dropping
  the HDR side. `promote_to_hdr()` is public and idempotent.
- `split_to_u16` carries the sidecar through both the fast path and the split path (`mesh.rs:242-303`);
  `bytes_used` counts it (`:92-97`); `clear` clears it (`:85-90`); `AllocInfo::from_mesh` counts it so
  the perf numbers do not lie (§4); and `translate`/`transform`/`rotate` are colour-free and
  unchanged.
- `Mesh::append_ref`'s existing "same texture" assert is unchanged.

### 8.3 Path color: `ColorMode` stays untouched (**decision reversed 2026-11-02**)

The earlier draft of this section added `ColorMode::HdrSolid` + `HdrUv`, a `PathStroke::hdr`
constructor and HDR arms in `get_color`/`mul_color`/`adjust_color_mode`, on §5's assumption that
`ColorMode` had "8 B of headroom". Measured, it has none: the 16 B two-variant enum is a
fat-pointer-niche layout and any third variant is 24 B regardless of payload type (§5). 24 B
`ColorMode` → `PathStroke` 32 → `PathShape` 64 → `Shape` past its guard, i.e. it breaks a
non-negotiable to buy an API convenience. **Reversed**, and the replacement is the same wrapper the
rest of the API uses (§8.4):

- **Antialiased HDR strokes and polylines** (§1.2 item 1):
  `painter.add_hdr(Shape::line(points, PathStroke::new(w, Color32::WHITE)), color)`. The stroke is
  tessellated by the existing path code — joins, caps, feathering, `StrokeKind`, varying-width bands
  and every future path feature — then recoloured by its own coverage alpha. Nothing about strokes is
  re-implemented for HDR.
- **HDR ramps along path-shaped geometry** (§1.2 item 3): the wrapper's positional colour source.

```rust
// epaint/src/color.rs, next to `ColorMode` — the same two-variant shape, so also 16 B
pub enum HdrColorMode {
    /// One HDR value for the whole shape.
    Solid(HdrVertexColor),

    /// Provide a callback which takes in the shape's bounding box and a position and converts it
    /// to an HDR value. The bounding box has the same feathering margin `ColorMode::UV` gets
    /// (`color.rs:16-19`).
    UV(Arc<dyn Fn(Rect, Pos2) -> HdrVertexColor + Send + Sync>),
}
```

- `Clone`, `Debug` and `PartialEq` follow `ColorMode`'s treatment (`color.rs:8,28-44`): hand-written
  `Debug`/`PartialEq`, `UV == UV` is `false`. **No serde at all** — `Shape` is not serde-derived
  (`shapes/shape.rs:26`: `Clone, Debug, PartialEq`), so `Shape::Hdr`'s payload never crosses a
  serialization boundary and a `serde(skip)` attribute would be dead weight. `impl From<HdrVertexColor>
  for HdrColorMode` makes `Solid` the `impl Into<…>` case at call sites.
- **Why a sibling type and not `ColorMode<C = Color32>`** (investigated 2026-11-02): the tree's
  idiom for "same concept, richer payload" is a sibling type plus a `From` — `CornerRadius` /
  `CornerRadiusF32` (`corner_radius.rs:13-18`, `corner_radius_f32.rs:22-30`), `Margin` /
  `MarginF32`, `Stroke` / `PathStroke`. There is no generic-with-default type in `epaint` or `egui`
  (only `egui_kittest::Harness<'a, State = ()>`), generalising would make `ColorMode`'s
  `Color32`-specific `Default`/`TRANSPARENT` conditional, force the first generic serde derive in
  `epaint`, and buy no shared implementation — `get_color` (`tessellator.rs:1118-1121`) and
  `adjust_color_mode` (`shape_transform.rs:132-147`) are `Color32`-only either way. Two contracts
  under one name (path colour vs. post-tessellation coverage mask) would be the cost with no benefit.
- **`PathStroke`, `ColorMode`, `Stroke`, `Shape::LineSegment`, `get_color`, `mul_color`,
  `adjust_color_mode` and `PathShape` are unchanged.** That removes three tessellator edits, one
  `shape_transform` edit, two size guards and one new enum variant in the `PathStroke` surface — and
  the tessellator keeps a single colour type.

### 8.4 Fills (deliberately not widened)

`PathShape::fill` and `RectShape::fill` are `Color32` and their structs are at the `Shape` ceiling
(§5). HDR *fills* are expressed as meshes:

- **`Shape::gradient_rect_hdr(rect, direction, [HdrVertexColor; 2])`** — the exact mirror of the
  existing `Shape::gradient_rect` (`epaint/src/shapes/shape.rs:323-346`), which is already a
  four-vertex mesh. Same `Direction` semantics, same corner assignment, plus the sidecar. Shipping a
  differently-named `gradient_quad` instead would be inventing a second API for a solved problem.
- **`Painter::rect_filled_hdr(rect, corner_radius, color)`** — `add_hdr(RectShape::filled(rect,
  cr, Color32::WHITE), color)`, i.e. the wrapper of the bullet below: interior vertices are
  `(1,1,1,1)` and take the colour, feather vertices are transparent and stay transparent, and the
  component-wise multiply of two premultiplied encoded pairs is exactly what the fragment shader
  does with a texel. This inherits rounded corners, stroke kinds, blur, brushes and every future
  `RectShape` feature instead of duplicating corner tessellation.
- **`Painter::add_hdr(shape: impl Into<Shape>, color: impl Into<HdrColorMode>) -> ShapeIdx`** — the
  generic escape hatch behind the parity map in §8.7. **Mechanism: a `Shape` wrapper, not eager
  tessellation.** The painter does not tessellate, and there is no public "tessellate one shape"
  API, so the wrapper is what makes one code path cover every shape:

  ```rust
  // 24 B payload (Arc<Shape> 8 + HdrColorMode 16) — under PathShape's 56 B ceiling, so Shape stays 64
  Shape::Hdr { inner: Arc<Shape>, color: HdrColorMode }   // §8.3 for the mode
  ```

  with one tessellator arm (§9): tessellate `inner` into `out`, then for the vertices it just
  emitted set `hdr = color(pos) × (vertex.alpha / 255)` and
  `vertex.color = hdr.projected_color32(alpha)`. `color(pos)` is the solid value or the callback
  evaluated on `inner.visual_bounding_rect().expand(feathering)` — the analogue of the path
  tessellator's `uv_bbox` (`tessellator.rs:1111-1116`). The inner shape's RGB is discarded; **its
  alpha channel is the coverage mask**, which is why feathering, rounded corners, antialiasing and
  glyph edges all keep working with no new geometry code.

  Consequences to document, and to test: a shape's fill and stroke collapse to one colour (uniform
  recolour), and an inner `ColorMode::UV` ramp is lost — a per-position ramp needs
  `HdrColorMode::UV`, `gradient_rect_hdr` (6a) or a hand-built mesh (6b).

  The constructor **flattens `Shape::Vec` at construction**, because `tessellate_shape`'s own
  contract says it can panic for `Vec`/`Callback` (`tessellator.rs:1563-1564`) and its mesh-level
  `Vec` branch loops children into one mesh, tripping the texture accumulator's
  `assert_eq!(out.texture_id, …)` (`tessellator.rs:952-956`) the moment two children use different
  textures. `Shape::hdr(inner, color)` therefore maps over `Vec` children recursively (so each child
  keeps its own primitive split and its own tier decision, §9), and passes `Shape::Callback` through
  uncoloured with a `debug_assert` — a callback owns its pipeline, so there is nothing to recolour and
  the honest fallback is uncoloured, not a rewrite of the callback's semantics. Constructing
  `Shape::Hdr` twice collapses to the outer colour (the inner wrapper's colour is already baked into
  its alpha mask); document that, and collapse it in the constructor.

  The variant needs arms in every exhaustive `match Shape`: `shape_transform::adjust_colors`
  (recurse — alpha-only is what matters, and the existing opacity/fade fold happens there, §9),
  `visual_bounding_rect` (`shape.rs:403-435`), `transform` (`:467+`), `PaintStats::add`
  (`stats.rs:192-230`), `tessellate_shape` (`tessellator.rs:1568`+), `should_parallelize`
  (`:2536-2552`) — and a **delegating `texture_id()` arm** (`shape.rs:437-445`), which is *not* an
  exhaustive match and therefore will not be flagged by the compiler (§4).
- **`Painter::add_hdr_uv(shape, callback)`** (6b) — `add_hdr(shape, HdrColorMode::UV(Arc::new(callback)))`,
  taking the closure directly the way `PathStroke::new_uv` does (`stroke.rs:151-161`), so a ramp call
  site never spells `Arc` either.
- `Painter::rect_filled_hdr` is then one line:
  `add_hdr(RectShape::filled(rect, cr, Color32::WHITE), color)` — the fill color supplies alpha only.
- A `Shape` variant carrying a *sized* HDR fill payload is still **not** proposed: the wrapper above
  is 24 bytes, while a per-shape HDR field would need headroom these structs do not have (§5).
  Boxing the wrapper (`Shape::Hdr(Arc<HdrShape>)`, the `Shape::Mesh` idiom) buys 16 B and costs an
  allocation per wrapped shape; 24 B already fits, so it is not worth it until a second HDR-only
  field appears.

### 8.5 The u8 twin (`Vertex::color` on an HDR mesh)

The wgpu HDR path reads `@location(3)` only, so `Vertex::color`'s four bytes are unread *there*.
The tessellator still writes them, as the **legal-premultiplied-`Color32` projection** of the HDR
color. Those bytes are the GL backend's entire rendering of an HDR mesh (§11): `egui_glow` reads
`a_srgba` at `offset_of!(Vertex, color)` (`painter.rs:244-252`) and its shader multiplies it in gamma
space (`shader/vertex.glsl:24-36`, `shader/fragment.glsl:56-70`), so the twin is a shipped consumer,
not dormant data — which is what keeps it inside the stack's "nothing dead" binding rule instead of
being a projection nobody ever looks at.

**The projection scales in linear light; it does not clamp per channel.**

```rust
// straight alpha first, or alpha contaminates the maximum
let lin = [r / a, g / a, b / a];
let k = 1.0 / lin.iter().fold(0.0_f32, |m, c| m.max(*c));
let scaled = lin.map(|c| c * k);          // chromaticity preserved exactly
// re-premultiply, encode, quantize: rgb <= a keeps the Color32 legal
```

A uniform scale of the tristimulus vector *is* a luminance change — every inter-channel ratio is
preserved, so hue and saturation are untouched by construction. Per-channel clamping alters the
ratios (`[6.0, 2.0, 0.5] → [1.0, 1.0, 0.5]`: a 3:1 red:green ratio becomes 1:1, a large hue shift)
and, in the encoded domain, compounds the error with the transfer curve. Two independent mistakes,
and neither is acceptable in a value the GL backend displays *today* (§11).

**Where Oklab fits — and where it does not.** `ecolor` has no Lab/Oklab/Oklch today (only
`Color32`, `Rgba`, `Hsva`, `HsvaGamma`, plus the transfer functions), and this tier does not need
any of it: the tier encodes linear values to f16 and hands them to an fp16 target, never mapping
between ranges. Chromaticity-preserving *range* reduction is exact arithmetic in linear light
(above). Perceptual *gamut* mapping — keep lightness, cut chroma until representable, CSS Color 4's
Oklch binary search — is a different promise (desaturate rather than dim) and is the only place
Oklab/Oklch earns its 60 lines of matrices. It is required by, in order of appearance:

1. a real HDR→SDR *preview* (if `egui_glow` ever degrades instead of rejecting, or if HDR scenes are
   dumped to PNG for humans);
2. the PR 5 picker's OkLCh geometry (already demand-gated in the main plan §5/§11);
3. nothing in this tier.

**Existing precedent, deliberately unchanged:** the kittest snapshot path clamp-converts fp16
targets to u8 (`egui_kittest/src/texture_to_image.rs:17-20,37`), documented as "values above 1.0
saturate to white" with numeric assertions living on the float readback instead. That is acceptable
*for a test oracle* (4.0 and 10.0 are equally white, by construction) and is **not** a precedent for
user-facing output — which is why the twin does not copy it.

### 8.6 Text

Measured (§5): `Option<HdrVertexColor>` is 10 B and fits `TextShape`'s trailing padding — 48 → 56,
`Shape` unchanged. So text takes a field, no boxing, and no need to dismiss text HDR:

- `TextShape::hdr_color: Option<HdrVertexColor>`: when set, it replaces the *glyph* colour of the
  whole shape — the same scope as `override_text_color` (`text_shape.rs:28-33`), and deliberately
  **not** the backgrounds, underline or strikethrough, which keep their own colours. That is the
  reason this is a `TextShape` field rather than `add_hdr` over a text shape: the wrapper's uniform
  alpha mask would recolour a text background too (§8.4).
- **Uniform per shape, stated honestly:** a galley with several runs in different colours (syntax
  highlighting, links, mixed `RichText`) loses that variation while `hdr_color` is set, because the
  HDR value lives on the shape and a per-run HDR colour would have to ride `TextFormat`/`Galley`,
  which are `Color32`-bound by design. Document it on the field; those callers use one
  `Shape::Text` per colour run (or leave the text SDR).
- `egui::RichText::color_hdr` / `ui.hdr_label(...)` as the public ergonomics — `RichText::color`'s
  mirror (`widget_text.rs:320`), and `hdr_label` is `colored_label`'s mirror
  (`ui.rs:1828-1834`: colour first, then text). The `hdr_text` texture-cache prototype from the main
  plan §6 is retired by this PR.
- The tessellator writes glyph vertices with the HDR sidecar set (coverage × color happens in the
  fragment shader, where `in.color * tex_gamma` is already value-transparent); the three special
  cases at `tessellator.rs:2328-2358` map as: `override_text_color` → the HDR value wins (it is the
  stronger override), `PLACEHOLDER` → HDR fallback, color-glyph anti-tint
  (`from_white_alpha(color.a())`) → alpha-only HDR value.

### 8.7 API parity map

**Rule: every SDR painting entry point needs an HDR way to say the same thing.** Satisfying that
with N bespoke methods would be bad design — and impossible for anything taking `Stroke`, which is
8 B and cannot grow (§5). So parity is satisfied twice over: one generic mechanism that covers every
shape (`add_hdr`, §8.4), plus bespoke mirrors only where the existing API is a *builder taking
colors positionally* (so the call reads like the SDR one).

| SDR entry point | HDR counterpart | PR |
|---|---|---|
| `Shape::gradient_rect(rect, dir, [Color32; 2])` | `Shape::gradient_rect_hdr(rect, dir, [HdrVertexColor; 2])` | 6a |
| `Painter::rect_filled(rect, cr, color)` | `Painter::rect_filled_hdr(rect, cr, color)` | 6a |
| `Painter::add(Shape::Mesh(mesh))` | `Mesh` with `hdr_colors` — no new method | 6a |
| `PathStroke` / `ColorMode::Solid` | `add_hdr(Shape::line(points, stroke), color)` — `ColorMode` untouched (§8.3) | 6a |
| `ColorMode::UV` (ramp along a path) | `add_hdr_uv(Shape::line(points, stroke), \|rect, pos\| …)` (`HdrColorMode::UV`) | 6b |
| `Painter::line_segment` / `hline` / `vline` | `add_hdr(Shape::line_segment(points, stroke), color)` — no bespoke method; `Stroke` cannot grow | 6a |
| `RichText::color`, `ui.label` | `RichText::color_hdr`, `ui.hdr_label` | 6c |
| `circle_*`, `ellipse`, `rect_stroke`, polygon fills, band fills, `hline`/`vline` | `Painter::add_hdr(shape, color)` — construct the shape yourself with a white color; §8.4 | 6a |
| `Painter::arrow` | **not** wrap-able: it is a procedure emitting three `line_segment` calls (`painter.rs:417-430`), not a builder. Needs `arrow_hdr` (a body copy over `add_hdr` + `line_segment`) or stays SDR | 6b |

Two classes of API, and only one is generic: **builders** return a `Shape` that `add_hdr` can wrap;
**procedures** add several shapes themselves and need their own HDR twin. Audit any new painting
method for which class it is before assuming parity is free.

`add_hdr` reuses the robust tessellation path rather than duplicating per-shape math, so it inherits
every future shape feature for free. Its documented limitations: it re-colours a shape *uniformly*
(a shape with a differently-coloured fill and stroke collapses to one colour — use `HdrColorMode::UV`,
or a hand-built mesh with the per-vertex sidecar), and it discards the inner shape's own positional
colours (an inner `ColorMode::UV` ramp is lost; put the ramp on the wrapper instead).

**Naming, once:** the suffix is the one the HDR stack already uses (`color_edit_button_hdr`,
`swatch_hdr`, `ColorMode` → `HdrColorMode`), and every new painting method takes
`impl Into<HdrVertexColor>` / `impl Into<HdrColorMode>` the way its SDR twin takes `impl Into<Color32>`
/ `impl Into<Stroke>` (`Shape::rect_filled` at `shapes/shape.rs:304-310`, `Painter::add` at
`painter.rs:213`). That is what makes `rect_filled_hdr(rect, cr, hdr_color)` — an egui
`color_picker::HdrColor` — compile through `impl From<HdrColor> for HdrVertexColor` (§8.1) without a
hand-written `.into()` at every call site.

### 8.8 The HDR picker (`color_picker.rs`) — the tier's cleanup story

PR 5's HDR widgets render HDR color as **float textures** because at the time §6 forbade HDR
vertices: `paint_hdr_swatch` allocates a 1×1 `FloatImage` per distinct color, cached by an
encoded-domain bucket key with `HDR_TEXTURE_CACHE_CAP = 256` and whole-cache eviction
(`color_picker.rs:917-926`). A solid color, rendered as a cached, capped, evictable texture, because
there was no other mechanism. That carve-out is what this tier removes.

After the tier:

- **Flat colors become meshes.** `paint_hdr_swatch` and the edit button's swatch become
  `rect_filled_hdr` (rounded corners included, §8.4). This **deletes** the swatch half of the
  texture cache — no 1×1 float textures, no bucket keys, no cap pressure — which is the strongest
  "consumed" evidence in the ladder: the widget code gets smaller.
- **Ramps stay textures, for a stated mathematical reason.** A hue ramp is not a linear function of
  position in RGB, and the exposure ramp is exponential in stops (§7 of the main plan's anchors); a
  four-vertex bilinear interpolation of either is visibly wrong. `FloatImage::exposure_ramp` stays.
- **Curved HDR gradients** (a future OkLCh wheel above 1.0) ride the wrapper's `HdrColorMode::UV` —
  per-position HDR values with arbitrary curves, no texture.
- The §6-era comments in `color_picker.rs` are superseded, in 6d, by the above split — the inline
  note ("the float-texture path is the one §6-compliant way to show the true color") and
  `HdrColor`'s own rustdoc claim that "vertex colors cap at 1.0 (plan §6 — stroked shapes and text
  cannot be HDR)" (`color_picker.rs:847-849`). Both are now false statements about the shipped tree
  and must go in the same PR that makes them false.

---

## 9. Tessellator integration

The tier is a property of a **mesh**, never of a vertex, so the tessellator has exactly two
producers and no colour threading:

- **`Shape::Hdr`** (§8.4) — one new arm in `tessellate_shape` (`tessellator.rs:1568`). Record
  `let first = out.vertices.len()`, tessellate `inner` (`Vec` → recurse per child, `Callback` →
  pass through uncoloured with a `debug_assert`, everything else → the existing arms), then for
  `out.vertices[first..]`: `a = vertex.color.a() / 255`, `color = source(rect, pos) × a`, write the
  sidecar entry and overwrite `vertex.color` with the projection (§8.5). `rect` is
  `inner.visual_bounding_rect()` expanded by the feathering margin, matching what
  `ColorMode::UV` receives today (`color.rs:16-19`). No `get_color` arm, no `mul_color` arm, no
  `PathStroke` field.
- **`TextShape::hdr_color`** (§8.6) — `tessellate_text` (`:2228`) writes the sidecar for emitted
  glyph vertices when the field is set; same projection for the twin.

Consequences:

- **Opacity/fade keep working with no HDR-space code.** `Painter::transform_shape` folds opacity and
  fade into `inner`'s `Color32` alpha before tessellation (`painter.rs:200-210`), and the wrapper
  reads alpha as the mask — so the HDR colour is dimmed exactly once, in encoded space, which is the
  §3 contract.
- **Feathering** is the alpha-0 outer ring the tessellator already emits; the wrapper writes a
  transparent sidecar there, and the encoded-space multiply keeps premultiplication exact.
- **"One mesh per (texture, tier)" is enforced where the tessellator already enforces one mesh per
  texture.** `tessellate_clipped_shape` starts a new primitive when
  `output_mesh.texture_id != shape.texture_id()` (`tessellator.rs:1524`); that condition gains
  `|| output_mesh.is_hdr() != shape.is_hdr()`. Without it, an HDR shape appended after SDR ones
  promotes the whole accumulated primitive (§8.2) — correct, but it would make §6.3's "sparse HDR
  costs proportionally nothing" claim false, because the merge granularity is the contiguous
  same-texture run and nearly every shape shares the white texture. With it, the tier is part of the
  batching key and the claim holds. `Shape::is_hdr()` is the predicate (`Shape::Hdr`, or
  `Shape::Text` with `hdr_color`), also the thing a `PaintCallback` can ask (§16 question 5).
- **A mesh is never mixed.** `Mesh::append`/`append_ref` promote (§8.2); the tessellator above never
  needs to, because it splits first.
- Anti-aliased `ColorMode::UV` gradients still clamp at 1.0; `HdrColorMode::UV` does not, because the
  ramp is evaluated on the HDR side of the wrapper.

---

## 10. Primaries semantics for vertex colors

**Decision: vertices are surface-referred, like `Color32` — not tagged.** Rationale:

- `Color32` chrome already means "values in the surface's granted primaries"; making vertices
  tagged would create two contracts for the same enum-shaped content and a new rejection path
  (`Primaries` pairing, `renderer.rs:1452-1570`) that would fire on *geometry* — a much larger
  blast radius than on images.
- The §5 chrome-shift analysis (saturated accents 2.2–6.8 CIEDE2000) therefore applies to HDR
  vertex colors unchanged: on a P3 surface, an `HdrVertexColor::from_linear([0.9, 0.1, 0.1])` built
  for sRGB shows shifted. This is the same behavior as the surrounding UI, which is the point.
- **The asymmetry must be documented loudly:** tagged `FloatImage` content pairs strictly and
  rejects; untagged vertex content never rejects and never converts. Apps wanting exact primaries
  convert app-side (the §5 recipe) — identical for vertices and textures.
- Tagged vertex content (`HdrVertexColor` + `Primaries`) stays demand-gated with §5's matrices; the
  mechanism (enum variant + pairing row + matrix) is already planned, so this defers nothing new.

---

## 11. Backends

**egui-wgpu (the only HDR-capable backend)**

- Layout variant: `@location(3)` as `Float16x4` (stride 8), backed by the sidecar buffer; in WGSL it
  arrives as `vec4<f32>` — no `enable f16`, no `Features::SHADER_F16` (`wgpu-core-30.0.1/src/validation.rs:894-906`).
- Module shape: **one WGSL module, two vertex entry points** (`vs_main`, `vs_main_hdr`); the
  pipeline variant selects the entry point *and* the layout. Both fragment entry points stay
  value-transparent, so no fragment change is needed at all — the multiply and the odd-symmetric
  decode already do the right thing at any magnitude.
- Renderer: a second `SlicedBuffer` (`renderer.rs:207-211`) for the sidecar, filled by a staging loop
  that mirrors the vertex one (`:1373-1410`) and pushed in lockstep so the two `slices` vectors stay
  index-aligned. `set_vertex_buffer(0, ..)` gains slot 1 on HDR meshes (`:728`).
- **No batching layer exists to key on.** `render()` draws one `draw_indexed` per `Primitive::Mesh`
  in paint order (`:714-735`) and sets the pipeline exactly once per pass (`:686`), from a lookup
  performed up front (`:652-661`). So the tier is honoured the only way this renderer can honour any
  state change: `render_pass.set_pipeline(..)` again when the tier differs from the previous mesh —
  a state switch, **never a reorder**. Batching by tier would reorder egui's primitives, and
  primitive order *is* paint order for translucent HDR content; that is a correctness regression, not
  an optimization. The cache key (`:305`) gains the tier component.
- **Both variants must exist before `render()`.** Pipelines are created in `update_buffers`
  (`ensure_pipeline`, `:597-624`, called at `:1262`); `render()` takes `&self` and no device, so it
  can only look up and `.expect(…)`. `ensure_pipeline` therefore checks and creates *both* tiers for
  the frame's `(format, encoding)` pair — two `contains_key` checks, one creation each, on first use.
- The zero-size-clip skip (`:694-710`) advances the index and vertex slice iterators by hand; it must
  advance the sidecar iterator too, or a clipped HDR mesh silently shifts every later mesh's colour.
- Dithering stays off for float targets (PR 3), unaffected.
- MSAA: orthogonal; PR 3's fp16-MSAA test already covers the pipeline creation path.

**egui_glow** — `Vertex` and the u8 attribute path are untouched, so SDR rendering is byte-identical
by construction, and **HDR meshes render through the u8 twin (§8.5)** with no shader or VAO change:
the tier's colour lives in `Vertex::color` as the legal SDR projection, which is exactly what
`a_srgba` and `fragment.glsl` already read. The tier is therefore *degraded*, not dropped: the
geometry is still antialiased, still in place, hue-correct, with values above 1.0 range-reduced by a
uniform linear-light scale (§8.5) — never gamut-clipped per channel.

Two consequences, stated because they contradict the earlier draft of this section:

- **Not the float-texture skip.** `set_texture` skips an `ImageData::Float` upload and warns
  (`painter.rs:515-531`) because there is no per-texel fallback to draw. A mesh has one, already in
  the vertex buffer, so skipping would discard geometry for no reason. This also matches the stack's
  content precedent: HDR *content* on an SDR surface is shown clamped/range-reduced
  (`color_picker.rs:850-851`, main plan §7), while a *tagged* upload the surface cannot interpret is
  the thing that rejects.
- **The warning needs a different key.** The existing warns are once-per-`TextureId`
  (`float_content_warned: HashSet<TextureId>`, `painter.rs:99-100`) because textures persist and own
  an id. Meshes are per-frame and have none, so the painter keeps a single `bool` and warns once per
  painter on the first HDR mesh: one line, no allocation, no per-frame spam. The message names the
  consequence (range-reduced SDR projection) and the escape hatch.

Document the escape hatch: apps targeting both backends gate the feature on
`Frame::wgpu_render_state()` (or accept the SDR projection on GL).

**web/wasm** — **the tier works, and there is no probe to write.** `Float16x2/x4` vertex fetch is
core WebGPU (browser backend maps it directly, `wgpu-30.0.0/src/backend/webgpu.rs:537-573`) and core
WebGL2/ES 3.0 (`GL_HALF_FLOAT` is a legal `glVertexAttribPointer` type — `wgpu-hal-30.0.0/src/gles/conv.rs:196-213`,
`glow-0.16.0/src/web_sys.rs:5259-5277`), and wgpu 30 exposes no per-`VertexFormat` capability query
at all (only texture-format features), so any gate would be a false gate. The real web constraint on
this stack is the **fp16 surface**, not vertex fetch: on WebGL2, `Rgba16Float` render targets need
`EXT_color_buffer_half_float` (`wgpu-hal-30.0.0/src/gles/adapter.rs:668-676,1199-1203`) — PR 4's
surface work already owns that decision. This interacts with the main plan's "web wiring deferred for
scope" (Open Question 1): nothing here expands that scope.

**capture / readback** — surface-level, format-driven, no per-mesh knowledge. Unchanged.

---

## 12. Testing and evidence

| Layer | Test | Why it can plausibly fail |
|---|---|---|
| CPU contract | `HdrVertexColor`: `From<Color32>` error `< 1/512`; `from_rgba_premultiplied` ↔ `to_array` round-trip; negatives preserved; `>1.0` preserved; premultiplication invariant under feathering multiply; `from_linear` premultiplies after encoding | f16 rounding and odd-symmetry at the negative branch are exactly where PR 2a found a real bug, and "linear" without a premultiplication convention is where every translucent value goes wrong |
| CPU structure | `size_of::<Shape>() == 64` still passes; `PathStroke`/`ColorMode`/`Vertex` asserted **unchanged** (24/16/20); new guards for `Shape::Hdr`'s payload and `TextShape` (56); the `hdr_colors.len()` invariant checked through `Mesh::is_valid`; `promote_to_hdr` idempotence; `split_to_u16` carries the tier | silent size regressions and forgotten mutation paths |
| CPU projection | the u8 twin preserves chromaticity (channel ratios intact within quantization), never emits `rgb > a`, and `promote_to_hdr` ↔ twin is stable across a round-trip | per-channel clamping is the tempting wrong implementation, and it is silently plausible |
| CPU wrapper | `Shape::Hdr` around a textured mesh starts its own primitive (the `texture_id` delegation, §8.4) instead of panicking in `append_ref`; a wrapped `Shape::Vec` tessellates child-by-child; a wrapped `Shape::Callback` passes through uncoloured | `Shape::texture_id()` is the one `Shape` site the compiler cannot point at (§4) |
| GPU numeric | kittest fp16 readback: mesh at 4.0 reads 4.0; at −2.0 reads −2.0; alpha-feathered edge midpoint; a `Shape::Hdr` gradient (`HdrColorMode::UV`) sampled at the midpoint of 1.0→6.0 | the whole point — u8 snapshots cannot see >1.0 |
| GPU numeric (variant) | same, MSAA 4× on fp16 | resolved-multisample fp16 readback is the PR 3 trap |
| Snapshot | HDR gradient + HDR text PNGs: silhouette, AA, and geometry only (values above 1.0 clip in a u8 PNG by definition); SDR snapshots must be **byte-identical** | a regression on the SDR path is the one unacceptable outcome |
| Serde | `Mesh` round-trip with and without the sidecar; old payload → SDR; `HdrVertexColor` serializes as its `[u16; 4]` bits; `Shape` round-trips **without** an HDR mode at all (`Shape` is not serde-derived) | the `cfg_attr(serde(default))` form on the new `Mesh` field, and the old-payload path |
| glow | SDR snapshot byte-identical; an HDR mesh draws its **twin** and logs one warning per painter (asserted through the same `log` capture PR 2b's reject test uses) | a backend that degrades must say so once, not silently |
| Perf | demo app vertex bytes/frame before/after via `stats`; SDR frame path asserted unchanged | the claim that makes B acceptable |

The GPU numeric and snapshot tests live in `crates/egui_kittest/tests/hdr_vertex_colors.rs`, next to
the existing per-concern files (`fp16_target.rs`, `snapshot_colorspace.rs`,
`wgpu_paint_callback.rs`), not inside `screenshot.rs`.

Every numeric test carries the value through *both* fragment entry points that the platform can
grant (gamma-encoded fp16 and, on CI headless fixtures, the linear path) — the same discipline PR 2a
applied to the GPU round-trip.

---

## 13. Performance budget

- **SDR:** 20 B/vertex, one staging buffer, one pipeline per `(format, encoding)` — unchanged.
  Assert it: a test that stages a mesh with `hdr_colors: None` must produce the same byte count and
  the same pipeline handle as before the change.
- **HDR meshes:** 28 B/vertex total (20 + 8) and a second traffic stream. The one CPU cost is the
  twin (§8.5): per emitted HDR vertex, at tessellation, a reciprocal, a max-reduce over three
  channels, a scale, and three `gamma_u8_from_linear` + quantize — paid only by meshes that asked for
  the tier, never by the SDR path. **No backend converts anything**: wgpu and glow cast slices
  (`bytemuck::cast_slice`), and glow reads the twin exactly as it reads an SDR vertex.
- **Worst case:** a full-frame HDR mesh costs +40% vertex bandwidth. Measure with the demo app's
  scope-shaped content; if the number is unacceptable, the fallback is A (28 B always) → worse, or
  C (uniform, no gradients) → less capable. There is no design that gives per-vertex HDR for free.
- Memory reasoning is the review argument, so state it in bytes: `20k` text-heavy vertices =
  400 KB → 440 KB when that mesh is HDR; SDR stays 400 KB.

---

## 14. PR ladder

| PR | Ships | Consumer | Tests |
|---|---|---|---|
| **6a** | `epaint::HdrVertexColor`; `Mesh`/`Mesh16` sidecar + the `is_valid` length invariant + promote + serde; `Shape::Hdr` wrapper with `HdrColorMode::Solid` and `Painter::add_hdr`; `Shape::gradient_rect_hdr`, `Painter::rect_filled_hdr`; wgpu layout variant + `vs_main_hdr` + cache key + second `SlicedBuffer` + per-mesh pipeline switch; glow draws the twin with a warn-once; `CHANGELOG` entries in `crates/epaint`, `crates/egui-wgpu`, `crates/egui_glow` and the root `CHANGELOG.md` (including epaint's `⚠️ Breaking changes`: `Mesh` gained a field, `Shape` gained a variant) | raw HDR geometry in any eframe app; the picker's flat swatches (§8.8) | CPU contract + twin projection, size guards (incl. `PathStroke`/`ColorMode` unchanged), wrapper/`texture_id` test, fp16 numeric readback (4.0 / −2.0), MSAA variant, serde, SDR-byte-identical, glow twin + warn-once, parity (`gradient_rect_hdr` equals `gradient_rect` for SDR-equivalent colors) |
| **6b** | `HdrColorMode::UV` + `Painter::add_hdr_uv` (positional HDR ramps on any wrapped shape); `Painter::arrow_hdr`; the stroke/polyline/band/plot-trace story that `add_hdr` already carries gets its snapshots and its documented density limit; no `PathStroke`, `ColorMode`, `Stroke` or `Shape::LineSegment` change | antialiased HDR strokes, polylines, plot traces, band gradients | ramp sampling at the 1.0→6.0 midpoint, feathering through the mask, snapshot of an HDR trace, SDR path identical |
| **6c** | `TextShape::hdr_color`, `RichText::color_hdr`, `ui.hdr_label`; tessellator's three text special cases; retire the demo's `hdr_text` prototype | HDR labels/readouts as first-class content | numeric readback of glyph cores >1.0, color-glyph anti-tint, SDR text snapshots identical |
| **6d** | picker re-wiring (§8.8: flat swatches off float textures, `HDR_TEXTURE_CACHE_CAP` narrowed to ramps, §6-era comments superseded), `MANUAL_TESTING.md` HDR section, perf write-up, demo `hdr_test_app` migrated off texture-strips | adoption + evidence for upstream | picker snapshots unchanged; full-suite |

**Definition of done for the tier:** all three §1.2 items demonstrably solved in the demo app
(HDR trace, HDR text, HDR gradient), SDR snapshots byte-identical across wgpu/glow, no `Shape`
growth, and no per-frame CPU color conversion on the SDR path on any backend (HDR meshes pay the
one-pass twin projection from §13, at tessellation, in epaint).

Anything not in 6a–6d is demand-gated by construction (primaries tagging, `Shape`-level HDR fills,
vertex-color tier on web, HDR10 plumbing).

---

## 15. Risks and anticipated review objections

1. **"Why does a 2D UI need HDR vertices?"** Answer with §2's scenario list, the demo, and the fact
   that the SDR path is byte-identical. The tier is opt-in at the *mesh* level, not an app setting.
2. **"+40% vertex memory."** Only for meshes that carry HDR color; the default path is untouched —
   the concrete numbers in §13 are the argument.
3. **"A second pipeline variant per format is a maintenance cliff."** It is one boolean in an
   existing key plus one extra `@vertex` entry point in the same module (no new WGSL module, no
   `enable f16`); PR 1 already proved the cache mechanism, PR 4 added a key component, and the
   invariant test that enumerates the matrix lives with the existing
   `float_target_tables_agree`-style checks.
4. **Sentinel temptation.** A′ is cheaper; reject it in review with §6.2's explicit argument
   (invisible-alpha vertices are the only safe case, and that is a proof no reader should have to
   redo).
5. **Tier mixing on `append`.** `promote_to_hdr` is the only lawful merge; a silent drop is a bug
   class, so the invariant is asserted in both `Mesh` and the renderer.
6. **glow divergence.** The GL path draws the twin and warns once (§11): degraded, loud, and — the
   point of the twin — hue-correct rather than gamut-clipped. Documented in one place with the
   `wgpu_render_state()` gate recipe.
7. **"`Shape::Hdr` is a public wrapper variant."** It is the only mechanism that reaches every shape
   without growing a payload struct (§5, §8.4), and it is 24 B under `PathShape`'s ceiling. The
   honest limitation — it recolours *uniformly* by alpha, so a shape with a differently-coloured
   fill and stroke collapses to one value — is rustdoc'd and tested, and the escape hatches
   (`gradient_rect_hdr`, `HdrColorMode::UV`, a hand-built mesh) are named next to it.
8. **Primaries asymmetry (tagged textures vs untagged vertices).** Documented in §10 and in both
   types' rustdoc; the alternative (tagging vertices) is a bigger rejection surface and is deferred
   with the matrices.
9. **Serde evolution.** Sidecar field defaults to `None`; old payloads keep working; new payloads
   read by old code lose the tier (documented, not a data-loss bug — the mesh still renders).

---

## 16. Open questions

1. ~~**Channel-dtype abstraction**~~ — **answered 2026-11-02 (§6.6):** no candidate removes the
   second pipeline variant, because pipeline count follows the vertex buffer layout; design B stands.
   Side result: the tier needs no `enable f16` anywhere.
2. ~~**`Float16x4` vertex fetch on wgpu-on-web**~~ — **answered 2026-11-02 (§11):** core on both web
   paths; wgpu 30 has no per-`VertexFormat` query, so no gate is possible or needed. The web
   constraint is the fp16 *surface* (PR 4's territory).
3. ~~**`TextShape` payload**~~ — **answered 2026-11-02 (§5):** `Option<HdrVertexColor>` measures 10 B
   and lands in `TextShape`'s existing trailing padding — 48 → 56, `Shape` unchanged. No boxing, and
   no need to dismiss text HDR.
4. **`HdrColorMode::UV` callback cost** — a per-position closure is fine for a trace; if a plot
   library calls it per emitted vertex per frame, document the density limit or provide a precomputed
   `Vec<HdrVertexColor>` path. (Solid HDR content never pays it: `HdrColorMode::Solid` is a copy.)
5. **Tier interaction with `Shape::Callback`** (custom paint callbacks) — callbacks own their
   pipelines; note only that they can read `Mesh::is_hdr()`.
6. ~~**`Option<Vec<_>>` vs `Option<Box<[_]>>`**~~ — **resolved 2026-11-02 (§5):** `Vec`. The sidecar
   grows in lockstep with `vertices` in the tessellator and in `append`/`promote_to_hdr`/
   `split_to_u16`; a boxed slice reallocates and copies on every pushed vertex to save 8 bytes on an
   `Arc`-ed struct.
7. **Whether the staging-side `trait VertexColor` (§6.6, last row) pays for itself** — internal only;
   decide while writing 6a's staging loop, not before.
8. **Oklab/Oklch in `ecolor`** — not needed by this tier (§8.5), but it is the right tool the moment
   anyone wants a *perceptual* HDR→SDR mapping: a real preview if `egui_glow` ever stops drawing the
   twin, human-readable PNG dumps of HDR scenes, or the picker's OkLCh geometry. Demand-gated
   as its own `ecolor` item; the range-reduction half of any such mapping is linear-light scaling,
   which needs no color science at all.
9. ~~**Where HDR path colour lives**~~ — **answered 2026-11-02 (§5, §8.3):** not `ColorMode` — any
   third variant measures 24 B and takes `Shape` past its guard. It rides the `Shape::Hdr` wrapper,
   whose 24 B payload fits. See Appendix C.3.

---

## Appendix A — carried pre-decisions from `hdr-egui-upstream.md` §11, re-adjudicated

| Pre-E decision then | Status under design B |
|---|---|
| `Eq` drop for `Mesh`/`Shape` | **moot** — `Vertex` untouched; the sidecar is `[u16;4]` (Eq). (`Shape` was never `Eq` — its `ColorMode::UV` closure forbids it — so the original carry overstated the cascade; see §1.1.) |
| serde repr of the vertex color field (`Color32` repr vs `half/serde`) | **moot** for `Vertex`; `HdrVertexColor` serializes as `[u16;4]`, documented as f16 bits |
| `half`'s `bytemuck` feature as epaint's direct surface | **not needed** — `[u16;4]` is already `Pod`; `half` stays an internal conversion detail |
| Layout fight: stride 20→24 / dual-format meshes / sidecar buffers | **decided: sidecar**, with the +40%-for-all-apps number as the disqualifier for the other two |
| The `unity` field-order packing trick | **moot** — `Vertex` shape unchanged |
| Trigger to revisit: animated HDR vector content, HDR text as a first-class need | **both are the reason this spec exists**; the demo's scope trace is the acceptance vehicle |

## Appendix B — carried from §6/§7 of the main plan

- The intermediate-linear render pass (§7) remains the alternative for photometric blending *without*
  the vertex tier; if it lands first, its linear blend grammar does not change this contract
  (vertices store encoded values either way; the linear path decodes at the fragment, odd-symmetric).
- "Gradients above 1.0 across arbitrary shapes" is satisfied for path-shaped geometry by the
  wrapper's `HdrColorMode::UV` (§8.3) and for arbitrary filled shapes by mesh builders (§8.4);
  *interpolated* gradients across
  tessellated non-path shapes (rounded joins, feathered fills) are limited by the tessellator's
  per-vertex masking, which this tier makes HDR-capable but does not make more expressive.

## Appendix C — investigation record (2026-11-02)

Two read-only investigations were run before this spec was accepted; both are folded into the body,
recorded here with their provenance so the reasoning is not re-derived.

**C.1 — Channel-dtype abstraction (§6.6).** Question: can one type abstract the vertex colour
dtype away, so epaint has one colour concept and one pipeline? Answer: no. Pipeline count follows
the vertex buffer layout (`wgpu-core-30.0.1/src/device/resource.rs:4268-4334`; draw-time coverage
validation at `.../command/render.rs:446-534,3120-3121`), so an SDR draw cannot use an
f16-declaring pipeline or a dummy sidecar; every one-pipeline design pays colour bytes on the SDR
path or leaves the vertex layout entirely. Corrections it forced on this document:

- The "spliced const" mechanism in early drafts of §6.3 was wrong-headed; `Float16x4` arrives as
  `vec4<f32>` in WGSL, so a second `@vertex` entry point is the right shape and **no `enable f16` /
  `Features::SHADER_F16` is required** on any backend.
- The glow blocker is `GL_HALF_FLOAT` not being a legal `glVertexAttribPointer` type on
  GL 1.20 / ES 1.00 (`egui_glow/src/shader_version.rs:24-43,57-62`), not any WGSL/`unpackHalf2x16`
  limitation.
- The §11 "`Eq` cascade `Vertex → Mesh → Shape`" was overstated: `Shape` derives `PartialEq` only
  (`shapes/shape.rs:25-27`) because `ColorMode::UV` holds a closure whose `PartialEq` returns
  `false` for `UV == UV` (`color.rs:44-57`). Only `Mesh`'s `Eq` is at stake.

**C.2 — f16 vertex fetch capability (§11).** Question: is `Float16x2/x4` fetch usable on every
backend, and how should the renderer probe it? Answer: it is core on all five backends including
both web paths, and **wgpu 30 exposes no per-`VertexFormat` capability query at all** — the only
per-format query is texture-shaped (`Adapter::get_texture_format_features`). Pipeline validation
gates only f64 attributes (`Features::VERTEX_ATTRIBUTE_64BIT`), so an f16 attribute passes
unconditionally. Consequences recorded: Metal needs no relevant hardware support (naga unpacks via
vertex pulling), GLES 3 / WebGL2 use core `HALF_FLOAT` with `normalized` inert for float types (so
values > 1.0 survive), and no gate should be written because a gate would be a false gate. On the GL
path an unsupported attribute type would fail *silently* (no `glGetError` check in wgpu-hal), which
is another reason the tier's reject path is backend-explicit rather than probe-driven. The web
constraint that actually matters is the fp16 *surface* (`EXT_color_buffer_half_float` on WebGL2),
which is PR 4's territory.

**C.3 — house-style and measurement re-verification (2026-11-02).** A second pass checked every
proposed API, derive, name, error path and test against how this tree already does the same thing.
Three claims did not survive; each is corrected where it bites, and none of the corrections weakens a
capability, the §3 contract, or the SDR-untouched rule.

1. **`ColorMode` cannot take an HDR variant** (§5, §8.3) — the §5 assumption of "8 B of headroom" was
   measured false: any third variant is 24 B (the two-variant fat-pointer niche is exhausted), which
   propagates `PathStroke` 32 → `PathShape` 64 → `Shape` past its guard. HDR path colour was moved
   to the `Shape::Hdr` wrapper, which is *also* the mechanism every other shape uses. A read-only
   style investigation then compared generalising `ColorMode<C = Color32>` against a sibling
   `HdrColorMode` and recommended the sibling: 24 B payload either way, no shared implementation
   (`get_color`/`adjust_color_mode` are `Color32`-only), no generic-with-default type anywhere in
   `epaint`/`egui`, and the tree's own idiom for "same concept, richer payload" is a sibling plus
   `From` (`CornerRadius`/`CornerRadiusF32`, `Margin`/`MarginF32`, `Stroke`/`PathStroke`). It also
   caught that `Shape` has no serde derive, so the new mode needs no serde at all.
2. **The renderer has no batching layer** (§11) — `render()` sets one pipeline per pass (`:686`) and
   draws one primitive at a time in paint order (`:714-735`); pipelines are created eagerly from
   `update_buffers` (`:597-624`), and `render()` can only look up. The tier is honoured by a pipeline
   *switch* when the tier changes, never by reordering; and the zero-size-clip skip (`:694-710`) must
   advance a third slice iterator.
3. **glow's warn-once cannot key on a mesh** (§11, §8.5) — every existing warn is once per
   `TextureId` (`float_content_warned`, `painter.rs:99-100`) because textures own ids; meshes do not.
   The GL path therefore renders the u8 twin (which makes the twin a shipped consumer rather than
   dormant data) and warns once per painter through a `bool`. That also resolved an internal
   contradiction: §8.5 already called glow the twin's consumer while §11 skipped the mesh.

Two smaller items from the same pass: naming now follows `ecolor`'s value-type vocabulary
(`from_rgba_premultiplied`/`to_array`/`TRANSPARENT`/`from_white_alpha`, `impl From<Color32>`), and the
`sized` claims that were "measure before committing" are measured (§5: `Option<HdrVertexColor>` is
10 B and `TextShape` lands at 56, `Shape` unchanged; the wrapper payload is 24 B).
