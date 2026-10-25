# egui HDR support — working notes

Working doc for "what must change in egui for native HDR". Accumulates findings from our
investigations and discussions; not an implementation plan. The app-side workaround this all
stems from is documented in [`hdr-egui-port-plan.md`](hdr-egui-port-plan.md) (hand-rolled
winit loop + `egui_wgpu::Renderer` + `Rgba16Float`/`ExtendedSrgb` surface).

All source anchors below were verified against the vendored sources on 2026-09-11:
egui / egui-wgpu / egui-winit / ecolor / epaint **0.36.2**, wgpu **30.0.x**.
Upstream tracker status (checked 2026-09-11): no open egui issue or PR requests HDR;
wgpu 30 shipped the foundation (`SurfaceColorSpace`, `display_hdr_info`) closing
[wgpu#2920](https://github.com/gfx-rs/wgpu/issues/2920). `egui_glow` does not exist at 0.36
— wgpu is the only GPU backend, so there is exactly one backend to fix.

---

## 1. Presentation: eframe can't create an HDR surface (the blocker, small diff)

Three hard-coded decisions, all in `egui-wgpu`:

- **Format choice** — `RenderState::create` (~lib.rs:289) does
  `target_format = preferred_framebuffer_format(&caps.formats)`, which is a `for` loop
  matching `Rgba8Unorm | Bgra8Unorm` (lib.rs:411-427). Metal always offers both, so you can
  never get `Rgba16Float`.
- **Config surface area** — `SurfaceConfig` (lib.rs:76-94) exposes exactly `present_mode` +
  `desired_maximum_frame_latency`. No format, no color space.
- **Color space** — `Painter::configure_surface` (winit.rs:100-154) builds the
  `wgpu::SurfaceConfiguration` with `format: render_state.target_format` and never touches
  `color_space`, so wgpu 30 defaults it to `Auto`. Even if you smuggled a float format in,
  you'd get an SDR-interpreted fp16 surface.

The change: extend `SurfaceConfig` with an HDR *policy*, not raw wgpu knobs:

```rust
pub enum HdrPreference {
    Off,                        // today's behavior, the default
    PreferExtendedSrgb,         // Rgba16Float + SurfaceColorSpace::ExtendedSrgb
}
```

`RenderState::create` probes
`caps.color_spaces(Rgba16Float).contains(SurfaceColorSpaces::EXTENDED_SRGB)`, picks
accordingly or falls back to `Off`, records what it got (`RenderState::hdr_active: bool`),
and `configure_surface` threads the color space through. An `Option<TextureFormat>` escape
hatch for power users is cheap to add beside it.

**Why a policy enum instead of exposing `format` + `color_space` directly:** naive
knob-twiddling lands exactly in issue
[#7484](https://github.com/emilk/egui/issues/7484) territory (pick `ExtendedSrgbLinear` →
double gamma; pick a float format at runtime → black screen). The policy encodes the one
combination that is shader-correct today. That's the difference between an API upstream
will merge and one they won't.

**This PR alone would have eliminated our entire hand-rolled loop.** Everything else we did
— callbacks, tick drawing, headroom label — works fine on top of eframe once the surface is
right. Diff size: ~150 lines in `lib.rs` + `winit.rs`, no shader changes, no `epaint`
changes. It's the scandalously cheap part.

## 2. Output encoding: the shader selection is accidentally correct

`Renderer::new` picks its fragment entry point by `output_color_format.is_srgb()`
(renderer.rs:407-414): `*Srgb` formats get `fs_main_linear_framebuffer`, everything else
`fs_main_gamma_framebuffer`. For `Rgba16Float` (not an `*Srgb` format) this selects the
gamma path — which happens to be *right* for `SurfaceColorSpace::ExtendedSrgb`
(gamma-encoded, extended range) and *wrong* for `ExtendedSrgbLinear`/scRGB, where you'd
want linear output and instead get a gamma curve baked in. The heuristic can't see the
color space, so it can't distinguish them.

Interesting detail from `egui.wgsl`: `fs_main_linear_framebuffer` already does
`linear_from_gamma_rgb(vertex_color × texture)` — precisely the correct shader for scRGB.
So the fix is not new shaders, it's an explicit selection:

```rust
enum OutputEncoding { Gamma, Linearize } // or a richer ColorSpace enum
```

threaded from surface config → `RenderState` → `Renderer::new`, replacing the `is_srgb()`
sniff. Side fix: auto-disable `dithering` for float targets — `dither_interleaved(rgb,
256.0, …)` is meaningless on fp16 (and assumes [0,1]).

Also: the WGSL `gamma_from_linear_rgb` is already unclamped above 1.0, and vertex colors
are u8 anyway — so the SDR UI renders correctly on an HDR surface *by construction* (SDR
content semantics: [0,1] in extended sRGB). No UI washout, no changes to the style system.
This is why HDR is much cheaper for egui than people assume: the UI chrome needs nothing.

## 3. HDR content: egui's data model tops out at u8

The actual ceiling. `epaint::Vertex` carries `Color32` packed as a u32 (renderer.rs:378-382,
`array_stride: 5*4`), and the only image type is `ImageData::Color(ColorImage { pixels:
Vec<Color32> })` (epaint/src/image.rs:16-51) uploaded as `Rgba8Unorm` (renderer.rs:702).
Nothing in egui's normal paint path can express a value > 1.0. That's why our quad is a
`CallbackTrait` shader.

What makes HDR *content* first-class (the piece an HDR image viewer actually needs):

- a float image variant — `ImageData::Float`-style half-float pixels;
- `Renderer::update_texture` gaining an `Rgba16Float` path (currently hardcoded
  `Rgba8Unorm` at renderer.rs:702; `register_native_texture` also documents Rgba8Unorm-only
  at renderer.rs:772-821);
- `TextureOptions`/sampling for float textures, so `ui.image()` displays HDR without a
  custom pipeline.

Self-contained, additive, doesn't disturb the SDR path.

**Theme vs content — the distinction that matters:**

- **Theme/chrome** (window backgrounds, buttons, text, `Visuals`): SDR is *correct*, not a
  compromise. The scrollbar shouldn't bloom at 1000 nits; SDR white is the anchor the
  display's tone mapping is built around. Making the style system float buys nothing and
  costs everything — `Color32` is in every egui API, style files, serde, snapshots.
- **Content widgets** (a color swatch in an HDR tool *is* content, not chrome): an HDR
  editor whose "current color" preview shows a tonemapped SDR lie is broken as a tool. The
  question isn't *whether* these widgets go HDR, it's *which mechanism* carries it
  (→ tier ladder below).

Encouraging: `ecolor`'s scalar transfer functions are already unclamped
(`linear_from_gamma`/`gamma_from_linear`, "works for numbers outside this range",
ecolor:157-179) — only the u8 conversions clamp (ecolor:112-131). And `ecolor::Rgba` is
f32 linear with no internal clamping. The color *math* is HDR-ready; the *plumbing* isn't.

## 4. Lifecycle and info APIs

- **Runtime switching** ([#7484](https://github.com/emilk/egui/issues/7484)):
  `SurfaceConfig` is documented "edit any field to have the surface reconfigured on next
  paint" — but a format change invalidates baked pipeline targets. Upstream needs either
  Renderer recreation on `target_format` change (detect in the paint path) or HDR declared
  create-time-only via `NativeOptions`. Given our findings that the safe recreate is
  context+state+renderer together, push for create-time-only plus an explicit
  `Painter::set_hdr()` that does the full recreate.
- **Display transitions** (dragging between SDR/HDR monitors): needs surface recreate at
  minimum; should fall out of the above.
- **Headroom reporting**: expose `display_hdr_info().tone_map_headroom()` through
  `RenderState`/`frame.info()` so apps can build UI like our orange marker without
  reaching into wgpu. Trivial, main-thread-only (Metal reads `NSScreen`) — eframe already
  lives there. Note EDR headroom ramps over ~1-2 s on macOS → poll per frame +
  `request_repaint_after` when idle.
- **Capture/kittest**: the `capture` feature reads back Rgba8; float targets need a
  conversion step or snapshot tests of HDR swatches will lie.

---

## 5. The invasiveness ladder (tiers A–E)

Ordered by how deep you cut into egui. Mental model: **invasiveness = which crates you
edit**, and egui's stack is layered `ecolor`/`emath` → `epaint` (tessellation) → `egui`
(widgets) → `egui-wgpu`/`eframe` (backends). The deeper the layer, the more code above it
ripples.

| | Tier | Crates touched | ~Size | What it unlocks |
|---|---|---|---|---|
| **A** | None — app-side | *none* | — | Custom shader rects (paint callbacks) |
| **B** | HDR surface creation | `egui-wgpu` only | ~150 lines | An HDR *window*; callbacks in any eframe app |
| **C** | Float textures | `epaint` + `egui` + `egui-wgpu` | medium, additive | HDR *images*, swatches-as-textures |
| **D** | HDR color type & widgets | `egui` + `ecolor` | small-medium | First-class HDR picker/swatch widgets |
| **E** | Float vertex colors | `epaint` core + **every** backend | large | *Any* mesh HDR — the complete answer |

**A — no egui changes (what we have now).** The only HDR-capable channel today is
`egui_wgpu::CallbackTrait`: your own pipeline, your own shader, writing >1.0 values.
Unlocks exactly one shape of app: "HDR content in a rect the app hand-paints" — our test
quad. Locked: everything widget-shaped. Also wgpu-specific; no sanctioned way to slip a
float texture in (`register_native_texture` documents Rgba8Unorm-only).

**B — let eframe create the surface (the keystone).** = §1 above. One crate, two files.
Unlocks the *presentation* layer: any eframe app runs on an extended-range swapchain, so
anything that can write >1.0 (i.e. callbacks) displays as actual HDR, with correct SDR UI
alongside. This single tier deletes our entire hand-rolled winit loop. Locked: no egui
widget can *produce* an HDR value — `Color32` and `Vertex` still cap at 1.0.

**C — float content through textures.** First cut into egui's data model, purely additive:
float variant in `epaint::ImageData`, `Rgba16Float` path in `Renderer::update_texture`,
float-capable `TextureHandle`. Unlocks: `ui.image()` displaying HDR bitmaps (PQ AVIF →
extended-sRGB floats, no tonemap), and any widget whose visual is a rectangle of color —
because a color swatch is a 1×1 float texture, a gradient strip is an N×1 float texture,
the picker's SV square is a W×H float texture. All cacheable, uploaded only when the
edited color changes. An app can build an HDR picker out of this tier alone. Locked:
vector-shaped HDR (AA'd strokes, curves, meshes), and ergonomics are DIY.

**D — HDR color type and widgets (UX assembly).** `ecolor::Rgba` is already unclamped
f32-linear — so this is mostly `egui`-crate widget work: `color_edit_button_hdr(&mut
Rgba)`, swatch widgets, exposure/stops sliders (the slider in our test app), nits
readouts, reusing tier C textures for the visuals. `&mut Rgba` state is linear f32 =
scene-referred scRGB, arguably the *right* edit space for HDR. Unlocks: HDR editing UI as
a library feature. Locked: still nothing mesh-shaped — a tier D swatch is internally a
texture; `rect_filled` with an HDR color remains impossible.

**E — float colors in the tessellator (the full monty).** `epaint::Vertex` color u32 →
f16×4 (8 bytes, stride 20→24; the display chain is fp16 end-to-end on Metal anyway, so
zero precision loss vs the surface itself), tessellator emits them, `_hdr` painter
methods, then ripple through **every consumer of the mesh**: egui-wgpu pipelines, the
software renderer, kittest snapshot comparison, `capture` readback, mesh serde. Unlocks:
everything paintable is potentially HDR. Theme stays `Color32` by choice, not limitation.

**Reading the ladder:**

- Invasiveness is superlinear at E. A→D each touch one or two layers with additive APIs;
  E changes a struct every backend unpacks.
- B is the only tier that is a workaround-eliminator. C–E are ergonomics and coverage.
- Each tier is a separable upstream PR and they merge in order without rework.
- C+D already cover "HDR image viewer/editor with a real picker" — ~all realistic apps.

### Suggested upstream sequencing (PRs mapped to tiers)

1. **PR 1 (tier B)**: `HdrPreference` + capability probe + color space in
   `configure_surface`, plus the §7.4 API shape — `HdrStatus` reporting
   (`RenderState` + `frame.info()`), in-place `preferred_framebuffer_format(caps, hdr)`
   replacement, symmetric `preferred_color_space`, single internal `decide_target`.
   Restores eframe. Small.
2. **PR 2 (tier B hardening)**: explicit `OutputEncoding`, scRGB correctness, dither
   suppression, #7484 recreate semantics.
3. **PR 3 (tier C)**: float textures + `ui.image()` HDR path.
4. **PR 4 (tier D)**: unclamped color plumbing + `color_edit_button_hdr` + swatch widgets.
5. **Side items**: `DisplayHdrInfo` pass-through in `frame.info()` (not just headroom —
   Windows gets nits + chromaticity free, see §7.5), capture fp16 readback, docs +
   upstream our test app as an example.
6. **Deferred (tier E)**: float vertex colors — until a workload textures can't serve
   shows up (animated HDR vector scopes, HDR text).

**Non-goals to defend upstream:** float theme/`Visuals`, HDR-styled chrome, tone mapping
operators (app's job), web/wasm until wgpu's web surface color spaces settle.

## 6. D vs E — raster vs vector HDR

**D gives you HDR color as a function of position inside a rectangle (raster). E gives you
HDR color as a property of geometry (vector).** At D, every HDR pixel you can put on
screen must first exist as texels in some float texture you upload. At E, the tessellator
can attach HDR color to the triangles it generates. Concretely, what stays impossible
at D:

1. **Stroked paths.** Lines, polylines, curves, circle/rect outlines. egui's tessellator
   turns a stroke into a triangle strip with miters, joins, caps, and anti-aliasing via
   the feathering pass — and the stroke color lives in the *vertices*. Can't be reproduced
   with textures: `Painter::image` draws an axis-aligned quad with axis-aligned UVs, so
   you can't even draw a rotated solid HDR line, let alone a joined polyline.
   `egui_plot` series in HDR colors, waveform/vectorscope traces, HDR outlines — all
   stroke-shaped, all locked at D.
2. **Text.** Glyphs are meshes sampling an R8 alpha mask from the font atlas, multiplied
   by the vertex color (`in.color * tex_gamma` in `egui.wgsl`). The mask is texture, but
   the *color* is vertex-borne — capped at 1.0. At D, text is forever SDR; workaround is
   CPU-baking text into a float texture.
3. **Gradients across arbitrary shapes.** `rect_gradient` / per-vertex Gouraud
   interpolation across any triangulated shape — gradient-filled circle, color varying
   along a curved path, glow falloff in mesh vertices. Textures fake axis-aligned (N×1)
   and radial (W×H) gradients, but a gradient *clipped to an arbitrary shape* needs
   masking, which egui's painter doesn't have. Per-vertex interpolation is the only
   masking mechanism the tessellator offers.
4. **Anything where color is computed during tessellation.** Feathering modulates vertex
   alpha along edges; custom mesh effects tie color to vertices. No texture hook exists.

**The D escape hatch** for all of the above: CPU-rasterize it yourself and upload a float
texture (waveform trace → W×H float bitmap per frame; static HDR label → baked text). It
works, and for static/slow content it's fine. Where it falls over:

- **Animated content at 60fps** — replaces GPU tessellation + small vertex buffer with a
  CPU rasterizer + full-frame upload every frame. Real CPU/battery cost for exactly the
  apps (video scopes) that want this.
- **AA quality and stroke geometry** — reimplementing joins/caps/feathering the
  tessellator already has; egui's snapshot tests / DPI handling don't know about baked
  bitmaps.
- **Per-app engine work** — the thing we're trying to eliminate by pushing HDR upstream.

Who actually needs E: video scopes (waveform/vectorscope at framerate), design tools with
HDR gradient meshes, plot-heavy scientific UIs over HDR canvases, HDR-styled annotations.
Arguably the most universal one is **text** — any app labeling content above SDR white
(nits readouts, scope annotations) wants labels that can actually go bright, and there's
no clean D-tier story for text at all.

---

## 7. Open questions / not yet discussed

Threads surfaced but not resolved, roughly ordered by how much they gate the next concrete
step (drafting the upstream issue / building PR 1).

### 7.1 The float color contract — DECIDED (2026-09-12): extended-gamma sRGB

**Contract: a float color/texel/vertex-color in egui's HDR path is extended sRGB —
gamma-encoded, premultiplied (matching `ColorImage` storage), unclamped above 1.0,
odd-symmetric for negatives; 1.0 = SDR reference white. Linear (`ecolor::Rgba`) stays the
app-side math space, converted at the boundary.**

Rationale (full discussion 2026-09-12):

1. **Two of three spaces were never open.** App/math space is already linear (`Rgba`);
   output space is already extended-gamma (tier B chose `ExtendedSrgb`). Only the pipeline
   space (texels/vertex colors) was undecided.
2. **Linear content is a mirage on a gamma surface.** Blending is fixed-function in
   framebuffer space = gamma-encoded. Linear-stored content gets gamma-encoded at output
   and blended in gamma anyway — the correctness benefit only exists on
   `ExtendedSrgbLinear` surfaces, which tier B rejected (SDR UI washout). Linear would
   cost: new gamma-*encode* entry points, dual filtering semantics, tier E forced early.
3. **Gamma slots into the existing grammar with zero shader changes.** `in.color ×
   tex_gamma` stays valid with both factors gamma-extended; both WGSL entry points are
   already unclamped; a future scRGB-linear surface reuses the same content contract with
   the existing linearize entry point.
4. **Quantified cost of gamma math:** gamma-space multiply skews luminance ~5.5% per
   multiply (the 1.055 coefficient mismatch in the power-law region); power laws compose
   correctly for large values so the error is bounded — same class as the existing SDR
   dark-toe quirk. Ingest = one pow per channel (noise vs PQ EOTF decode).
5. **Precision is a wash at fp16.** Floating point has constant relative precision; the
   classic gamma-redistributes-precision argument is fixed-point-only. Neither option wins
   on precision — the decision rests on grammar/compositing.
6. **Symmetry with the existing `Color32` (gamma paint) ↔ `Rgba` (linear math) split.**
   HDR extends the split instead of forking it. Picker edits `&mut Rgba` (linear,
   scene-referred — the right edit space), converts via `gamma_from_linear` at paint.
   Bonus: gradient strips interpolated in gamma = perceptual interpolation, what pickers
   want anyway.

Derived API decisions:

- Float image variant: pixels `Vec<[f32; 4]>` gamma-extended premultiplied in RAM
  (f32 now, no `half` dep; 16 B/px desktop-v1-acceptable), converted to `Rgba16Float`
  at upload; sampling/filtering path unchanged.
- `ui.image()` tint: existing `Color32 × texel` multiply just works.
- Tier D picker state: linear `Rgba`; stops/exposure slider above 1.0.
- Tier E vertex colors: f16×4, same gamma-extended contract.
- Issue sentence covering §7.2 simultaneously: "egui's HDR color space is extended sRGB
  (display-referred); gamut conversion is app territory."
- Future out: tagged/color-managed textures (à la OCIO) remain possible — implicit
  extended-sRGB is the degenerate case of a tag system.

### 7.2 Gamut scope — effectively settled by §7.1 (2026-09-12)

**Resolution:** §7.1's contract decision *is* the gamut decision — extended sRGB is
declared "the egui color space" and gamut conversion is documented as app territory (a v1
non-goal, one explicit sentence in the upstream issue). Wide-gamut surfaces / color-
managed images stay out of scope until demand appears.

An HDR display is usually a P3 display; HDR video (Rec.2100/PQ) uses bt.2020 primaries.
Displaying it on an extended-sRGB surface requires the *app* to gamut-convert, and
saturated >1.0 colors clip. Decision needed: declare extended sRGB "the egui color space"
and document gamut conversion as app territory (probably right for v1), or pull wide-gamut
(P3 surfaces, color-managed images) into scope. Should be an explicit line in the upstream
issue either way — someone *will* ask.

Data note (from the §7.5 source read): wgpu already exposes `DisplayChromaticity`
(Windows, via DXGI) and a coarse `DisplayGamut` bucket (sRGB/P3/Rec2020, web + cross-check
elsewhere) — so if gamut ever enters scope, part of the data layer exists already.

### 7.3 Platform matrix — DECIDED (2026-09-11): macOS-only tested

macOS is verified end-to-end (our app). **Decision:** upstream PR(s) ship with macOS as
the only tested platform; Windows and Linux labeled **experimental** — the probe + SDR
fallback path is exercised, real-HDR output is unverified on real hardware. No hardware
testing commitments before merge.

Cheap research still wanted (non-blocking):

- **Windows**: which DXGI color space wgpu 30's `ExtendedSrgb` maps to — scRGB FP16
  compositor path or HDR10/PQ? `[INFERENCE]` scRGB-FP16 is the likely mapping. Promising
  regardless: wgpu already surfaces Windows DXGI nits + chromaticity (§7.5), so the data
  side is rich there.
- **Linux/Wayland**: color-management protocol maturity; Vulkan
  `VK_EXT_swapchain_colorspace` support in wgpu.
- **Web**: canvas float16 + color spaces — deferred.

Also from wgpu docs: wgpu never writes HDR metadata (`vkSetHdrMetadataEXT` / DXGI
`SetHDRMetaData`); only relevant if a PQ/HDR10 output mode ever enters scope — the
extended-sRGB path needs none.

### 7.4 Reporting/fallback API shape — DECIDED (2026-09-19)

**Reporting:** poll-based. `RenderState` holds the authoritative `HdrStatus` — an enum,
not a bool, carrying the fallback *reason* (`Hdr { .. }` / `Sdr { reason }`, reasons at
least `Disabled` and `NoExtendedSrgb`; aids bug reports) — mirrored as a plain field in
`frame.info()`, refreshed at reconfigure/recreate (the same points `configure_surface`
runs). No callback/event: HDR state changes rarely, egui's immediate-mode model makes
per-frame reads free, and next-frame pickup suffices for every plausible consumer
(badges, tone-map switches, slider disabling).

**Per-viewport:** split the question.

- *Format/capability is global.* One `RenderState { target_format, renderer }` is shared
  by all viewports ("all egui viewports share the same painter" — baked pipeline targets).
  An `Rgba16Float + ExtendedSrgb` surface is valid on an SDR display too (values >1.0
  just clip/composite down), so a global fp16 choice is harmless everywhere and the
  shared Renderer stays shared.
- *Headroom is per-surface.* `display_hdr_info()` is queried per surface anyway (§7.5
  `DisplayHdrInfo` pass-through) — that's the live per-display quantity.
- Residual policy footnote: which surface's probe decides the global format (primary?
  any?) — one sentence to pick when writing PR 1.

**Format-selection API (supersedes the earlier "richer sibling" sketch):**

- `preferred_framebuffer_format` **keeps its name** and takes an in-place signature
  replacement riding the 0.x minor bump:
  `(caps: &SurfaceCapabilities, hdr: HdrPreference) -> Result<(TextureFormat, HdrStatus),
  WgpuError>`. Rationale: egui's 0.x culture is routine breaking changes (0.35→0.36:
  wgpu major bump, glow removed, `Renderer::new` params → struct); a loud compile error
  beats silent semantic drift; and no-overloading means the 1-arg form can't coexist
  anyway. The *deceiving* variant is keeping the old function untouched beside a sibling
  — its implicit "this predicts egui's choice" identity silently fails under
  `HdrPreference::Prefer`.
- New symmetric projection **`preferred_color_space(caps, hdr) -> SurfaceColorSpace`**
  (total; `Auto` if no formats at all). Same inputs as the format function — never
  asymmetric parameter lists.
- **One internal decider**: `decide_target(caps, hdr) -> (format, color_space, hdr_status)`
  is the single source of the decision; both public functions are thin projections of it,
  and `RenderState::create` stores all three outputs (`target_format`,
  `target_color_space`, `hdr_status`). `configure_surface` reads the stored color-space
  field — no derivation match anywhere; nothing "assumes" a color space, the decision
  *has* one, stored and named. Coherence is by construction, not discipline; the probe
  re-run is free (`caps.color_spaces()` reads an in-memory struct).
- **Invariant unit test**: `decide_target` never returns an incoherent pair
  (`(Rgba16Float, Auto)` or `(u8-format, ExtendedSrgb)`) — catches drift in the act.
- **PR-2 evolution**: only `decide_target` grows (new policy variant for scRGB, richer
  `HdrStatus`); both public signatures survive unchanged.

### 7.5 Calibration: nits vs multipliers — RESOLVED via wgpu source (2026-09-11)

No egui research exists to do — egui has no luminance/HDR API at all. wgpu 30's
`DisplayHdrInfo` (wgpu-types/src/surface.rs:587-639) already surfaces everything each
platform offers, and the split is stark:

- **Windows (DXGI)**: absolute nits — `DisplayLuminance { max_nits,
  max_full_frame_nits, min_nits, sdr_white_nits }`, with `sdr_white_nits` moving with
  the brightness slider. Plus `DisplayChromaticity` (primaries + white point, CIE 1931
  xy). True-nits readouts are feasible on Windows.
- **macOS (Apple EDR)**: relative multipliers only — `DisplayHeadroom { current,
  potential, reference }` (maps to the `NSScreen`/`UIScreen` EDR properties). wgpu docs
  are explicit: "Apple exposes no absolute-nit equivalent … can't be converted to nits."
  Nits readouts are architecturally unavailable on macOS.

Consequences:

- The **universal currency for HDR UI is stops / headroom multiplier** (what our test
  app shows). Nits can only be a Windows-only best-effort annotation.
- The egui side-item should pass through the whole `DisplayHdrInfo` via
  `frame.info()` / `RenderState`, not a distilled headroom `f32`.
- Remaining open crumb (app-level, not egui/wgpu): mapping absolute-encoded content
  (PQ / ST 2084 — the future AVIF viewer) onto EDR requires an SDR-white-in-nits
  assumption on macOS since the OS won't report one; nominal 100 nits is the common
  convention.

### 7.6 Tier E's tessellator format controversy (deferred, but shapes the debate)

Always-float vertices double memory/bandwidth for every app — upstream won't take that.
Alternatives: dual-format meshes (complexity in every backend), sidecar float color buffer
(ugly), pack RGBA as f16×4 = 8 bytes in the existing layout slot + adjacent u32 (stride
20→24, needs vertex format juggling). Not needed until E, but expect this to be the
stickiest review thread when it lands.

### 7.7 Testing / QA story

How is HDR verified in CI? Headless offscreen rendering to fp16 works without any HDR
display (values >1.0 are just numbers), so kittest/wgpu-offscreen can regression-test
encoding correctness; what's untestable in CI is the *display* interpretation (EDR). Plan:
snapshot tests for the encoding contract + our test app upstreamed as the manual example.

### 7.8 Feature interactions to verify (not design)

MSAA resolve on fp16 targets; `capture` readback of float surfaces; transparent windows
(alpha mode × HDR); `present_with_transaction` macOS resize fix coexistence;
`WgpuSetup::Existing` (user-supplied device) + HDR probing ordering.

### 7.9 Process / strategy

- Draft the upstream issue text (lead with PR 1; embed the shader-correctness analysis —
  ExtendedSrgb vs ExtendedSrgbLinear — since that's the non-obvious part that keeps
  others from getting burned).
- Decide whether we build PR 1 against an egui fork now and switch our app to it via a
  `[patch]` override — also derisks the PR (proves it against a real app).
- Timing: wgpu 30 landed recently; target the 0.37 window?

---

## Appendix: verified source anchors (0.36.2)

`egui-wgpu/src/lib.rs`:
- `SurfaceConfig { present_mode, desired_maximum_frame_latency }` + `LOW_LATENCY` /
  `HIGH_THROUGHPUT` consts — :76-109
- `RenderState { adapter, available_adapters, instance, device, queue, target_format,
  renderer, surface_config }` — :111-140
- `RenderState::create`: `preferred_framebuffer_format(&surface_formats)` pick; defaults
  `vec![Rgba8Unorm]` when no compatible surface — ~:289
- `WgpuConfiguration { surface, wgpu_setup, on_surface_status }` — :335-346
- `preferred_framebuffer_format`: first `Rgba8Unorm | Bgra8Unorm`, else `formats[0]` —
  :411-427
- `SurfaceErrorAction { SkipFrame, Reconfigure, RecreateSurface }` — :307-328

`egui-wgpu/src/winit.rs`:
- `Painter::configure_surface` — builds `wgpu::SurfaceConfiguration` with
  `format: target_format`, `view_formats: vec![target_format]`, never sets `color_space`
  — :100-154
- `recreate_surface` / `install_surface` / alpha-mode selection — :157-297

`egui-wgpu/src/renderer.rs`:
- `RendererOptions::depth_stencil_format` — :187-191
- `Renderer::new` doc: "should preferably be Rgba8Unorm or Bgra8Unorm, i.e. in
  gamma-space" — :266-267
- vertex layout, `array_stride: 5 * 4`, color as u32 — :378-382
- entry point selection `if output_color_format.is_srgb()` with warn — :407-414
- texture creation hardcoded `TextureFormat::Rgba8Unorm` (+view_formats) — :701-705
- `register_native_texture` / `..._with_sampler_options` documented Rgba8Unorm-only —
  :772-824

`egui-wgpu/src/egui.wgsl`:
- `fs_main_linear_framebuffer` = `linear_from_gamma_rgb(in.color × tex)` — the correct
  shader for scRGB, already exists
- `fs_main_gamma_framebuffer` = gamma passthrough — correct for ExtendedSrgb
- `dither_interleaved(rgb, 256.0, …)` assumes 8-bit [0,1] target
- `gamma_from_linear_rgb` / `linear_from_gamma_rgb` unclamped (pow works > 1.0)

`ecolor/src/lib.rs`:
- conventions: u8 = gamma, f32 0-1 = linear — :8-16
- `gamma_u8_from_linear_f32` clamps — :112-124; `linear_u8_from_linear_f32` — :126-131
- `linear_from_gamma` / `gamma_from_linear` NOT clamped, "works for numbers outside this
  range" — :157-179
- `Rgba` = f32×4 linear, no internal clamping

`epaint/src/image.rs`:
- `pub enum ImageData { Color(Arc<ColorImage>) }` — single variant — :16-18
- `ColorImage { size: [usize; 2], pixels: Vec<Color32> }` — :47-51

`wgpu-types-30.0.1/src/surface.rs`:
- `DisplayHdrInfo { luminance, headroom, chromaticity, coarse, bits_per_color }` —
  :587-610; per-platform population documented per field
- `DisplayLuminance { max_nits, max_full_frame_nits, min_nits, sdr_white_nits }` —
  Windows/DXGI only — :619-643
- `DisplayHeadroom { current, potential, reference }` — Apple only, explicitly not
  convertible to nits — :645-660
- `DisplayChromaticity { red, green, blue, white }` — Windows/DXGI only — :662-673
- `DisplayCoarseRange { high_dynamic_range, gamut: DisplayGamut }` — web + cross-check —
  :675-706
- `tone_map_headroom()` precedence: Apple multiplier → Windows `max/sdr_white` → None —
  :712-754
