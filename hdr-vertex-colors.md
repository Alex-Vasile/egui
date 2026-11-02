# HDR vertex colors — extension spec

**Status:** spec (no code) on `hdr-pr6-float-vertex-colors`, branched off
`hdr-side-items-test-app` — the top of the HDR stack (`hdr_enablement` → … → `hdr-pr5-hdr-widgets`
→ `hdr-side-items-test-app`).

**Relation to the main plan:** this document *extends* `hdr-egui-upstream.md`. That plan's §11
parked the vertex-color tier, and its §6 listed exactly what its absence costs; both are carried here
in full (§1), re-grounded against the tree (§4), and turned into a design space (§6), an API surface
(§8), and a PR ladder (§14). Nothing here changes a decision already shipped in PRs 1–5.

**Anchors:** re-verified against the working tree at `714f2a0cf`
(`hdr-side-items-test-app`, 2026-11-02), then re-checked in a second (house-style) pass, then a
third pass (review of the second) re-resolved **every** anchor in this document against the file it
names and re-ran the §5 layout probe; stale anchors, one non-existent file (`egui/src/visuals.rs`)
and one orphan-rule violation were corrected in that third pass (Appendix C.4). A fourth pass — the
fixed-point check *of* C.4's own edits — then found four of those edits wrong in their details, one
gap left open, one of C.4's own anchor "corrections" wrong, and six stale or bare anchors; all are
fixed where they bite (Appendix C.5). A fifth pass (Appendix C.6) — the fixed-point check of C.5 —
still found one uncovered vertex path (the wholesale `Vec` arm) and one wrong fade prescription, plus
a third round of stale anchors; all fixed where they bite. Line numbers are as of this branch and
were confirmed by reading, not by arithmetic.

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
| What is the contract of the stored value? | The §3 contract, unchanged: gamma-encoded in the paint space, premultiplied in the encoded components, unclamped, odd-symmetric for negatives | 3 |
| Does it touch the default path? | No. `Vertex` stays 20 B, `Shape` stays 64 B, SDR staging is byte-identical | 5, 6.6 |

**Headline design decision (§6.7):** the ripple the main plan §11 feared — `Eq` cascade, serde rep
of the color field, every `Vertex` literal site, the `unity` layout, glow's stride — belongs to
designs that put the dtype *inside* `Vertex`. A per-mesh sidecar keeps `Vertex`, `Mesh`'s `Eq`, serde
of vertices, and glow's vertex path untouched, and costs nothing for the all-SDR default path. Accept
the sidecar's one new invariant (`hdr_colors.len() == vertices.len()`) and a second pipeline variant
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

Reviewed again the same day (third pass, Appendix C.4): C.3's measurements and its one reversal
stand, but its *mechanism* claims did not. The blocking ones were an incomplete `Shape::is_hdr()`
(a hand-built HDR mesh missed the batching key), opacity/fade never reaching HDR content, a twin
projection that divided by zero on feather vertices and inverted all-negative values, a text sidecar
that ignored the underline, an `impl From<Rgba>` that cannot compile where it was placed, a wrapper
over `Shape::Callback` that would panic, and §11/§13 disagreeing about whether an SDR frame compiles
an HDR pipeline. All seven are fixed where they bite; the SDR-is-untouched rule is *stronger* for it
(an SDR frame now provably creates one pipeline, dims nothing differently, and allocates nothing
new).

Checked a fourth time (Appendix C.5) — this time re-deriving C.4's *fixes* rather than trusting them
— four of those fixes were themselves wrong in the details: the second colour adjuster double-dimmed
HDR text (its sidecar multiplies in the very alpha the `Color32` fold writes), the twin's
premultiply/encode order was inverted (which breaks the `rgb <= a` legality the same line asserts),
`peak <= 0` returned `TRANSPARENT` instead of black (opaque sub-black geometry vanished on glow), and
the callback interception left a callback nested in a wrapped `Vec` reaching `tessellate_shape`'s
panic. `tessellate_text` also needed the wrapper's promote-first rule, and §11's "the pipeline scan is
the same pass" was not the same pass (`ensure_pipeline` runs above the fold). Each is fixed where it
bites; none of them touches a capability, the §3 contract, or the SDR-is-untouched rule.

Checked a fifth time (Appendix C.6): C.5's corrections hold, but the fixed point was still not
reached. Promote-first turned out to cover only the vertices that already existed, so the rayon
wholesale `Vec` path could still append raw vertices after an HDR producer — an invariant break, and
a debug-build panic. The fade mirror's "halve alpha" was wrong for opaque values, which would have
dimmed an HDR mesh on wgpu while its `Color32` twin on glow stayed opaque, and `render()`'s single
up-front pipeline lookup would have panicked on an HDR-only frame. All three are fixed where they
bite, and the anchors got corrected again.

---

## 1. What was deferred (carried from `hdr-egui-upstream.md`)

### 1.1 The ladder entry (main plan §11, verbatim intent)

`Vertex` color `u32` → `f16×4` ripples through every mesh consumer: egui-wgpu pipelines, egui_glow
(or explicit scoping-out), kittest, capture, mesh serde + the `Eq` cascade (`Vertex → Mesh → Shape`,
both cfg variants including `unity` — the packing trick must work in both field orders), and
`Context::tessellate` consumers. The layout fight (stride 20→24 vs dual-format meshes vs sidecar
buffers) is the sticky review thread.

> **Correction to the carry (2026-11-02):** the `Eq` cascade stops at `Mesh`. `Shape` derives
> `Clone, Debug, PartialEq` only (`epaint/src/shapes/shape.rs:25-27`) and cannot be `Eq`, because
> `ColorMode::UV` holds a closure whose manual `PartialEq` returns `false` for `UV == UV`
> (`epaint/src/color.rs:37-46`). Nothing in this spec can make `Shape` `Eq` fail; only `Mesh`'s
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

- **Content contract (main plan §4).** Float colors are gamma-encoded in the declared primaries,
  premultiplied in the encoded components, odd-symmetric for negatives, 1.0 = SDR reference white.
  `FloatImage`'s rustdoc (`epaint/src/image.rs:440-479`) is the canonical statement. Vertex colors
  adopt it verbatim (§3).
- **Primaries mechanism (main plan §5).** The mechanism is primaries-generic: a primaries = enum variant +
  pairing row + conversion matrix, one PR, no design change. Vertex colors are **un**tagged content
  (§10), so they ride the surface's granted primaries the way `Color32` chrome does.
- **Chrome-shift numbers (main plan §5).** A P3-referred surface reinterprets sRGB-referred bytes: neutrals
  shift 0.0, saturated accents 2.2–6.8 CIEDE2000. Applies to vertex colors identically, and is
  amplified by HDR values.
- **Pipeline cache (main plan §2 / PR 1).** Format-keyed and already extended by PR 4 to
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

Adopt the main plan's §4 exactly (restated as this section), so a float-textured swatch and an HDR
vertex at the same value are indistinguishable:

- **Stored values are encoded** (gamma curve of the paint space), never linear. The shader's
  existing multiply `in.color * tex_gamma` (`egui.wgsl:172,201`) stays valid at any magnitude.
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
  `:352-358` `Mesh16::is_valid` (no derives at all on `Mesh16`, so no serde obligation there).
- `epaint/src/mesh.rs:85-96` — `clear` (must also clear a sidecar) and `bytes_used`
  (`size_of::<Vertex>()` arithmetic); `:99-107` `is_valid` — the existing `debug_assert` hook every
  mutation path already calls.
- `epaint/src/mesh.rs:132-165` — `append` / `append_ref`: where the tier-mixing rule lives.
- `epaint/src/mesh.rs:199-227` — `add_rect_with_uv` (four literal `Vertex` constructions).
- `epaint/src/mesh.rs:337-348` — `Mesh16` (the u16-index split; mirrors `Mesh` field-for-field).
- `epaint/src/mesh.rs:243-306` — `split_to_u16`.

**Every `Vertex` literal site** (all would need the new field under a design that touches `Vertex`):
`mesh.rs:206,211,216,221` · `tessellator.rs:988,995,1008,2356` · `text_layout.rs:1311,1316,1321,1326`
· `egui-wgpu/src/renderer.rs:3334` (test).

**Color plumbing that must learn about HDR values**

- `epaint/src/color.rs:8-20` — `ColorMode { Solid(Color32), UV(Arc<dyn Fn(Rect, Pos2) -> Color32>) }`,
  16 B: the path-color abstraction. **Not** the seam for HDR path color: a third variant measures
  24 B and the growth propagates to `Shape` (§5, §8.3).
- `epaint/src/stroke.rs:113-160` — `PathStroke { width, color: ColorMode }`; `:198` `is_empty`.
- `epaint/src/stroke.rs:13-16` — `Stroke { width: f32, color: Color32 }`, 8 B — **cannot** grow
  without tripping the `Shape` size guard (§5). `Shape::LineSegment { points, stroke }` therefore
  stays SDR; HDR line segments go through `Mesh`.
- `epaint/src/tessellator.rs:1072-1130` — stroke tessellation; `get_color(&ColorMode, Pos2)`.
- `epaint/src/tessellator.rs:1408-1412` — `mul_color` → `gamma_multiply`.
- `epaint/src/tessellator.rs:854-857` — opacity folded into existing vertices.
- `epaint/src/tessellator.rs:2328-2358` — text vertex propagation: `override_text_color`,
  `PLACEHOLDER` fallback, color-glyph anti-tint, opacity factor.
- `epaint/src/shape_transform.rs:88-111,132-147` — color adjustment for placed rows and
  `ColorMode` callbacks (must gain the HDR arm). `:114-121` destructures
  `Mesh { indices: _, vertices, texture_id: _ }` **exhaustively** — it stops compiling the moment
  `Mesh` gains a field, which is the compiler doing the work of finding every such site. This
  function is also where `Painter::multiply_opacity` / `tint_shape_towards`
  (`egui/src/painter.rs:555-569`) land, so it is the one place that has to learn to dim
  `Mesh::hdr_colors` and `TextShape::hdr_color` (§9).
- `epaint/src/shapes/shape.rs:437-445` — `Shape::texture_id()`: an `if let` chain, **not** an
  exhaustive match, so the compiler will *not* point at it when `Shape` gains a variant. It drives
  the tessellator's mesh-grouping (`tessellator.rs:1518-1536`), so a variant that wraps geometry
  must delegate here or the tessellator appends across textures and panics on
  `tessellator.rs:954-957`'s `assert_eq!`.
- `epaint/src/shapes/shape.rs:403-431` — `visual_bounding_rect` (exhaustive, compiler-visible) and
  `transform` (`:467`).
- `epaint/src/stats.rs:192-230` — `PaintStats::add` (exhaustive; a new `Shape` variant needs an
  arm), `:160-176` its fields, `:232-245` `with_clipped_primitives`. `AllocInfo` (`:14-21`) is a
  bare four-field aggregator and is *not* where a counter goes: the sidecar is counted by extending
  `AllocInfo::from_mesh` (`:93-95`) — and only if it deserves its own row, by a new `PaintStats`
  field. `egui/src/introspection.rs:78-120` destructures `PaintStats` exhaustively and renders one
  row per field, so a new field also gains its row there.
- `epaint/src/text/text_layout.rs:1284-1330` — glyph vertex emission.
- `epaint/src/shapes/rect_shape.rs:24,30,56` — `fill: Color32`, `stroke: Stroke`, `brush`.
- `epaint/src/shapes/band_shape.rs:59-71`, `shapes/shape.rs:27-78` — shape payloads.
- `egui/src/visuals.rs` **does not exist** (an earlier draft cited `:87-89` here). The
  text-colour-override mechanism is a `TextShape` field, not a `vertex.color` write:
  `TextShape::override_text_color` (`text_shape.rs:30-33`) consumed in `tessellate_text`
  (`tessellator.rs:2330-2337`), glyph-range only. That is the exact scope `TextShape::hdr_color`
  copies (§8.6).
- `egui/src/text_selection/label_text_selection.rs:286-305` — the selection *hides* already-painted
  text by setting `row.visuals.mesh.vertices[i].color = Color32::TRANSPARENT` on the **galley mesh**,
  i.e. before `tessellate_text` runs. This is safe for the tier only if the HDR sidecar carries the
  vertex alpha as a factor (a cleared vertex must produce a zero sidecar), which is why §8.6 folds
  `vertex.color.a()` into the stored value rather than relying on the fragment.

**GPU side**

- `egui-wgpu/src/renderer.rs:597-624` — `ensure_pipeline`: one pipeline per `(format, encoding)`,
  created eagerly from `update_buffers` (`:1262`), so `render()` can only *look up*.
- `egui-wgpu/src/renderer.rs:643-661` — `render()` looks the one pipeline up front and
  `.expect(…)`s it; `:686` sets it once per pass and only again after a paint callback.
- `egui-wgpu/src/renderer.rs:694-706` — the zero-size-clip skip: advances the index *and* vertex
  slice iterators by hand; `:714-735` the per-mesh draw (`set_vertex_buffer(0, …)` at `:728`,
  `draw_indexed` at `:734`).
- `egui-wgpu/src/renderer.rs:1761-1771` — the vertex layout:
  `entry_point: Some("vs_main")`, `array_stride: 5 * 4`,
  `vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32]`.
- `egui-wgpu/src/renderer.rs:1642-1698` — `OutputEncoding::of(format, color_space)`, the total table;
  `:1630-1632` `is_float_target`.
- `egui-wgpu/src/renderer.rs:1796-1805` — entry-point selection
  (`fs_main_linear_framebuffer` vs `fs_main_gamma_framebuffer`).
- `egui-wgpu/src/renderer.rs:1359-1403` — vertex staging: one contiguous buffer,
  `bytemuck::cast_slice(&mesh.vertices)`, per-mesh byte ranges (the `SlicedBuffer` at `:207-211`).
- `egui-wgpu/src/egui.wgsl:1-5` — `VertexOutput{tex_coord, color, position}` (the `color` comment
  still says "gamma 0-1" and needs the tier's "may exceed 1.0" in 6a);
  `:60-70` `unpack_color`; `:79-90` `vs_main`; `:169-212` the two fragment entry points
  (`DITHER_CAPABLE` is an existing spliced constant — the precedent for compile-time *branching*;
  note the tier deliberately does **not** follow it, see the pipeline-matrix bullet below).
- `egui-wgpu/src/egui_transfer.wgsl` — transfer fns shared by production and the round-trip test.
- `egui-wgpu/src/renderer.rs:1452-1570` — `PairingRejection` for tagged float content (textures).
- `egui-wgpu/src/surface_policy.rs:328-380` — pairing walk; `is_u8` `:348`, `is_f16` `:361`.
- `egui_glow/src/painter.rs:224-252` — VAO layout: `a_srgba` = 4 × `UNSIGNED_BYTE` at
  `offset_of!(Vertex, color)` (`:250`), stride `size_of::<Vertex>()`; `:483-491` raw `cast_slice`
  upload.
- `egui_glow/src/shader/vertex.glsl:22-36` — `v_rgba_in_gamma = a_srgba / 255.0` (`:34`).
- `egui_glow/src/shader/fragment.glsl:46-57` — gamma-space multiply (`:50`) + dither.

**Testing / capture**

- `egui_kittest/src/texture_to_image.rs:16-30` — `Rgba16Float` readback returning floats.
- `egui-wgpu/src/capture.rs:91,207-215,271-320` — per-format texel size, fp16 decode, u8-path
  refusal to guess.
- `egui_kittest/src/wgpu.rs:92-101,158-186` — `with_render_options` (format override, MSAA samples).
- `epaint/src/stats.rs:93-95,192-230` — vertex byte accounting (`AllocInfo`, `PaintStats::add`).

**Public HDR surface that must not fragment**

- `epaint/src/image.rs:427-436` — `Primaries { Srgb, DisplayP3 }`; `:470-479` `FloatImage`.
- `egui/src/widgets/color_picker.rs:860-900` — `HdrColor { rgb: [f32;3] linear, exposure_stops }`.
- `egui/src/ui.rs:2258-2277` — `color_edit_button_hdr`, `swatch_hdr`.

---

## 5. Sizing budget (measured on this branch)

`cargo run -p epaint --example size_probe` (throwaway probe, removed) on aarch64 macOS. Re-run
2026-11-02 for the *proposed* layouts as well as the shipped ones — the proposed columns are the
ones that decide §8, and one early claim did not survive the re-run (consequence 3 below). The
layout probe was re-run a third time in the review pass with a standalone `rustc` file that mirrors
`Color32`/`HdrVertexColor`/`ColorMode`/`HdrColorMode`/`PathStroke`/`PathShape`/`TextShape`/`Mesh`/
`Shape` field-for-field; every number in both tables below reproduced, and `HdrColorMode` and the
`Shape::Hdr` payload are now measured rows rather than inferences from `ColorMode`.

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
| `HdrColorMode` (`{Solid(HdrVertexColor), UV(Arc<dyn Fn…>)}`) | 16 | — (the same niche layout as `ColorMode`; measured, not inferred) |
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
   `ColorMode` as **full at 16 B** and do not build a design on a future re-packing.
   24 B `ColorMode` → `PathStroke` 32 → `PathShape` 64 → `Shape` past its guard. So HDR path
   colour does **not** ride `ColorMode`; it rides the `Shape::Hdr` wrapper, whose 24 B payload fits
   under `PathShape`'s 56 B ceiling (§8.3). This is a measured reversal of the §5-era assumption,
   not a preference.
4. **`TextShape` has room for exactly one `Option<HdrVertexColor>`** (48 → 56, measured: the 10-byte
   `Option` lands in the struct's 4 bytes of tail padding plus 6 new bytes), and `Shape` stays 64
   because `PathShape` sets the payload ceiling. There is no room for a *second* such field — the
   next text-only HDR datum takes the boxed/`add_hdr` fallback (§8.6).
5. **`Stroke` stays 8 B** → `Shape::LineSegment` stays SDR. HDR segments are meshes or wrapped
   shapes.
6. Every PR that touches `Shape` sub-structs must keep `shape_size` green, and must add a guard for
   anything new that is size-sensitive (`PathStroke`, `ColorMode`, `Shape::Hdr`'s payload,
   `TextShape`, `Mesh`'s field set). The guard is a `#[test]`, not a `const` assert
   (`shapes/shape.rs:80-91`), so it runs in the normal test job.

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
  majority; touches every `Vertex` literal (13 sites, the §4 list), the `unity` variant, `Vertex`'s serde shape,
  and any external code that constructs `Vertex` by literal (breaking change).
- **Note:** storing the field as `[u16; 4]` (f16 *bits*) rather than `[half::f16; 4]` preserves
  `Eq` and the serde derive with no new trait surface — the main plan §11 "`Eq` drop" concern
  dissolves for this design too. The memory cost is the deal-breaker, not the traits.

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
- **For:** zero regression for every existing app; the main plan §11 ripple list collapses to `Mesh`
  (one new `Option` field, `Eq` preserved since `[u16;4]` is `Eq`), the wgpu renderer, and one WGSL entry
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
  second derived-quantity enum next to `OutputEncoding` (`renderer.rs:1634-1652`), same
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
| f16-as-superset (A2) | one layout, **one** pipeline | blocked by glow, and for a different reason than first written: not `unpackHalf2x16` (unnecessary — drivers convert `GL_HALF_FLOAT` attributes to float), but **`GL_HALF_FLOAT` is not a legal `glVertexAttribPointer` type on GL 1.20 / ES 1.00**, which `egui_glow` explicitly supports (`egui_glow/src/shader_version.rs:9-21,33-68`). It would also need a per-frame CPU conversion of every vertex on the GL backend |
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
| **A** dual field in `Vertex` | 28 (+40%) | 28 | 13 sites + `unity` | untouched | changed | none | 2 (or 1 with A′) |
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
  "pre-E decision" cluster from main plan §11 (`Eq` drop, f16 serde repr, `half` feature surface) is
  **mooted**, not answered. Record that as a win.
- `Mesh` gains `#[cfg_attr(feature = "serde", serde(default))] hdr_colors:
  Option<Vec<HdrVertexColor>>` — the *gated* form every optional serde attribute in this tree uses
  (`tessellator.rs:665`, `egui/src/style.rs:243`, …); a bare `#[serde(default)]` does not compile without
  the feature. `Mesh16` has no derives at all (`mesh.rs:337-348`), so it takes the field and no
  serde obligation. `TextShape` **is** serde-derived (`text_shape.rs:11-12`), so its new
  `hdr_color` field takes the same gated `serde(default)` form (§8.6) — the only HDR field that
  crosses a serialization boundary. Old serialized meshes load as SDR (the correct semantic). New
  meshes read by old code lose the HDR tier — acceptable, and documented.
- `HdrVertexColor` stores `[u16; 4]` (f16 bit patterns) so `Eq`/`PartialEq`/serde derive without a
  NaN caveat: bit equality is the right semantics for a *render list* (a different NaN payload is a
  different byte pattern, and `-0.0 ≠ 0.0` is harmless). This is a deliberate deviation from
  `FloatImage`, which keeps `[half::f16; 4]` at rest and only turns it into bits at the serde
  boundary (`image.rs:746-800`): `FloatImage` is `PartialEq`-only, `Mesh` is `Eq`, and keeping
  `Mesh: Eq` is the cheaper half of the main plan §11 trade. `half` stays an internal conversion detail.
  Document the representation in the type's rustdoc, and provide the accessors below so no user
  handles bits.
- The `unity` cfg variant needs no change at all (it only reorders `Vertex`), which is a second
  reason to prefer B: the "packing trick must work in both field orders" problem disappears.

---

## 8. API surface

**Two notions, not two mechanisms.** The tier appears at two levels, and they are layers of one
design, not competing conventions:

| Level | Type | What it says | Existing SDR counterpart |
|---|---|---|---|
| Storage (data plane) | `Mesh::hdr_colors` | *these vertices* carry HDR values | `Vertex::color` |
| Operator (shape level) | `Shape::Hdr { inner, color }` | paint this already-tessellated geometry in this HDR colour, using its **coverage** | `RectShape::fill` / `PathStroke::color` |
| Source (per-shape) | `HdrColorMode::{Solid, UV}` | how the operator's colour is evaluated per vertex | `ColorMode::{Solid, UV}` |
| Field (text only) | `TextShape::hdr_color` | the glyph colour of this text run | `TextShape::override_text_color` |

The operator lowers *into* the storage: `Shape::Hdr` writes the sidecar for the vertices it
recolours (§9). That is the same relationship `ColorMode` has with the mesh it tessellates, and the
same relationship `RectShape::fill` has with the vertices `RectShape` emits — one vocabulary at the
shape level, one storage at the vertex level. What is deliberately *not* added is a second storage
(a field on `Vertex`) or a second path-colour enum inside tessellation (`ColorMode::Hdr*`): §5
rules both out by measurement, and §8.3 records the reversal. The narrowing that keeps this honest:
`Shape::Hdr` is an **operator over coverage** and never the way to carry per-vertex HDR data a caller
already has — that is `Mesh::hdr_colors` + a hand-built mesh, and the operator's rustdoc says so.

### 8.1 `epaint::HdrVertexColor` (new, `epaint/src/hdr_color.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]   // wire form *is* the bits: `[u16; 4]`
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
- `projected_color32(&self) -> Color32` — the SDR twin: the legal-premultiplied range reduction the
  GL backend draws and the HDR mesh's unused `Vertex::color` carries (§8.5). Total on every input,
  including transparent and negative values.
- `from_linear([f32; 4]) -> Self` — the ergonomic constructor. Input is **straight (unmultiplied)**
  linear RGBA: each component is encoded with `ecolor::gamma_from_linear` (odd-symmetric since PR 2a)
  and then premultiplied, `stored = a × enc(L)`; `a` is the coverage and is not encoded. Say so in
  the rustdoc — "linear" alone does not settle premultiplication, and the two conventions differ for
  every translucent value.
- `impl From<Color32>` — the lift, a per-component `/255` rescale exactly as
  `FloatImage::from_color_image` (`image.rs:634-648`), not a transfer-function round trip. Test:
  absolute error `< 1/512` (< half an 8-bit step) for all 256 values per component. A `From` impl
  rather than an inherent `from_color32`, matching `impl From<Color32> for Rgba`.
- `TRANSPARENT` (`[0.0; 4]`), `WHITE`, `from_white_alpha(f32)` — the `Color32` const/method set,
  names included (`TRANSPARENT` at `color32.rs:60`, `from_white_alpha` at `:171`). `ZERO`/`from_alpha`
  would be a second vocabulary for the same two values.
- `From<HdrColor>` lives in `egui` (epaint must not depend on the picker type; `HdrColor` is an
  egui type) and goes through `HdrColor::effective_linear` → `from_linear([r, g, b, 1.0])`.
  `effective_linear` clamps only the *exposure* to ±6 stops and scales `rgb` by `2^stops`
  (`color_picker.rs:887-892`); nothing is per-channel clamped on the way in, so the ratio is
  preserved and alpha is exactly 1 (`HdrColor` is opaque by design).
- `impl From<Rgba> for HdrVertexColor` lives in **epaint**, next to `impl From<Color32>`: both
  `From` and `Rgba` are foreign to `egui`, so an impl there would violate the orphan rule and not
  compile. (Rgba's components are linear *and* premultiplied — `Rgba::from_rgba_premultiplied`'s
  convention — so the impl encodes in place, unlike `from_linear`.)

There is deliberately no way to detect "is this `HdrVertexColor` also a `Color32`": the tier is a
property of the *mesh*, never of a value, which is what keeps the SDR path free.

### 8.2 `Mesh` / `Mesh16`

- `hdr_colors: Option<Vec<HdrVertexColor>>` on both (serde-gated, §7).
- `is_hdr(&self) -> bool`, and the length invariant lives in **`Mesh::is_valid`**
  (`mesh.rs:99-107`) / `Mesh16::is_valid` (`:352-358`): `hdr_colors.is_none() ||
  hdr_colors.len() == vertices.len()`. That is the existing hook — `append`, `append_ref`,
  `split_to_u16` and `Shape::mesh` already `debug_assert!(mesh.is_valid())`, so one added clause
  covers every mutation path that matters and no new "is_valid-adjacent" convention is invented.
  The renderer's staging loop asserts it too (it is the last place a desync would silently read
  out of bounds of the sidecar buffer).
- `colored_vertex_hdr(pos, color: HdrVertexColor)`, `add_colored_rect_hdr(rect, color)`,
  `add_rect_with_uv_hdr(rect, uv, color)` — the last one builds the four sidecar entries alongside
  the existing four `Vertex` literals (`mesh.rs:199-227`); the other two delegate to it the way
  `add_colored_rect` delegates today (`mesh.rs:231-237`).
- **Tier mixing (`append`/`append_ref`):** the merge never silently drops the HDR side, and it
  never needs to: an SDR `Color32` lifts to `HdrVertexColor::from(Color32)` within f16's worst-case
  relative error (~2^-11 ≈ 4.9e-4, well under half an 8-bit step — the `< 1/512` assertion §8.1
  carries), and nothing in the merge is clamped. `promote_to_hdr()` is public and idempotent.
  The two entry points differ in what they can mutate, and the doc must say so because
  `append_ref(&mut self, other: &Self)` (`mesh.rs:147`) cannot touch `other`:
  - `append(self, other: Self)` — promote whichever side is SDR (owned, so either is writable).
  - `append_ref(&mut self, other: &Self)` — if `other.is_hdr()` and `self` is SDR, `self.promote_to_hdr()`
    then extend; if `self` is HDR and `other` is SDR, lift `other`'s colours **into the extension
    iterator** (no mutation of `other`, no temporary `Vec`).
- `split_to_u16` carries the sidecar through both the fast path and the split path (`mesh.rs:243-306`);
  `bytes_used` counts it (`:92-96`); `clear` clears it (`:85-90`); `AllocInfo::from_mesh` counts it so
  the perf numbers do not lie (§4); and `translate`/`transform`/`rotate` are colour-free and
  unchanged.
- `Mesh::append_ref`'s existing "same texture" assert is unchanged. The tessellator's grouping key
  (`tessellator.rs:1518-1535`) has two halves: the texture half is `Shape::texture_id()`, which
  `Shape::Hdr` must delegate (§4), and the tier half is `Shape::is_hdr()` (§9).

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
// epaint/src/color.rs, next to `ColorMode` — the same two-variant niche layout; measured 16 B (§5)
pub enum HdrColorMode {
    /// One HDR value for the whole shape.
    Solid(HdrVertexColor),

    /// Provide a callback which takes in the shape's bounding box and a position and converts it
    /// to an HDR value. The bounding box has the same feathering margin `ColorMode::UV` gets
    /// (`color.rs:16-19`).
    UV(Arc<dyn Fn(Rect, Pos2) -> HdrVertexColor + Send + Sync>),
}
```

- `Clone`, `Debug` and `PartialEq` follow `ColorMode`'s treatment (`color.rs:9,32-50`): hand-written
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
(§5). HDR *fills* are expressed as meshes, or as a wrapped shape:

- **`Shape::gradient_rect_hdr(rect, direction, [HdrVertexColor; 2])`** — the exact mirror of the
  existing `Shape::gradient_rect` (`epaint/src/shapes/shape.rs:329-347`), which is already a
  four-vertex mesh. Same `Direction` semantics, same corner assignment, plus the sidecar. Shipping a
  differently-named `gradient_quad` instead would be inventing a second API for a solved problem.
- **`Painter::rect_filled_hdr(rect, corner_radius, color)`** — `add_hdr(RectShape::filled(rect,
  cr, Color32::WHITE), color)`, i.e. the wrapper of the bullet below: interior vertices are
  `(1,1,1,1)` and take the colour, feather vertices are transparent and stay transparent, and the
  component-wise multiply of two premultiplied encoded pairs is exactly what the fragment shader
  does with a texel. This inherits rounded corners, stroke kinds, blur, brushes and every future
  `RectShape` feature instead of duplicating corner tessellation — with the caveat that a brush's
  texels keep multiplying *on top of* the HDR tint (§8.4's operator note).
- **`Painter::add_hdr(shape: impl Into<Shape>, color: impl Into<HdrColorMode>) -> ShapeIdx`** — the
  generic escape hatch behind the parity map in §8.7. **Mechanism: a `Shape` wrapper, not eager
  tessellation.** The painter does not tessellate, and there is no public "tessellate one shape"
  API, so the wrapper is what makes one code path cover every shape:

  ```rust
  // 24 B payload (Arc<Shape> 8 + HdrColorMode 16) — under PathShape's 56 B ceiling, so Shape stays 64
  Shape::Hdr { inner: Arc<Shape>, color: HdrColorMode }   // §8.3 for the mode
  ```

  with one tessellator arm (§9): record the vertex cursor, tessellate `inner` into `out`, then
  `out.promote_to_hdr()` and for the vertices it just emitted set
  `hdr = color(pos) × (vertex.alpha / 255)` (sidecar) and
  `vertex.color = hdr.projected_color32()` (the glow twin, §8.5). `color(pos)` is the solid value or
  the callback evaluated on `inner.visual_bounding_rect().expand(feathering)`, where `feathering` is
  `TessellationOptions::feathering_size_in_pixels` on the tessellator that is running the arm. That
  is the same construction as the path tessellator's `uv_bbox` (`tessellator.rs:1111-1116`: path
  bounds + half the stroke width + feathering) but *not* bit-identical for `StrokeKind::Inside`/
  `Outside` strokes, because `PathShape::visual_bounding_rect` (`path_shape.rs:69-75`) uses the
  unshifted points while `uv_bbox` uses the stroke-shifted ones; the difference is ±`width/2` on the
  rect the callback sees, and it is stated rather than papered over.

  **What the operator does to colour, exactly.** It replaces the *vertex colour* and keeps everything
  else: UVs, `texture_id` and the geometry are untouched, so textured content keeps its texels and the
  wrapper acts as an **HDR tint** (fragment: `in.color * tex_gamma`). For untextured geometry the
  white texture makes that tantamount to a replace. **The alpha channel is the coverage mask** —
  combined with the HDR value's own alpha — which is why feathering, rounded corners, antialiasing and
  glyph edges keep working with no new geometry code.

  Consequences to document, and to test: a shape's fill and stroke collapse to one colour (uniform
  recolour), and an inner `ColorMode::UV` ramp is lost — a per-position ramp needs
  `HdrColorMode::UV`, `gradient_rect_hdr` (6a) or a hand-built mesh (6b).

  **Clipping and transforms need no code.** Clipping is a renderer scissor over the primitive's
  `clip_rect`; the tessellator only uses `self.clip_rect` for culling, so a wrapped shape participates
  exactly like an unwrapped one. `tessellate_clipped_shape`'s `transform_after_tessellation` branch
  moves vertices *after* the arm runs (`tessellator.rs:1545-1556`), and the sidecar holds colours, so
  it needs no transform. `Shape::transform` on the wrapper transforms `inner` and leaves the mode
  alone: like `ColorMode::UV`, the callback is defined against the shape's *recomputed* bounding box,
  so the ramp moves with the shape instead of shearing.

  The constructor **flattens `Shape::Vec` at construction**, because `tessellate_shape`'s own
  contract says it can panic for `Vec`/`Callback` (`tessellator.rs:1564-1565`; the comment's `Vec`
  half is itself stale — the body loops children into one mesh, which is exactly the texture hazard
  below) and its mesh-level `Vec` branch loops children into one mesh, tripping the texture
  accumulator's `assert_eq!(out.texture_id, …)` (`tessellator.rs:954-957`) the moment two children
  use different textures. `Shape::hdr(inner, color)` therefore maps over `Vec` children recursively (so each child
  keeps its own primitive split and its own tier decision, §9), and passes `Shape::Callback` through
  uncoloured — a callback owns its pipeline, so there is nothing to recolour and the honest fallback
  is uncoloured, not a rewrite of the callback's semantics. The `Callback` case **cannot** be handled
  inside `tessellate_shape` (that function cannot emit a `Primitive::Callback` and panics on one), so
  `tessellate_clipped_shape` intercepts `Shape::Hdr` next to its existing `Shape::Callback` fast path
  (`tessellator.rs:1509-1516`) and unwraps it there: a `Callback` inner is emitted uncoloured with a
  `debug_assert`, and a `Vec` inner is split into one `Hdr`-wrapped `ClippedShape` per child and fed
  back through the same interception. Both halves are needed: unwrapping only the top level lets the
  `Hdr`-over-`Vec` arm re-wrap a `Callback` child in `Hdr`, which walks straight into the
  `tessellate_shape` callback panic — the one panic the wrapper promises a public variant does not
  cause. The constructor's pass-through is the ergonomic path; the interception is the
  contract for directly-constructed variants. Constructing `Shape::Hdr` twice collapses to the outer
  colour (the inner wrapper's colour is already baked into its alpha mask); document that, and
  collapse it in the constructor.

  The variant needs arms in every exhaustive `match Shape`: `shape_transform::adjust_colors`
  (**recurse into `inner` only** — the HDR mode must *not* be dimmed, because the inner alpha is the
  single dimming path, §9), `visual_bounding_rect` (`shape.rs:403-431`, delegate to `inner`),
  `transform` (`:467`+, transform `inner`), `PaintStats::add` (`stats.rs:192-230`, delegate — the
  sidecar is counted once, by the mesh's `AllocInfo`), `tessellate_shape`
  (`tessellator.rs:1568`+), `should_parallelize` (`:2535-2552`, recurse) — and a **delegating
  `texture_id()` arm** (`shape.rs:437-445`), which is *not* an exhaustive match and therefore will not
  be flagged by the compiler (§4).
- **`Painter::add_hdr_uv(shape, callback)`** (6b) — `add_hdr(shape, HdrColorMode::UV(Arc::new(callback)))`,
  taking the closure directly the way `PathStroke::new_uv` does (`stroke.rs:151-161`), so a ramp call
  site never spells `Arc` either.
- `Painter::rect_filled_hdr` is then one line:
  `add_hdr(RectShape::filled(rect, cr, Color32::WHITE), color)` — the fill color supplies alpha only.
- A `Shape` variant carrying a *sized* HDR fill payload is still **not** proposed: the wrapper above
  is 24 bytes, while a per-shape HDR field would need headroom these structs do not have (§5). The
  `Arc<Shape>` is mandatory, not a size trick — `Shape` cannot contain itself inline — so the wrapper
  costs one allocation per wrapped shape per frame, exactly like `Shape::Mesh`'s `Arc<Mesh>`. Boxing
  the *whole* wrapper (`Shape::Hdr(Arc<HdrShape>)`) would shrink the payload 24 → 8 B and still
  allocate, saving nothing under `PathShape`'s 56 B ceiling; it is not worth it until a second
  HDR-only field appears.

### 8.5 The u8 twin (`Vertex::color` on an HDR mesh)

The wgpu HDR path reads `@location(3)` only, so `Vertex::color`'s four bytes are unread *there*.
The tessellator still writes them, as the **legal-premultiplied-`Color32` projection** of the HDR
color. Those bytes are the GL backend's entire rendering of an HDR mesh (§11): `egui_glow` reads
`a_srgba` at `offset_of!(Vertex, color)` (`painter.rs:250`) and its shader multiplies it in gamma
space (`shader/vertex.glsl:22-36`, `shader/fragment.glsl:46-57`), so the twin is a shipped consumer,
not dormant data — which is what keeps it inside the stack's "nothing dead" binding rule instead of
being a projection nobody ever looks at.

**The projection scales in linear light; it never clamps per channel, and it is total.** It is
`HdrVertexColor::projected_color32(&self) -> Color32` (§8.1), a pure function of the stored bits
that both the tessellator (§9) and any test can call:

```rust
let [r, g, b, a] = self.to_array();          // stored, encoded, premultiplied
if a <= 0.0 { return Color32::TRANSPARENT; } // feather rings and cleared text land here
let lin = [r / a, g / a, b / a];             // straight light, or alpha contaminates the maximum
let peak = lin.iter().fold(0.0_f32, |m, c| m.max(*c));   // ignore negatives: SDR cannot show them
if peak <= 0.0 {                             // all-negative or exact black: black, not invisible
    return Color32::from_rgba_premultiplied(0, 0, 0, linear_u8_from_linear_f32(a));
}
let k = 1.0 / peak;
let scaled = lin.map(|c| (c * k).max(0.0));  // uniform scale, then drop unrepresentable negatives
// encode each straight component, *then* multiply by `a` (alpha is unencoded coverage) and
// quantize — the order `Color32`'s own conversion uses (`ecolor/src/color32.rs:147-151`), which is
// exactly what keeps `rgb <= a`
```

Three guards, each with a reason, and each is a test (§12):

- **`a <= 0`** — the wrapper writes a fully transparent sidecar on every feather vertex (§9), so the
  formula is evaluated on them on every antialiased edge. Without the guard it divides by zero and
  produces NaN that poisons the `Color32` quantizer. It is not a finiteness test, but none is
  needed: a hand-built bit pattern with `a` = NaN leaves every `f32::max` returning its non-NaN
  operand, so `peak` is `0.0` and the branch below yields alpha `0`; `a` = `+inf` makes `lin` zeros
  and the same branch fires at the saturating alpha `255`. Nothing non-finite reaches the
  quantizer.
- **`peak <= 0`** — the §3 contract is odd-symmetric, so an all-negative (or black) value is legal.
  `max` over negatives would return a negative peak, `k` would be negative, and the "scaled" colour
  would flip sign (a −4.0 red would display as white, not black). The branch returns **premultiplied
  black at the incoming alpha**, not `TRANSPARENT`: sub-black and exact black both mean "as dark as
  SDR goes", and an opaque one must still occlude. `TRANSPARENT` here — the round-2 draft — made
  such geometry vanish on the GL backend instead, contradicting §11's "the geometry is still in
  place" (§8.5's own "would display as … black" already said what the intent was).
- **`.max(0.0)` after the scale** — mixed-sign values (`[+4.0, −1.0, 0.2]`) have *no* positive `k`
  that puts every component in `0..=1`; the uniform scale is applied to the positives (their ratios
  are preserved exactly, which is the property that matters) and negative components drop to black
  rather than being folded in.

A uniform scale of the positive tristimulus *is* a luminance change — every inter-channel ratio among
the positive components is preserved, so hue and saturation are untouched by construction.
Per-channel clamping alters the ratios (`[6.0, 2.0, 0.5] → [1.0, 1.0, 0.5]`: a 3:1 red:green ratio
becomes 1:1, a large hue shift) and, in the encoded domain, compounds the error with the transfer
curve. Two independent mistakes, and neither is acceptable in a value the GL backend displays *today*
(§11).

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

- `TextShape::hdr_color: Option<HdrVertexColor>` (serde-gated with the same `serde(default)` form as
  `Mesh::hdr_colors`, because `TextShape` *is* serde-derived, `text_shape.rs:11-12`). When set, it
  replaces the *glyph* colour of the whole shape — the same scope as `override_text_color`
  (`text_shape.rs:30-33`), i.e. **only vertices inside `glyph_vertex_range`** — and deliberately
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
  (`ui.rs:1828-1832`: colour first, then text). **Where the value is stored matters**, because
  `RichText::color` reaches the text through the *layout job* (`TextFormat.color`) and that road is
  `Color32`-bound (§8.6's next bullet): `color_hdr` therefore stores
  `RichText::hdr_color: Option<HdrVertexColor>` and `Label` reads it from its `WidgetText` **before
  the layout job consumes it** (`layout_in_ui` moves `self.text` into `into_layout_job` at
  `label.rs:183`; `Label` does not call `WidgetText::into_galley`), then applies it to the
  `TextShape` it builds at `label.rs:318-320` via `TextShape::with_hdr_color`. `hdr_label` is the
  supported ergonomic; in 6c only `Label` (and therefore `ui.label`/`ui.hdr_label`) learns the
  field — `Button`, `Heading`, `SelectableLabel` and `TextEdit` keep SDR text until they opt in,
  which is stated in the rustdoc rather than implied.
  The `hdr_text` texture-cache prototype from the main plan §6 is retired by this PR.
- **The tessellator writes a sidecar entry for *every* vertex it emits from `tessellate_text`, not
  just the glyphs**, because the tier is a property of the mesh and `is_valid` demands
  `hdr_colors.len() == vertices.len()`. Two vertex sources exist in that one `out`:
  - **glyph vertices** (`glyph_vertex_range`): sidecar = `hdr_color × (vertex.color.a() / 255)`.
    The vertex alpha is the mask — it carries `opacity_factor`, the galley's own alpha, and the
    selection-clear write (§4, `label_text_selection.rs:286-305`) — so hiding a vertex also hides its
    HDR value. The *glyph coverage* is the font atlas' texel alpha, applied by the fragment's
    `in.color * tex_gamma`; the two are different things and both are needed.
  - **everything else** (the underline emitted by `tessellate_line_segment` at
    `tessellator.rs:2364-2370`, and any galley vertices outside the glyph range): sidecar =
    `HdrVertexColor::from(vertex.color)`, the exact SDR lift — so underlines and backgrounds still
    render, at their own colours, instead of disappearing or breaking the invariant.
  The three special cases at `tessellator.rs:2328-2358` map as: `override_text_color` → the HDR
  value wins for glyph vertices (it is the stronger override), `PLACEHOLDER` → the twin keeps using
  `fallback_color` and the sidecar is the lift of that, color-glyph anti-tint
  (`from_white_alpha(color.a())`) → alpha-only HDR value (`from_white_alpha`), i.e. colour glyphs
  keep their own pixels and only their coverage drives the HDR tint.

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
| `Painter::image` / `Shape::image(texture, rect, uv, tint)` | `add_hdr(Shape::image(…, Color32::WHITE), color)` — an HDR **tint**: the texels keep multiplying underneath (`in.color * tex_gamma`), so this is tinting, not flat recolouring | 6a |
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

PR 5's HDR widgets render HDR color as **float textures** because at the time main plan §6 forbade HDR
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
- The comments in `color_picker.rs` that cite the main plan's §6 are superseded, in 6d, by the above
  split — the inline note ("the float-texture path is the one §6-compliant way to show the true
  color") and `HdrColor`'s own rustdoc claim that "vertex colors cap at 1.0 (plan §6 — stroked shapes
  and text cannot be HDR)" (`color_picker.rs:847-849`). Both are now false statements about the
  shipped tree and must go in the same PR that makes them false.

---

## 9. Tessellator integration

The *storage* is a property of a **mesh**, never of a vertex, so the tessellator has exactly two
producers and no colour threading (the text field is a source that feeds one of them):

- **`Shape::Hdr`** (§8.4) — one new arm in `tessellate_shape` (`tessellator.rs:1568`). Record
  `let first = out.vertices.len()`, tessellate `inner` (everything except `Vec`/`Callback` → the
  existing arms; a `Vec` that reaches the arm is mapped to one `Hdr` per non-`Callback` child rather
  than tessellated into `out` — the constructor already flattens and `tessellate_clipped_shape` now
  splits a nested `Vec` before the arm (§8.4), but the variant is public and the arm has to stay
  total without tripping the texture assert; a `Callback` child cannot be emitted from
  `tessellate_shape` at all, so the arm skips one with a `debug_assert!(false, …)` — the interception
  is the path that paints it), then `out.promote_to_hdr()` — the prefix can be SDR on any call that
  reaches the arm with a non-empty `out`, which is a direct `tessellate_shape` call *and* the rayon
  path, which tessellates a whole `should_parallelize`d shape into one mesh (`tessellator.rs:2564`)
  before the grouping key ever sees it; promotion is idempotent when it is not needed — and finally
  for `out.vertices[first..]`:
  `a = vertex.color.a() / 255`, `hdr = source(rect, pos) × a`, write the sidecar entry, and overwrite
  `vertex.color` with `hdr.projected_color32()` (§8.5). `rect` is
  `inner.visual_bounding_rect().expand(feathering_size_in_pixels)` — the same construction as
  `ColorMode::UV`'s `uv_bbox` (`color.rs:16-19`), with the `StrokeKind` caveat §8.4 states. No
  `get_color` arm, no `mul_color` arm, no `PathStroke` field.
- **`TextShape::hdr_color`** (§8.6) — `tessellate_text` (`:2228`) writes a sidecar entry for **every**
  vertex it emits when the field is set: glyph vertices inside `glyph_vertex_range` get
  `hdr_color × (vertex.color.a() / 255)`, everything else (the underline, non-glyph galley vertices)
  gets `HdrVertexColor::from(vertex.color)`; same projection for the twin, and `out.promote_to_hdr()`
  first for the same reason as the wrapper's arm (the rayon path can hand this function an `out` that
  already holds SDR vertices). The invariant is
  `hdr_colors.len() == vertices.len()` on every path, including the underline.

  **Promote-first covers the prefix, not the whole mesh.** Nothing yet covered raw vertices appended
  *after* an HDR-producing arm (the wrapper's or the text's). `tessellate_shape`'s `Shape::Vec` arm
  (`tessellator.rs:1571-1575`) loops children into one `out`, and the rayon wholesale path
  (`tessellator.rs:2555-2572`) runs that arm
  *before* `tessellate_clipped_shape` ever applies the tier split — so
  `Shape::Vec([Text { hdr_color: Some(..), .. }, Shape::circle_filled(..), …])` with more than four
  children (or with any `should_parallelize` child, e.g. a 33-point path) is tessellated into **one**
  mesh, and the circle's vertices are pushed directly, never through `Mesh::append_ref`, so they
  carry no sidecar entry. `Circle`, `Ellipse`, `Path`, `Rect`, `Band`, `LineSegment`, the beziers and
  the debug text rect all emit that way. The **`tessellate_shape` call therefore repairs the tail on
  its way out**: after its `match`, if `out.hdr_colors` is `Some` and shorter than `out.vertices`,
  extend it with `HdrVertexColor::from(vertex.color)` for the missing tail. Put it at the call
  boundary rather than in the `Vec` arm alone, because `tessellate_shape` is public and can be
  handed an already-HDR `out` directly; the boundary rule covers the wholesale path, the `Vec` arm
  and direct callers with one clause. It is idempotent, a no-op wherever the tier split already gave
  the mesh a single tier, and it is what makes the length invariant true on the wholesale path
  instead of merely stated. Test it through the wholesale call (a `Vec` with an HDR text first and a
  vertex-pushing shape after it), not only through `tessellate_clipped_shape`.

Consequences:

- **Opacity/fade dim HDR content exactly once — and `adjust_colors` is where that happens.**
  `Painter::transform_shape` calls `multiply_opacity` / `tint_shape_towards`
  (`egui/src/painter.rs:201-207`, `:555-569`), which land in `shape_transform::adjust_colors`
  (`shape_transform.rs:9-130`) *before* tessellation. That function's single `adjust_color` closure is
  `Color32`-valued, so the HDR side needs a **second, HDR-valued adjuster**:
  `adjust_colors(shape, adjust_color, adjust_hdr)` — one traversal, two value spaces. Adding the
  parameter is a breaking change to a public epaint fn, already covered by 6a's `CHANGELOG` entry
  (`Mesh` gained a field, `Shape` gained a variant). `multiply_opacity` passes "multiply all four
  components" for both; `tint_shape_towards` passes `tint_color_towards`
  (`ecolor/src/lib.rs:279-300`) for `Color32` and, for stored HDR values, its encoded-space mirror
  toward `HdrVertexColor::from(target)`: **the same three branches `tint_color_towards` takes**,
  applied to the stored premultiplied components with `target` scaled into `0..=1`. So an opaque
  stored value (`a >= 170`) blends encoded RGB and keeps its alpha, and only below that threshold
  does alpha halve. Reproducing the branches is not optional: "always halve alpha" would dim an
  opaque HDR mesh on wgpu while its `Color32` twin on glow stayed opaque — a backend divergence the
  fade path must not introduce.
  Three rules, one per level — the asymmetry is the point, and a test pins each:
  - `Shape::Hdr` recurses into `inner` and **does not touch the mode**: the wrapper's mask is the
    inner alpha, so dimming the mode as well would apply the factor twice. The HDR colour is dimmed
    once, in encoded space, which is the §3 contract. Consequence of the alpha-only mask, stated
    because `fade_to_color` accepts any target: only the alpha half of a fade reaches wrapped HDR
    content — the wrapper has already discarded the inner RGB, so its hue is never tinted — and
    `tint_color_towards` leaves alpha alone at `a >= 170` (`ecolor/src/lib.rs:279-300`), so an
    RGB-only fade dims the wrapper's SDR neighbours and not the wrapper. Fades to transparent (the
    animation case) are exact: `Painter::add` drops the shape outright when
    `fade_to_color == Some(Color32::TRANSPARENT)` (`egui/src/painter.rs:214-216`), and
    `TextShape::opacity_factor` is a four-component multiply that the mask follows too.
  - `Shape::Mesh` applies `adjust_hdr` to every `hdr_colors` entry — otherwise a directly-built HDR
    mesh would ignore `Ui::multiply_opacity` and `fade_to_color` entirely on wgpu, because the only
    channel the `Color32` fold reaches is the *glow* twin. Opacity is a component multiply and needs
    no target; a fade blends toward `HdrVertexColor::from(target)` in encoded space (above).
  - `Shape::Text` **must not** apply `adjust_hdr` to `hdr_color`, and this is the one rule of the
    three that is easy to get wrong in the "do the same thing everywhere" direction. The text
    sidecar is not stored before tessellation — it is *derived* as
    `hdr_color × (vertex.color.a() / 255)` (§8.6) — and that alpha is exactly what the fold writes:
    the galley rows' vertex colours (`shape_transform.rs:103-111`) and, separately,
    `opacity_factor` (`tessellator.rs:2344-2346`). Dimming the field as well applies the factor
    twice (opacity `0.5` → colour `0.25`), so the `TextShape` arm leaves `hdr_color` untouched and
    the vertex alpha is the single dimming path, exactly as it is for the wrapper.
- **Feathering** is the alpha-0 outer ring the tessellator already emits; the wrapper writes a
  transparent sidecar there (`a == 0`, handled by §8.5's guard), and the encoded-space multiply keeps
  premultiplication exact.
- **"One mesh per (texture, tier)" is enforced where the tessellator already enforces one mesh per
  texture.** `tessellate_clipped_shape` starts a new primitive when
  `output_mesh.texture_id != shape.texture_id()` (`tessellator.rs:1524`); that condition gains
  `|| output_mesh.is_hdr() != shape.is_hdr()`. Without it, an HDR shape appended after SDR ones
  promotes the whole accumulated primitive (§8.2) — correct, but it would make §6.3's "sparse HDR
  costs proportionally nothing" claim false, because the merge granularity is the contiguous
  same-texture run and nearly every shape shares the white texture. With it, the tier is part of the
  batching key and the claim holds.

  `Shape::is_hdr()` is the predicate, and it must be **complete** — one method, three arms, or
  hand-built meshes silently miss the key and promote their SDR neighbours:

  ```rust
  pub fn is_hdr(&self) -> bool {
      match self {
          Self::Hdr { .. } => true,                                  // the operator
          Self::Text(text) => text.hdr_color.is_some(),              // the text field
          Self::Mesh(mesh) => mesh.is_hdr(),                         // sidecar built by hand
          _ => false,
      }
  }
  ```

  It is also the thing a `PaintCallback` can ask (§16 question 5).
- **A mesh is never mixed — by the tier split; the call boundary repairs instead.**
  `Mesh::append`/`append_ref` promote (§8.2), and `tessellate_clipped_shape` splits by tier before a
  shape is tessellated, so the normal path never mixes. The paths that can mix are the rayon
  wholesale pass (which calls `tessellate_shape` directly, before the split) and any direct caller
  of the public `tessellate_shape`, which is why that call repairs the tail (above).
- **Transforms:** `transform_after_tessellation` moves vertices after the arm (§8.4); the sidecar is
  colour-only. `Shape::transform` transforms `inner` and leaves the mode alone.
- **Clipping** is the renderer's scissor over `clip_rect`; the tessellator uses `clip_rect` only for
  culling, so neither the operator nor the text field needs a clip arm.
- Anti-aliased `ColorMode::UV` gradients still clamp at 1.0; `HdrColorMode::UV` does not, because the
  ramp is evaluated on the HDR side of the wrapper.

---

## 10. Primaries semantics for vertex colors

**Decision: vertices are surface-referred, like `Color32` — not tagged.** Rationale:

- `Color32` chrome already means "values in the surface's granted primaries"; making vertices
  tagged would create two contracts for the same enum-shaped content and a new rejection path
  (`Primaries` pairing, `renderer.rs:1452-1570`) that would fire on *geometry* — a much larger
  blast radius than on images.
- The main plan's §5 chrome-shift analysis (saturated accents 2.2–6.8 CIEDE2000) therefore applies to HDR
  vertex colors unchanged: on a P3 surface, an `HdrVertexColor::from_linear([0.9, 0.1, 0.1])` built
  for sRGB shows shifted. This is the same behavior as the surrounding UI, which is the point.
- **The asymmetry must be documented loudly:** tagged `FloatImage` content pairs strictly and
  rejects; untagged vertex content never rejects and never converts. Apps wanting exact primaries
  convert app-side (the main plan's §5 recipe) — identical for vertices and textures.
- Tagged vertex content (`HdrVertexColor` + `Primaries`) stays demand-gated with the main plan's §5
  matrices; the mechanism (enum variant + pairing row + matrix) is already planned, so this defers
  nothing new.

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
  that mirrors the vertex one (`:1359-1403`) and pushed **in lockstep — one slice per `Primitive::Mesh`,
  including the SDR meshes with an empty range**. That last part is load-bearing: the vertex staging
  loop only allocates when `vertex_count > 0` and pushes one slice per mesh, so a sidecar loop that
  skipped SDR meshes would offset every later mesh's colour by the number of SDR meshes before it.
  `set_vertex_buffer(0, ..)` gains slot 1 on HDR meshes (`:728`); an SDR pipeline ignores the stale
  slot-1 binding, so no unbind is needed.
- **No batching layer exists to key on.** `render()` draws one `draw_indexed` per `Primitive::Mesh`
  in paint order (`:714-735`) and sets the pipeline exactly once per pass (`:686`), from a lookup
  performed up front (`:652-661`). So the tier is honoured the only way this renderer can honour any
  state change: `render_pass.set_pipeline(..)` again when the tier differs from the previous mesh —
  a state switch, **never a reorder**. Batching by tier would reorder egui's primitives, and
  primitive order *is* paint order for translucent HDR content; that is a correctness regression, not
  an optimization. The cache key (`:305`) gains the tier component, so the up-front lookup can no
  longer resolve *one* handle: `render()` keeps one resolved pipeline per tier and looks the second
  one up on the first mesh of that tier. An HDR-only frame has no SDR entry in the cache at all, so
  keeping the single up-front `.expect(…)` would panic on exactly the frames the tier exists for.
- **The tier's pipeline must exist before `render()`, and only when the frame needs it.** Pipelines
  are created in `update_buffers` (`ensure_pipeline`, `:597-624`, called at `:1262`); `render()`
  takes `&self` and no device, so it can only look up and `.expect(…)`. The signature therefore becomes
  `ensure_pipeline(device, target_format, tier)` and `update_buffers` calls it **once per tier present
  in that frame**. That is a reordering, not a new pass: the fold that counts vertices
  (`:1294-1314`) already walks every `paint_jobs` entry, but the `ensure_pipeline` call sits *above*
  it (`:1262`), so the fold moves up, records which tiers occur, and the call follows it — one walk
  plus one `contains_key` per tier present. This is what keeps the non-negotiable intact: an
  SDR-only frame compiles and caches exactly the pipeline it does today, and the HDR variant costs a
  second `contains_key` + one creation only on the first frame that actually has HDR content. (An
  earlier draft created both tiers unconditionally; that is a first-frame cost every existing app
  would pay for a feature it does not use — rejected here.)
- The zero-size-clip skip (`:694-706`) advances the index and vertex slice iterators by hand; it must
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
  (`float_content_warned: HashSet<TextureId>`, `painter.rs:101`) because textures persist and own
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
| CPU contract | `HdrVertexColor`: `From<Color32>` error `< 1/512`; `from_rgba_premultiplied` ↔ `to_array` round-trip **on f16-representable inputs** (arbitrary `f32` cannot round-trip through 11 bits, and a test that expects it to would encode a lie); negatives preserved; `>1.0` preserved; premultiplication invariant under feathering multiply; `from_linear` premultiplies after encoding | f16 rounding and odd-symmetry at the negative branch are exactly where PR 2a found a real bug, and "linear" without a premultiplication convention is where every translucent value goes wrong |
| CPU structure | `size_of::<Shape>() == 64` still passes; `PathStroke`/`ColorMode`/`Vertex` asserted **unchanged** (24/16/20); new guards for `Shape::Hdr`'s payload and `TextShape` (56); the `hdr_colors.len()` invariant checked through `Mesh::is_valid`; `promote_to_hdr` idempotence; `split_to_u16` carries the tier | silent size regressions and forgotten mutation paths |
| CPU projection | the u8 twin preserves chromaticity (positive channel ratios intact within quantization), never emits `rgb > a`, and `promote_to_hdr` ↔ twin is stable across a round-trip; **`a == 0` returns `TRANSPARENT`, while an all-negative or exact-black value returns premultiplied black at its own alpha** (so an opaque one still occludes — §8.5's guards; the feather ring evaluates the first on every antialiased edge) | per-channel clamping is the tempting wrong implementation, and it is silently plausible; the divide-by-zero and the sign flip are *not* plausible on inspection, which is why they are asserted |
| CPU wrapper | `Shape::Hdr` around a textured mesh starts its own primitive (the `texture_id` delegation, §8.4) instead of panicking in `append_ref`; a wrapped `Shape::Vec` tessellates child-by-child; a wrapped `Shape::Callback` passes through uncoloured; a **directly constructed** `Hdr{inner: Callback}` also passes through (the `tessellate_clipped_shape` interception, §8.4) instead of hitting `tessellate_shape`'s panic, and so does a callback nested inside `Hdr{inner: Vec([…])}`; and a `Vec` of `[Hdr{..}, circle_filled(..)]` run through the rayon wholesale call keeps a full-length sidecar | `Shape::texture_id()` is the one `Shape` site the compiler cannot point at (§4), and the callback path is the one the constructor hides |
| CPU batching | a hand-built HDR mesh appended after an SDR shape starts its own primitive: `Shape::is_hdr()` returns true for `Shape::Mesh` with a sidecar (§9), so the SDR mesh is not promoted and keeps 20 B/vertex | the predicate is the only thing standing between "sparse HDR" and "every SDR vertex becomes HDR" |
| CPU opacity | `multiply_opacity` / `tint_shape_towards` over (a) a `Shape::Hdr` wrapper, (b) a bare HDR mesh, (c) HDR text: the wrapper dims once (inner alpha only, mode untouched), a bare HDR mesh dims its sidecar, and HDR text dims **not via `hdr_color`** but once through the galley vertex alpha the sidecar multiplies in (dimming both is the double-dim this row exists to catch) | §9's rule is asymmetric per level, so an implementation that "does the same thing everywhere" is wrong in one of the three cases |
| CPU text | `TextShape::hdr_color` with an underline and with a background-coloured galley: sidecar length equals vertex count, the underline and the background still draw, only `glyph_vertex_range` vertices take the HDR value, and a vertex cleared to `TRANSPARENT` (the selection path, §4) yields a zero sidecar; **and a `Shape::Vec` with an HDR text *first* and a vertex-pushing shape after it, run through the rayon wholesale call, ends with a full-length sidecar** (the `tessellate_shape` tail repair, §9) | the underline is emitted by a *different* call inside the same `out`, and the wholesale path lets *other* shapes append after it — exactly what the invariant catches |
| GPU numeric | kittest fp16 readback: mesh at 4.0 reads 4.0; at −2.0 reads −2.0; alpha-feathered edge midpoint; a `Shape::Hdr` gradient (`HdrColorMode::UV`) sampled at the midpoint of 1.0→6.0 | the whole point — u8 snapshots cannot see >1.0 |
| GPU numeric (variant) | same, MSAA 4× on fp16 | resolved-multisample fp16 readback is the PR 3 trap |
| Snapshot | HDR gradient + HDR text PNGs: silhouette, AA, and geometry only (values above 1.0 clip in a u8 PNG by definition); SDR snapshots must be **byte-identical** | a regression on the SDR path is the one unacceptable outcome |
| Serde | `Mesh` round-trip with and without the sidecar; old payload → SDR; `HdrVertexColor` serializes as its `[u16; 4]` bits (`serde(transparent)`, §8.1); `TextShape` round-trip with and without `hdr_color`, and an old `TextShape` payload still loads; `Shape` round-trips **without** an HDR mode at all (`Shape` is not serde-derived) | the `cfg_attr(serde(default))` form on the two new fields, and the old-payload paths |
| glow | SDR snapshot byte-identical; an HDR mesh draws its **twin** and logs one warning per painter (asserted through the same `log` capture PR 2b's reject test uses) | a backend that degrades must say so once, not silently |
| Perf | demo app vertex bytes/frame before/after via `stats`; SDR frame path asserted unchanged; the pipeline cache holds **one** entry after an SDR-only frame (§11, §13) | the claim that makes B acceptable, and the lazy-tier rule is the only thing keeping the SDR first frame cost identical |

The GPU numeric and snapshot tests live in `crates/egui_kittest/tests/hdr_vertex_colors.rs`, next to
the existing per-concern files (`fp16_target.rs`, `snapshot_colorspace.rs`,
`wgpu_paint_callback.rs`), not inside `screenshot.rs`.

Every numeric test carries the value through *both* fragment entry points that the platform can
grant (gamma-encoded fp16 and, on CI headless fixtures, the linear path) — the same discipline PR 2a
applied to the GPU round-trip.

---

## 13. Performance budget

- **SDR:** 20 B/vertex, one staging buffer, one *cached* pipeline per `(format, encoding, Sdr)` —
  unchanged in count and in bytes. The key gains a component (§11) but an SDR-only frame still
  creates exactly one pipeline, on the same first use, and never touches the HDR one.
  Assert it: a test that stages a mesh with `hdr_colors: None` must produce the same byte count and
  the same slice count as before the change, and the cache must hold exactly one entry — keyed
  `(format, encoding, Sdr)` — with no HDR pipeline created.
- **HDR meshes:** 28 B/vertex total (20 + 8) and a second traffic stream. The one CPU cost is the
  twin (§8.5): per emitted HDR vertex, at tessellation, a reciprocal, a max-reduce over three
  channels, a scale, and three `ecolor::gamma_u8_from_linear_f32` + quantize — paid only by meshes that asked for
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
| **6a** | `epaint::HdrVertexColor` (incl. `projected_color32`); `Mesh`/`Mesh16` sidecar + the `is_valid` length invariant + promote + serde; `Shape::Hdr` wrapper with `HdrColorMode::Solid`, `Painter::add_hdr`, the `Shape::is_hdr()` predicate and the `tessellate_clipped_shape` callback interception; `shape_transform::adjust_colors`' HDR parameter + arms (mesh sidecar, text field, wrapper recursion); `Shape::gradient_rect_hdr`, `Painter::rect_filled_hdr`, the `tessellate_shape` tail repair (the length invariant on the wholesale path); wgpu layout variant + `vs_main_hdr` + cache key + **lazy per-tier `ensure_pipeline`** + second `SlicedBuffer` (one slice per mesh, SDR included) + per-mesh pipeline switch; glow draws the twin with a warn-once; `CHANGELOG` entries in `crates/epaint`, `crates/egui-wgpu`, `crates/egui_glow` and the root `CHANGELOG.md` (including epaint's `⚠️ Breaking changes`: `Mesh` gained a field, `Shape` gained a variant) | raw HDR geometry in any eframe app; the picker's flat swatches (§8.8) | CPU contract + twin projection, size guards (incl. `PathStroke`/`ColorMode` unchanged), wrapper/`texture_id` test, fp16 numeric readback (4.0 / −2.0), MSAA variant, serde, SDR-byte-identical, glow twin + warn-once, parity (`gradient_rect_hdr` equals `gradient_rect` for SDR-equivalent colors) |
| **6b** | `HdrColorMode::UV` + `Painter::add_hdr_uv` (positional HDR ramps on any wrapped shape); `Painter::arrow_hdr`; the stroke/polyline/band/plot-trace story that `add_hdr` already carries gets its snapshots and its documented density limit; no `PathStroke`, `ColorMode`, `Stroke` or `Shape::LineSegment` change | antialiased HDR strokes, polylines, plot traces, band gradients | ramp sampling at the 1.0→6.0 midpoint, feathering through the mask, snapshot of an HDR trace, SDR path identical |
| **6c** | `TextShape::hdr_color` (+ `serde(default)`), the `tessellate_text` sidecar for **all** emitted vertices (glyph range HDR, underline/background lifted), `RichText::hdr_color` read by `Label` before layout, `ui.hdr_label`; the three text special cases; epaint `CHANGELOG` (TextShape gained a field — a breaking change for struct-literal users); retire the demo's `hdr_text` prototype | HDR labels/readouts as first-class content | numeric readback of glyph cores >1.0, color-glyph anti-tint, SDR text snapshots identical |
| **6d** | picker re-wiring (§8.8: flat swatches off float textures, `HDR_TEXTURE_CACHE_CAP` narrowed to ramps, the main-plan-§6 comments superseded), `MANUAL_TESTING.md` HDR section, perf write-up, demo `hdr_test_app` migrated off texture-strips | adoption + evidence for upstream | picker snapshots unchanged; full-suite |

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
10. **"Why two mechanisms (`Mesh::hdr_colors` and `Shape::Hdr`)?"** They are a storage and an
   operator, not two spellings of one thing (§8's table): the sidecar is where per-vertex HDR lives
   (the only place it can live without growing `Vertex`), and the wrapper is how an arbitrary shape
   gets HDR colour without a per-shape field. The wrapper *writes* the sidecar, so there is one
   storage; §8.3 measured why the third option (`ColorMode::Hdr*`) is not available.
11. **"Why does HDR text not go through `add_hdr` like everything else?"** Because the wrapper's mask
   is alpha, and a text mesh's background and underline share the mesh; `TextShape::hdr_color` is
   glyph-range-only (§8.6), which is the same shape as `override_text_color`. The trade is stated:
   one HDR value per text shape, mixed `RichText` runs lose their variation.
12. **"`Ui::multiply_opacity` silently ignores HDR?** It must not, and §9 says exactly where the fold
    lives (`shape_transform::adjust_colors`) and which of the three levels dims what — the wrapper
    dims once via its mask, a bare HDR mesh dims its own sidecar, and HDR text dims through the
    vertex alpha its sidecar is multiplied by (dimming its `hdr_color` as well would apply the factor
    twice). This is the finding that turned a "no HDR-space code needed" claim into one explicit rule
    per level and one test per level. Disabled greying enters at exactly one level and needs no
    fourth rule: `Ui::disable` multiplies the painter's opacity (§4, `ui.rs:525-528`), so it *is*
    the `multiply_opacity` path above, while `Visuals::disable`/`gray_out` (`style.rs:1288-1297`)
    are `Color32`-only colour-selection helpers applied before any shape exists — they never touch
    an HDR value, so there is no second greying to double-count.

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
   pipelines; note only that they can read `Mesh::is_hdr()`. Wrapping a callback in `Shape::Hdr` is
   defined, not undefined: it passes through uncoloured (§8.4), because the alternative would be a
   panic in `tessellate_shape` on a public variant.
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
  GL 1.20 / ES 1.00 (`egui_glow/src/shader_version.rs:9-21,33-68`), not any WGSL/`unpackHalf2x16`
  limitation.
- The main plan §11 "`Eq` cascade `Vertex → Mesh → Shape`" was overstated: `Shape` derives
  `PartialEq` only (`shapes/shape.rs:25-27`) because `ColorMode::UV` holds a closure whose
  `PartialEq` returns `false` for `UV == UV` (`color.rs:37-46`). Only `Mesh`'s `Eq` is at stake.

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
   *switch* when the tier changes, never by reordering; and the zero-size-clip skip (`:694-706`) must
   advance a third slice iterator.
3. **glow's warn-once cannot key on a mesh** (§11, §8.5) — every existing warn is once per
   `TextureId` (`float_content_warned`, `painter.rs:101`) because textures own ids; meshes do not.
   The GL path therefore renders the u8 twin (which makes the twin a shipped consumer rather than
   dormant data) and warns once per painter through a `bool`. That also resolved an internal
   contradiction: §8.5 already called glow the twin's consumer while §11 skipped the mesh.

Two smaller items from the same pass: naming now follows `ecolor`'s value-type vocabulary
(`from_rgba_premultiplied`/`to_array`/`TRANSPARENT`/`from_white_alpha`, `impl From<Color32>`), and the
`sized` claims that were "measure before committing" are measured (§5: `Option<HdrVertexColor>` is
10 B and `TextShape` lands at 56, `Shape` unchanged; the wrapper payload is 24 B).

**C.4 — review of C.3 (third pass, 2026-11-02).** The second pass was re-derived from the tree rather
than trusted. It was right about the measurements (both §5 tables reproduced exactly, including
`HdrColorMode` = 16 B and the `Shape::Hdr` payload = 24 B) and about the reversal itself; it was
incomplete or wrong in eight places, and each is now fixed where it bites:

1. **`Shape::is_hdr()` was incomplete** (§9). It named `Shape::Hdr` and `Shape::Text` but not
   `Shape::Mesh` with a sidecar, so a mesh built with `Mesh::colored_vertex_hdr` — the documented
   low-level path — would miss the tessellator's batching key and promote the whole accumulated
   SDR primitive to +40% bytes. The predicate is now spelled out with all three arms and has its own
   test.
2. **Opacity/fade did not reach HDR at all** (§9). C.3's "opacity/fade keep working with no
   HDR-space code" was true only for the wrapper (which reads inner alpha). For a bare HDR mesh and
   for HDR text the channel the fold touches (`vertex.color`) is the *glow* twin, so on wgpu
   `Ui::multiply_opacity` was a no-op. `shape_transform::adjust_colors` now gains an HDR-valued
   adjuster next to its `Color32` one (one traversal, two value spaces), and the asymmetry (wrapper:
   inner only; mesh: its own values; text: the vertex alpha only) is stated and tested. (The last
   clause is C.5's correction: this item first had text dimming `hdr_color` too, which double-dims.)
3. **The twin projection was not total** (§8.5). It divided by `a` and took `max` over possibly
   negative components: `a == 0` (every feather vertex) produced NaN, and an all-negative
   (odd-symmetric, legal) value produced a negative scale and inverted the colour. Both are now
   guards, with the mixed-sign rule stated.
4. **`tessellate_text` has more than glyphs** (§8.6). The underline is emitted by a
   `tessellate_line_segment` call into the same `out`, and galley meshes can carry non-glyph
   vertices; a sidecar written only for "emitted glyph vertices" breaks `hdr_colors.len() ==
   vertices.len()` and would blank the underline on wgpu. The rule is now: every emitted vertex gets
   a sidecar entry, glyph-range ones HDR, the rest the SDR lift. The glyph mask is the *vertex* alpha
   (which is what the selection-clear path writes), while glyph coverage is the atlas texel.
5. **`impl From<Rgba> for HdrVertexColor` "in egui" does not compile** (§8.1). Both `From` and `Rgba`
   are foreign to `egui`; the impl moved to epaint, next to `impl From<Color32>`.
6. **§11 and §13 contradicted each other on pipeline count** (§11, §13). §11 had `ensure_pipeline`
   create both tiers for every `(format, encoding)`, while §13 promised the SDR pipeline count was
   unchanged and the non-negotiable is that the SDR path pays nothing. `update_buffers` already has
   the paint jobs, so the pipeline is now created per tier present in the frame; an SDR frame creates
   exactly what it creates today.
7. **`Shape::Hdr` over `Shape::Callback` would panic** (§8.4). `tessellate_shape` cannot emit a
   `Primitive::Callback` (that is its documented panic), and C.3's pass-through-in-the-arm was
   therefore unimplementable for a directly-constructed variant. The interception moved to
   `tessellate_clipped_shape`, next to the existing callback fast path.
8. **Anchors and housekeeping.** `egui/src/visuals.rs` does not exist (the override mechanism is
   `TextShape::override_text_color`); the `shape_size` guard is `shapes/shape.rs:80-91` (this item
   itself had it wrong first — see C.5); the glow fragment shader's multiply is `:46-57`, not `:56-70` and the file is 58 lines;
   bare `painter.rs` anchors are now qualified; `Vertex` literal sites are 13, not 12;
   `TextShape` is serde-derived so its new field needs the gated `serde(default)`;
   `append_ref` takes `&Self`, so "promote the SDR side" had to be split per entry point; and the
   wrapper's `Arc<Shape>` is mandatory rather than an optional 16 B saving.

Two claims were re-checked and deliberately kept, so the next reviewer does not re-open them:
`HdrColorMode` as a sibling of `ColorMode` (measured 16 B; generalising buys nothing, C.3's argument
holds), and the `Shape::Hdr` wrapper as the path for path colour (the measured `ColorMode` growth is
disqualifying, and the wrapper is the same mechanism every other shape uses — §8's storage/operator
table is the layering answer, not a second convention).

**C.5 — fixed-point check of C.4 (fourth pass, 2026-11-02).** C.4's edits were re-read as a fresh
reader, against the tree rather than against C.4's own account. Its seven findings were real and the
mechanisms mostly stand; four of the *edits themselves* were wrong, one gap was left, and five
anchors introduced or re-corrected by those edits were stale. Recorded because a fix that is trusted
without re-derivation is exactly how round 2's defects got in.

1. **Opacity, rule 2 was inverted** (§9). C.4 item 2 added `adjust_hdr` for `Shape::Text`, which
   double-dims: the text sidecar is not stored pre-tessellation, it is *derived* as
   `hdr_color × vertex.alpha`, and that alpha is exactly what the `Color32` fold already multiplies —
   the galley rows (`shape_transform.rs:103-111`) and, separately, `opacity_factor`
   (`tessellator.rs:2344-2346`). Opacity `0.5` became `0.25`. Rule 2 is now **Mesh-only**; for text
   the vertex alpha is the single dimming path, the same rule as the wrapper.
2. **The projection's component order** (§8.5). The pseudo-code comment prescribed
   "re-premultiply by `a`, encode each component", which breaks the `rgb <= a` legality asserted by
   the same line: encoding *brightens*, so `enc(a·c) > a` for `a < 1`. §3's own contract
   (`stored = a × enc(L)`) and `Color32`'s own conversion (`color32.rs:147-151`) encode first, then
   scale. Comment corrected; the arithmetic is unchanged.
3. **`peak <= 0` made opaque geometry vanish** (§8.5, §12). `TRANSPARENT` (C.4 item 3) is safe
   against the sign flip but contradicts the guard's own stated reason ("would display as … black")
   and §11's "the geometry is still in place": an opaque sub-black value became see-through on the GL
   backend. It now returns premultiplied black at the incoming alpha (`rgb = 0 <= a`, still legal);
   `a == 0` keeps `TRANSPARENT`.
4. **The interception missed a nested `Vec`** (§8.4). Intercepting only `Hdr { inner: Callback }`
   leaves `Hdr { inner: Vec([… Callback …]) }` live: the arm's per-child mapping re-wraps the callback
   in `Hdr` and walks into `tessellate_shape`'s panic — the one panic §16 Q5 promises away. The
   interception now both emits a `Callback` inner uncoloured **and** splits a `Vec` inner, feeding the
   children back through itself.
5. **`ensure_pipeline`'s "same pass" was not the same pass** (§11). The fold C.4 pointed at
   (`:1294-1314`) runs *after* the `ensure_pipeline` call (`:1262`), so the tier scan cannot join it
   without moving the call. The text now calls it a reordering and names the move; the "SDR frame
   creates one pipeline" claim is unchanged and still holds.
6. **`tessellate_text` needs the wrapper's promote-first** (§9). §9 asserted the length invariant "on
   every path" but only the wrapper's arm promoted `out`. A `Shape::Vec` that `should_parallelize`
   accepts is tessellated *wholesale* into one mesh (`tessellator.rs:2564`), so the text arm can be
   handed a mesh that already holds SDR vertices. One clause added; the same sentence now also
   justifies the wrapper's own promote.
7. **Anchors.** C.4 item 8 *changed a correct anchor to a wrong one* — `shape_size` is
   `shapes/shape.rs:80-91`, not `:61-72` — so the correction is itself corrected. Five more were
   stale or bare: the shader multiply is `:172,201`, not `:180-200` (the file is 211 lines);
   `style.rs:243` is `egui/src/style.rs:243` (`epaint` has no `style.rs`); GL 1.20 / ES 1.00 support
   is proven by the `ShaderVersion` enum and its parse (`shader_version.rs:9-21,33-68`), not by
   `:24-43`; `ColorMode`'s manual `PartialEq` is `color.rs:37-46`, not `:44-57`; and the grouping key
   is `tessellator.rs:1518-1536`, not `:1512-1535`.
8. **Section references.** Eight bare `§4` / `§5` / `§2` citations in §3, §1.3 and §10 meant the *main
   plan's* sections while this document's §4 is the code map and its §5 the sizing budget; they are
   now qualified. §0's contract row said "§4" where the answer is this document's §3, and §3's
   opening said "Adopt §4" for what is the main plan's contract.

Re-verified and unchanged: the three `is_hdr` arms — with a second customer for the `Mesh` arm, since
`parallel_tessellation_of_large_shapes` replaces any tessellated shape with `Shape::Mesh`
(`tessellator.rs:2571`), so a rayon HDR path arrives at the grouping key *as a mesh*;
`adjust_colors` really is public API (`epaint/src/lib.rs:41`), so the semver claim stands;
`impl From<Rgba> for HdrVertexColor` in epaint satisfies the orphan rule (local self type; `Rgba`
reaches epaint through `ecolor`, `lib.rs:79`); the per-tier `ensure_pipeline` leaves an SDR-only
frame's cache at exactly one entry; and the text coverage rule is complete *for every vertex
`tessellate_text` itself emits* — `glyph_vertex_range` (`text_layout.rs:1232`) plus row backgrounds
before it (`:1199`) and row underline/strikethrough after it (`:1208,1217`), plus the shape-level
`TextShape::underline` that `tessellate_line_segment` emits into the same `out`
(`tessellator.rs:2364-2370`). (Completeness *across* producers is a different question, and C.6
found it open.) Color glyphs and `TextShape::angle` (italics)
change which colour a glyph vertex gets and where it lands, never how many vertices exist, so the
`hdr_colors.len() == vertices.len()` rule is unaffected by either.

**C.6 — fixed-point check of C.5 (fifth pass, 2026-11-02).** C.5 was re-derived from the tree rather
than read as an account of itself. Its mechanism corrections (C.5 items 1–6) hold and are not
re-opened. The fixed point had still not been reached: one vertex path was uncovered, one
prescription was wrong for the common case, one panic was left reachable, and the anchors needed
correcting again.

1. **The sidecar had an uncovered *tail*** (§9). C.5 item 6 added promote-first, which covers
   vertices that existed *before* `tessellate_text` runs. It does not cover vertices appended
   *after*: `tessellate_shape`'s `Shape::Vec` arm (`tessellator.rs:1571-1575`) loops children into one
   `out`, `parallel_tessellation_of_large_shapes` (`tessellator.rs:2555-2572`) runs that arm *before*
   `tessellate_clipped_shape` applies the tier split, and every non-mesh producer (`Circle`,
   `Ellipse`, `Path`, `Rect`, `Band`, `LineSegment`, the beziers, the debug text rect) pushes
   vertices directly instead of through `Mesh::append_ref`. So
   `Shape::Vec([Text { hdr_color: Some(..), .. }, circle_filled(..), …])` with five children (or any
   `should_parallelize` child) is tessellated wholesale into one mesh and leaves
   `hdr_colors.len() < vertices.len()` — an invalid mesh, caught by `tessellate`'s final
   `debug_assert!(mesh.is_valid())` and by the renderer's staging assert. Fixed by making the
   call boundary repair the tail (exact lifts, idempotent, no-op on the tier-split path) and by
   testing through the wholesale call. 6a's wrapper arm has the same tail for a wholesale
   `Vec([Hdr{..}, circle_filled(..)])`, and a direct `tessellate_shape(circle, hdr_mesh)` has it
   too — which is why the repair sits at the call boundary rather than in the `Vec` arm.
2. **The fade mirror prescribed "halve alpha"** (§9). `tint_color_towards` leaves alpha alone at
   `a >= 170` (`ecolor/src/lib.rs:279-300`); following the prescription would have dimmed an opaque
   HDR mesh on wgpu while its `Color32` twin on glow stayed opaque — a divergence on the one path
   (`fade_to_color`) that has a twin to diverge from. The text now requires the three branches
   themselves, with `target` scaled into `0..=1`.
3. **`render()`'s single up-front lookup was still implied** (§11). With the tier in the cache key,
   an HDR-only frame has no SDR entry, so the existing `.expect(…)` (`renderer.rs:652-660`) would
   panic on exactly the frames the tier exists for. Stated: one resolved handle per tier, the second
   looked up on the first mesh of that tier.
4. **Anchors, again.** `shape_size` is `shapes/shape.rs:80-91` (C.5's own 80-94 still overran into
   the `Send + Sync` const check); `Color32`'s encode-then-premultiply order is
   `color32.rs:147-151`, not `:86-92` (colour constants); vertex staging is `renderer.rs:1359-1403`
   (not `:551-552`, the buffer-start capacities, or `:1373-1410`, which begins on the blank line
   after the resize and runs past the staging loop into the callback-prepare block);
   `OutputEncoding` and `of` are `:1642-1698`, with the "derived, never stored" rule at
   `:1634-1652`; the zero-clip skip is `:694-706`; `Mesh::is_valid` is `:99-107`, `split_to_u16`
   `:243-306`, `Mesh16::is_valid` `:352-358`, `bytes_used` `:92-96`, `AllocInfo` `:14-21`;
   `from_color_image` is `image.rs:634-648`; `Stroke` is `stroke.rs:13-16`;
   `color_edit_button_hdr`/`swatch_hdr` are `ui.rs:2258-2277`; `tint_shape_towards`/
   `multiply_opacity` are `painter.rs:555-569` and `transform_shape` `:201-207`; `get_color` is
   `tessellator.rs:1118-1121`; and the panic contract is `:1564-1565` — whose `Vec` half is itself
   stale (the body loops children into one mesh, which is the texture hazard §8.4 relies on).
5. **`Label` does not call `into_galley`** (§8.6). `layout_in_ui` moves `self.text` into
   `WidgetText::into_layout_job` (`label.rs:183`); the read point is before *that*, not at
   `label.rs:280`, which is the `layout_in_ui` call site in `Widget::ui`.
6. **Section references.** Eleven more bare `§11` / `§6` citations that meant the *main plan* were
   qualified (§0, §1.1, §6.1, §6.3, §7, §8.8, §14, C.1) — completing C.5 item 8's sweep, which had
   caught the §4/§5/§2 family but not the design-space and picker sections. Also reworded §13's
   untestable "the same pipeline handle as before the change" into "exactly one cache entry, keyed
   `(format, encoding, Sdr)`".

Re-verified and unchanged: the twin projection's totality on the five cases the review named
(`a == 0` → `TRANSPARENT`; all-negative and exact black → premultiplied black at the incoming alpha;
`[NaN, 1, 1, 1]` → the `f32::max` fold ignores the NaN and the `.max(0.0)` after the scale absorbs
it; `[inf, 1, 1, 1]` → `k = 0`, every component saturates to black, and `inf * 0.0 = NaN` is again
absorbed by `.max(0.0)`; the `a = NaN` / `a = +inf` aside holds because `fast_round`'s `as u8` is a
saturating cast); monotone rounding of `round(c·a·255) ≤ round(a·255)` keeps `rgb <= a` in the main
branch; the three-way opacity rule traces to one dim per level with `Ui::disable`'s
`multiply_opacity(disabled_alpha)` (`ui.rs:525-528`) entering as the opacity path and
`Visuals::disable`/`gray_out` staying `Color32`-only colour selection; the callback interception is
total for `Hdr{Callback}`, `Hdr{Vec[…Callback…]}` and any nesting of either, because `Vec` is the
only shape container and the interception recurses through it; and an SDR-only frame still creates
exactly one pipeline under the moved `ensure_pipeline` call.
