# egui HDR support — working notes

Working doc for "what must change in egui for native HDR". Accumulates findings from our
investigations and discussions; not an implementation plan. The app-side workaround this all
stems from lives in `../hdr-test/rust` (its build plan was never committed to this repo):
a hand-rolled winit loop + `egui` + `egui-winit` + `egui_wgpu::Renderer` owning the
`SurfaceConfiguration`, because eframe 0.36 cannot create an HDR surface (§1). The app is
the SDL3 slider test ported to Rust: a 2×2 quad driven 0–4 stops above SDR white by a
`CallbackTrait` shader, an orange marker where EDR headroom falls, and a live headroom
label polled on the main thread each frame (250 ms idle repaint). Choices it validated
working end-to-end on macOS M1:
- `Rgba16Float` + `SurfaceColorSpace::ExtendedSrgb` — *not* `ExtendedSrgbLinear`, since
  egui's `is_srgb()` shader pick emits gamma-encoded values that would double-gamma on an
  scRGB surface (the #7484 washout);
- custom quad shader with the sRGB curve unclamped above 1.0; `dithering: false`;
- SDR fallback = `caps.formats[0]` + `Auto` (probe via
  `caps.color_spaces(Rgba16Float).contains(EXTENDED_SRGB)`);
- no runtime HDR toggle — a format change requires recreating `egui::Context` +
  `egui_winit::State` + `egui_wgpu::Renderer` together.

All source anchors below were verified against the vendored sources on 2026-09-11
(workspace note, 2026-10-25: Cargo.lock pins wgpu **30.0.0**; registry 30.0.1 diffs
identical in every cited region):
egui / egui-wgpu / egui-winit / ecolor / epaint **0.36.2**, wgpu **30.0.x**.
Upstream tracker status (checked 2026-09-11; **re-verified 2026-10-25**): the 2026-09-11
check found no open HDR issue or PR, but as of 2026-10-12
[PR #8703](https://github.com/emilk/egui/pull/8703) (`SurfaceConfig::prefer_hdr`,
scRGB-linear) occupies tier B — open, merge-blocked, zero maintainer comments (see §7.9).
wgpu 30 shipped the foundation (`SurfaceColorSpace`, `display_hdr_info`) closing
[wgpu#2920](https://github.com/gfx-rs/wgpu/issues/2920). egui 0.36 has **two** GPU backends
(wgpu the default; `egui_glow` opt-in since 0.34, kept for binary size). HDR is wgpu-only
**by capability**: OpenGL/GLES has no extended-sRGB swapchain concept and wgpu's GLES
backend reports `SRGB`-only color spaces — the glow path stays SDR by design, a stated
scope decision.

**kittest note (corrected 2026-10-25 — the earlier labor split was inverted):** the
workspace depends on the upstream `kittest` crate (rerun-io/kittest, **0.5.0**, sibling
checkout at `/Users/alex/Developer/kittest`) — but that crate is an *accesskit query
layer only*. `Harness`, the builder, snapshot machinery, the wgpu renderer, and readback
all live **in-repo** in `crates/egui_kittest`. All fp16 testing work (§7.7) lands
in-repo; the sibling checkout is a reading reference only, and no kittest fork/patch is
needed (decision log R2-10).

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
  `color_space`, so wgpu 30 defaults it to `Auto` — and `Auto` resolves `Rgba16Float` to
  `ExtendedSrgbLinear` (scRGB) wherever the surface supports it
  (`resolve_auto_color_space` falls back `[ExtendedSrgbLinear, Srgb]`). A smuggled fp16
  surface is therefore not SDR-interpreted but extended **linear**, misinterpreting egui's
  gamma-encoded output: washed mid-tones, broken headroom semantics. The #7484 failure is the *partial-recreate* case (format switch + renderer recreate
  with stale Context caches — corrected 2026-10-25; on surfaces without fp16 the
  failure differs). Note this means macOS already renders egui on an EDR
  surface by accident — explicit policy beats accidental EDR.

The change: add an HDR *policy* — not raw wgpu knobs — **on `WgpuConfiguration`,
documented create-time-only; explicitly *not* on `SurfaceConfig`** (decided 2026-10-25):

```rust
pub enum SurfaceColorSpacePreference {  // renamed from HdrPreference — governs
                                       // wide-gamut + HDR policy; final name at impl
    Off,               // today's behavior — the rollout default (Rgba8, SDR)
    Auto,              // probe; best available (DisplayP3 → extended pair), else SDR
    DisplayP3,         // PR 1 — wide gamut, SDR range (Rgba8, P3-referred)
    ExtendedSrgb,      // PR 3 — HDR brightness (Rgba16Float + EXTENDED_SRGB)
    ExtendedDisplayP3, // PR 3 — wide gamut + HDR brightness (Rgba16Float)
}
```
(shape decided 2026-10-25; `DisplayP3` added and HDR variants moved to PR 3 by the
split-PR reorder same day)

Why not `SurfaceConfig`: it is the *runtime-mutable* subset ("Runtime-mutable subset of
`WgpuConfiguration`" — edit any field to have the surface reconfigured on the next paint;
`Painter::set_surface_config` is cheap to call every frame), so a policy there would
advertise runtime flips that silently no-op, or half-apply (reconfigure without pipeline
rebuild = the #7484 black screen). Runtime HDR changes are a documented non-goal (§4).
Enum variants and default (incl. an `Auto` variant as the endgame default once capture/
kittest fp16 infrastructure exists) are a §7.4 decision — direction recorded 2026-10-25:
HDR-by-default is the goal; the `Off` default is upstream rollout safety only, and our
app/fork opts in unconditionally. Note for the #8703 evolution: that PR places
`prefer_hdr: bool` on `SurfaceConfig`; the hardening PR relocates it to the create-time
home.

`RenderState::create` probes
`caps.color_spaces(Rgba16Float).contains(SurfaceColorSpaces::EXTENDED_SRGB)`, picks
accordingly or falls back to `Off`, records what it got
(`RenderState::hdr_status: HdrStatus` — payload pinned in §7.4), and
`configure_surface` threads the color space through. An `Option<TextureFormat>` escape
hatch for power users is cheap to add beside it.

**Invariant: `configure_surface` sets `color_space` explicitly from the stored decision —
never leaves `Auto`.** egui's config comes from `..get_default_config(...)`, and wgpu's
default seeds `color_space: Auto`, so a forgotten explicit assignment + fp16 silently
ships scRGB-linear. The §7.4 invariant test asserts the *built* `SurfaceConfiguration` is
coherent, not just `decide_surface_target`'s return.

**Why extended-gamma (`ExtendedSrgb`) rather than the equally shader-correct scRGB-linear:**
two correct combinations exist — ExtendedSrgb + gamma passthrough, and ExtendedSrgbLinear +
the existing linearize entry point ([#8703](https://github.com/emilk/egui/pull/8703) ships
the latter). The choice is *continuity* (corrected 2026-10-25; "the one shader-correct
combination" was false): (a) formula-identical status-quo parity for SDR chrome — same blend
grammar, same u8 semantics; bit-exact on u8 targets, ≤½-code drift on fp16 targets
(the fp16 path skips store-time quantization and is the more accurate side; snapshots:
one-time regeneration) (egui "prefers gamma color spaces for all
blending", `egui/src/lib.rs:147`); (b) one texture grammar for u8 and float content;
(c) `ExtendedSrgbLinear` is **native-only** (`surface.rs:219-220`) — `ExtendedSrgb` and
`ExtendedDisplayP3` are the HDR spaces that cover web (amended 2026-10-25 when the
PR-1 reorder promoted P3; both verified on web fp16 canvases). Honest permanent cost,
stated once: HDR-over-HDR
alpha compositing and filtering happen in gamma (50%-alpha white over black → linear
0.214, not 0.25), fixed only by a future linear-surface policy. The policy enum still
earns its keep vs raw knobs: it encodes *which* combination, so the #7484 double-gamma
trap can't be re-created by knob-twiddling.

**This PR alone would have eliminated our entire hand-rolled loop.** Everything else we did
— callbacks, tick drawing, headroom label — works fine on top of eframe once the surface is
right. Sizing (corrected 2026-10-25; the original "~150 lines" pitched §1's core but the
sequenced PR carries more): the §1 core — policy enum, probe, color-space threading — is
~150 lines across `lib.rs` + `winit.rs`; PR 1 as sequenced (§5) adds `decide_surface_target`
+ `HdrStatus` + the primaries pairing table + tests + `MANUAL_TESTING.md` + the example:
realistically **~300–450 lines incl. tests/docs** — no shader changes, no `epaint` changes,
no eframe changes (the mirror was cut, entry 22). Still cheap; now honestly priced.
Re-measure against upstream `main` before drafting (entry 4).

## 2. Output encoding: the shader selection is accidentally correct

`Renderer::new` picks its fragment entry point by `output_color_format.is_srgb()`
(renderer.rs:407-414): `*Srgb` formats get `fs_main_linear_framebuffer`, everything else
`fs_main_gamma_framebuffer`. For `Rgba16Float` (not an `*Srgb` format) this selects the
gamma path — which happens to be *right* for `SurfaceColorSpace::ExtendedSrgb`
(gamma-encoded, extended range) and *wrong* for `ExtendedSrgbLinear`/scRGB, where you'd
want linear output and instead get a gamma curve baked in. The heuristic can't see the
color space, so it can't distinguish them.

Interesting detail from `egui.wgsl`: `fs_main_linear_framebuffer` already does
`linear_from_gamma_rgb(vertex_color × texture)` — the correct *existing* entry point for
scRGB, modulo dither placement (applied pre-linearization) and negative handling (§7.1);
both fold into the PR-4 scRGB work.
So the fix is not new shaders, it's an explicit selection:

```rust
enum OutputEncoding { GammaEncoded, Linear }
// OutputEncoding::of(target_format, target_color_space) — total function
```

Computed once at `Renderer::new` (replacing the `is_srgb()` sniff — the derivation
already exists, it is merely color-space-blind today); **never stored** — storing a
derivable value next to its inputs re-introduces the drift-on-discount that
`decide_surface_target` killed. Table: u8 formats → `GammaEncoded`; u8 + `DisplayP3` →
`GammaEncoded`; fp16 + `ExtendedSrgb` → `GammaEncoded`; fp16 + `ExtendedDisplayP3` →
`GammaEncoded`; fp16 + `ExtendedSrgbLinear` → `Linear`. Consequence: **PR 1 needs no
encoding change at all** (fp16 + ExtendedSrgb → gamma passthrough is exactly today's
`is_srgb()`-false path); the color-space input reaches `RendererOptions` only in PR 4,
when the scRGB policy needs the linearize path. The invariant test covers the whole
table. Side fix: on float targets, **force dithering off** regardless of
`RendererOptions::dithering` — the noise is applied to encoded values
(`egui.wgsl:180`), and with no 8-bit quantizer to hide in, near-black it becomes
×12.92 relative-luminance grain: actively harmful in shadows, not "meaningless".
The knob and its ON-default stay untouched for 8-bit targets (current behavior);
the float-target doc note updates to "assumes an 8-bit sRGB target".

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
  paint" — but a format change invalidates baked pipeline targets. **Decided
  (2026-10-25): create-time-only for the upstream-facing API; `Painter::set_hdr()` is
  dropped.** Rationale: a renderer-only recreate loses `Renderer.textures` (font atlas,
  uploads) and every `register_native_texture` bind group — egui sends *deltas* per frame
  (`paint_and_update_textures`), so a fresh Renderer's empty texture map means meshes
  reference TextureIds with no texture: blank glyphs and "Missing texture" fallbacks
  until each image happens to be re-touched. Re-pushing managed textures is possible via
  `ctx.tex_manager()`, but re-registering user native textures has no recipe
  (`next_user_texture_id` restarts; app-held TextureIds dangle). Recreating the egui
  `Context` — the only fully safe option — is not implementable as an eframe API (apps
  hold `Context` by value; there is no handle-swap story). Half-measures fail *silently*,
  which is worse than #7484's visible black screen. Full recreate remains an *app-level*
  technique (our app owns the whole stack and does exactly this; see header) — it is not
  an upstream deliverable. If runtime switching is ever specced, it must be: rebuild
  RenderState + re-push all managed textures from `ctx.tex_manager()` + return a
  texture-id remapping for user native textures.
- **Display transitions** (dragging between SDR/HDR monitors): **nothing to switch**
  (corrected 2026-10-25; supersedes "needs surface recreate at minimum"). The
  fp16+ExtendedSrgb config is display-agnostic — it stays valid on both display types;
  SDR content renders identically to a plain SDR surface (§2's by-construction result),
  and values >1.0 simply clip at SDR white (#8703 observed this live with HDR off). The
  only quantity that changes is `tone_map_headroom()`: on an SDR display it returns
  `Some(1.0)` (Apple EDR multiplier reports 1.0 on non-EDR screens, and
  `coarse.high_dynamic_range == Some(false)` pins 1.0 to prevent phantom headroom) —
  verified at `wgpu-types-30.0.1/src/surface.rs:732-757`. It is polled per frame
  (main-thread; EDR headroom ramps ~1-2 s), which is the entire transition story.
  [INFERENCE from config semantics: resize-driven reconfigure carries the stored color
  space; confirm in the macOS manual pass.] This is also why create-time-only is cheap:
  the surface never needs to change for a display change.
- **Headroom reporting**: expose `display_hdr_info().tone_map_headroom()` through
  `RenderState` (entry 22 cut the `frame.info()` mirror — apps read it via
  `frame.wgpu_render_state()`), so apps can build UI like our orange marker without
  reaching into wgpu. Trivial, main-thread-only (Metal reads `NSScreen`) — eframe already
  lives there. Note EDR headroom ramps over ~1-2 s on macOS → poll per frame +
  `request_repaint_after` when idle. The per-viewport `DisplayHdrInfo` pass-through is
  its own later PR (entry 22).
- **Capture/kittest**: the `capture` feature matches only Rgba8 surfaces and on float
  targets *silently produces nothing* (logs + returns — no screenshot delivered; entry
  34), so the fp16 readback branch is a PR-3 deliverable.

---

## 5. The invasiveness ladder (tiers A–E)

Ordered by how deep you cut into egui. Mental model: **invasiveness = which crates you
edit**, and egui's stack is layered `ecolor`/`emath` → `epaint` (tessellation) → `egui`
(widgets) → `egui-wgpu`/`eframe` (backends). The deeper the layer, the more code above it
ripples.

| | Tier | Crates touched | ~Size | What it unlocks |
|---|---|---|---|---|
| **A** | None — app-side | *none* | — | Custom shader rects (paint callbacks) |
| **B** | Surface creation (wide gamut + HDR) | `egui-wgpu` only | PR 1+2, ~300–450 lines | A wide-gamut/HDR *window*; callbacks in any eframe app |
| **C** | Float textures | `epaint` + `egui` + `egui-wgpu` | medium, additive + breaking enum variant | HDR *images*, swatches-as-textures |
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

**C — float content through textures.** First cut into egui's data model — additive +
one breaking enum variant (corrected 2026-10-25; "purely additive" was false): float
variant in `epaint::ImageData` drops `Eq` from `ImageData`/`ImageDelta`/`TexturesDelta`
(f32 has no `Eq`; matches `Rgba`'s PartialEq-only precedent) and extends the closed
serde enum (changelog note; downstream exhaustive matches break), plus an `Rgba16Float`
path in `Renderer::update_texture` and float-capable `TextureHandle` (epaint gains a
direct `half` dep — already transitive via wgpu; see §7.1). Unlocks: `ui.image()`
displaying HDR bitmaps (PQ AVIF →
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
methods, then ripple through **every consumer of the mesh** (corrected 2026-10-25 — the
old list named a software renderer that doesn't exist and omitted the glow painter):
egui-wgpu pipelines, `egui_glow` painter (or scope E out explicitly — entry 2), kittest
snapshot comparison, `capture` readback, mesh serde + the `Eq` cascade
(`Vertex → Mesh → Shape`, both `cfg` variants incl. `unity`), and `Context::tessellate`
consumers. Unlocks: everything paintable is potentially HDR. Theme stays `Color32` by
choice, not limitation.

**Reading the ladder:**

- Invasiveness is superlinear at E. A→D each touch one or two layers with additive APIs;
  E changes a struct every backend unpacks.
- B is the only tier that is a workaround-eliminator. C–E are ergonomics and coverage.
- Each tier is a separable upstream PR and they merge in order without rework.
- C+D + PR-1's pairing table already cover "HDR/wide-gamut image viewer/editor with a
  real picker" — **all apps whose content is rectangular, within extended brightness,
  and within the gamut of a *paired* primaries** (qualifiers added 2026-10-25; first
  paired instance: Display P3). Outside that envelope, deliberately: content tagged with
  unpaired primaries (e.g. Bt2020-gamma) app-converts to a paired primaries — a
  documented path, not a limitation of the mechanism; vector-shaped HDR (scopes,
  polylines, HDR text) waits for E (§6); node compositors therefore land in E's list.
  HDR video playback at tier C carries per-frame upload cost — halved by entry #29's
  f16-in-RAM decision (~33 MB/frame at 4K) but real at 60 fps. State the envelope
  explicitly in the issue draft — maintainers will ask about content in unpaired
  primaries immediately.

### Upstream sequencing — RESTRUCTURED 2026-10-25 (every PR ships consumed, testable functionality; PR 0 dissolved into its consumers)

1. **PR 1 (wide-gamut surface)**: `SurfaceColorSpacePreference { Off, Auto, DisplayP3 }`,
   `decide_surface_target`, `HdrStatus`, configure threading (color space from the
   stored decision), paint-callback demo showing out-of-sRGB saturation. **No epaint
   changes, no new color types.** Tests: decider property tests over synthetic
   `format_capabilities`, u8 kittest snapshots, windowed smoke (§7.3). Opt-in chrome
   shift documented (neutrals 0.0 ΔE, accents ≤6.2 ΔE; app-side conversion in v1).
   First paired instance: `DisplayP3` (Metal + web report it for u8 surfaces; DX12 does
   not — deterministic SDR fallback, entry 10).
2. **PR 2 (tagged float content — tier C)**: `Primaries` enum (`Srgb | DisplayP3`),
   `FloatImage` (f16 pixels, primaries-tagged), `ImageData::Float` (+ `Eq`/serde
   changelog + the `half` dep), `update_texture` pairing (Rgba16Float upload for paired
   tags; unpaired tags reject loudly), `ui.image()` float display, **WGSL negative
   sign-mirror fix** (consumed: converted float content can carry negatives), ecolor
   contract property tests. Consumer: tagged HDR/wide-gamut images render via
   `ui.image()`.
3. **PR 3 (HDR — extended range)**: `ExtendedSrgb` + `ExtendedDisplayP3` on
   `Rgba16Float` render targets — values >1.0, EDR brightness, macOS + web. The
   `Extended*` preference variants activate, `Auto` finalizes, dither force-off, **fp16
   kittest readback + capture branch** (entry 34), numeric encoding tests, negative GPU
   round-trip.
4. **PR 4 (scRGB — Windows/DX12)**: `ExtendedSrgbLinear` policy (the only HDR path
   there — entry 10) + explicit encoding selection (entry 27) + #7484 recreate
   semantics documentation.
5. **PR 5 (tier D)**: `color_edit_button_hdr` + swatch widgets (entry 30 shape).
6. **Side items**: `DisplayHdrInfo` pass-through (per-viewport home per entry 22), docs
   + test app as an example.
7. **Deferred (tier E)**: float vertex colors — until a workload textures can't serve
   shows up (animated HDR vector scopes, HDR text).

**Restructure rule (user directive, 2026-10-25 — binding):** every PR ships *consumed,
testable* functionality — nothing dead, nothing untestable at merge. Adding a primaries
later = enum variant + pairing row + conversion matrix, all in one PR (the §7.2 design
rule).

**Non-goals to defend upstream:** the glow backend (SDR-only by capability — OpenGL has no
extended color-space mechanism), float theme/`Visuals`, HDR-styled chrome, tone mapping
operators (app's job), web/wasm — deferred for scope (eframe web plumbing + fp16-canvas
request are unwired); wgpu 30's browser backend already reports
`EXTENDED_SRGB | EXTENDED_DISPLAY_P3` on fp16 canvases, so web comes nearly free once the
egui side is wired.

## 6. D vs E — raster vs vector HDR

**D gives you HDR color as a function of position inside a rectangle (raster). E gives you
HDR color as a property of geometry (vector).** At D, every HDR pixel you can put on
screen must first exist as texels in some float texture you upload. At E, the tessellator
can attach HDR color to the triangles it generates. Concretely, what stays impossible
at D:

1. **Stroked paths.** Lines, polylines, curves, circle/rect outlines. egui's tessellator
   turns a stroke into a triangle strip with miters, joins, caps, and anti-aliasing via
   the feathering pass — and the stroke color lives in the *vertices*. Can't be reproduced
   with textures: `Painter::image` draws an axis-aligned quad with axis-aligned UVs
   (corrected 2026-10-25: rotated HDR *rects* do work at D — `Image::rotate` exists,
   `image.rs:243-247, 867-880` — just unantialiased). What's genuinely missing at D:
   **antialiased rotated geometry, joins/caps, and polylines** — the PathShape+texture
   gap.
   `egui_plot` series in HDR colors, waveform/vectorscope traces, HDR outlines — all
   stroke-shaped, all locked at D.
2. **Text.** Glyphs are meshes sampling an R8 alpha mask from the font atlas, multiplied
   by the vertex color (`in.color * tex_gamma` in `egui.wgsl`). The mask is texture, but
   the *color* is vertex-borne — capped at 1.0. At D, text has **no *general* HDR
   mechanism** (amended 2026-10-25 — "no clean D-tier story at all" overstated: per-widget
   float-texture caching is viable for short labels — a galley-keyed cache of atlas
   coverage × HDR color, same cost model as egui's galley cache, ~16 KB fp16 for a
   readout label, re-rasterized only on string/color/DPI change). That `hdr_text` helper
   is the first thing to prototype if the demo app needs the nits readout: it either
   validates deferring E or becomes the evidence E is needed.
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

Negative extension — resolved 2026-10-25: **odd-symmetric everywhere.** ecolor already
complies; the WGSL gamma fns currently route negatives to the ÷/×12.92 linear branch
(`egui.wgsl:52-57` — GPU encode(−0.5) = −6.46 vs CPU −0.735) and evaluate
`pow(negative, …)` (WGSL-undefined, NaN-prone) in a discarded lane. Tier C must ship the
4-line `sign(x)·f(|x|)` WGSL fix *before* any negative-capable float content, and the
tier-C test matrix gains a CPU↔GPU round-trip test with negative inputs. Chosen over
(b) changing ecolor's documented public behavior, and (c) declaring negatives undefined
— which would close the scRGB-negatives wide-gamut path of §7.2.

Contract clarifications — resolved 2026-10-25 (each chosen to match current SDR behavior
exactly; no change of mechanics):
- **Premultiplication is encoded-space:** `stored = a × enc(L)` — exactly Color32's
  convention (`color32.rs:8`: "gamma space sRGBA with _premultiplied alpha_"), **not**
  `enc(a·L)`. Example: a=0.5, L=4.0 stores 0.912, not 1.353. **Cost, stated honestly
  (added 2026-10-25):** encoded-space premultiply attenuates HDR translucency by
  `a^2.4` — 50% alpha on L=4 ≈ −2.4 stops vs a photometric composite. This is the price
  of encoded-space blending; the fix is alpha-domain work in the future linear-surface
  policy (entry 16's residual cost), not a contract change. Forcing rationale
  (corrected — "forced by the pipeline" overclaimed; the blend is convention-agnostic):
  the real forcing is continuity (Color32 precedent, bit-parity with every existing
  app) + the compositor's `PreMultiplied` contract over the same stored values. Fixed-
  function blending consumes *stored* values in the gamma framebuffer
  (`One / OneMinusSrcAlpha`, `renderer.rs:415-428`) and the window layer is declared
  `PreMultiplied` over the same stored values. Contract test: swatch at a=0.5, L=4
  composites per this rule.
- **Filtering and blending occur in gamma (stored space):** hardware bilinear on
  non-sRGB float formats has no decode, so mip/filtering operates on stored values —
  inherited unchanged from the u8 path. A future linear-surface policy variant changes
  filtering semantics too (worth remembering when it lands).

Rationale (full discussion 2026-09-12):

1. **Two of three spaces were never open.** App/math space is already linear (`Rgba`);
   output space is already extended-gamma (tier B chose `ExtendedSrgb`). Only the pipeline
   space (texels/vertex colors) was undecided.
2. **Blending is fixed-function in framebuffer space — so storage space is blend space
   (verified), and the argument for gamma storage is continuity, not correctness.**
   Linear-*stored* content on this pipeline gets blended in gamma anyway; the correctness
   benefit of linear storage exists only on `ExtendedSrgbLinear` surfaces — which are
   equally shader-correct today (the existing linearize entry point; #8703), and the
   "SDR UI washout" was a property of the *wrong entry point*, not of linear surfaces.
   ExtendedSrgb still wins: formula-identical parity for SDR chrome (§1; bit-exact on u8,
≤½-code drift on fp16), one texture grammar for
   u8 + float content, and web support (`ExtendedSrgbLinear` is native-only). Scope note
   (2026-10-25): the zero-shift property covers Off/ExtendedSrgb only — the DisplayP3
   pairing is opt-in and shifts chrome (quantified in §7.2). Honest
   permanent cost: all HDR-over-HDR compositing and filtering happen in gamma (50%-alpha
   white over black → linear 0.214, not 0.25). A linear-surface policy variant remains
   the future path (§7.1; PR 3/4).
3. **Gamma slots into the existing grammar with zero shader changes.** `in.color ×
   tex_gamma` stays valid with both factors gamma-extended; both WGSL entry points are
   already unclamped; a future scRGB-linear surface reuses the same content contract with
   the existing linearize entry point.
4. **Quantified cost of gamma math (corrected 2026-10-25 — the "~5.5% per multiply"
   figure does not reproduce; measured shape is level-dependent):** multiplying in gamma
   and decoding gives ~+1.5% luminance error at mid-gray (0.5), ~+0.1% near white (0.8),
   +38% at linear 0.1, growing sharply for dark factors (+672% at 0.01) as products
   cross the encode knee; the error is **branch-crossing**: products whose decoded
   value crosses the encode knee (0.0031308 / 0.04045) take the wrong branch's scale —
   in-toe products compose to exactly `12.92·ab` (12.9× off); no single coefficient
   dominates (dropping the intercept alone makes the power-region error 26× worse).
   Accepted
   regardless: formula-identical to today's SDR `in.color × tex` path for [0,1] content
   (bit-exact on u8 targets; ≤½-code drift on fp16 where the u8 path quantized at
   store; extended values have no SDR baseline — their correctness argument is the
   contract, not parity). No regression vs status quo. Ingest = one pow per channel (noise vs PQ EOTF decode).
5. **Precision: linear is measurably finer, but neither is perceptible (corrected
   2026-10-25 — "neither option wins" was half-wrong).** f16 has constant *stored-space*
   relative ULP (~0.05–0.1%), but the display-relevant quantity is linear luminance,
   scaled by |d ln L/d ln V| = 2.4 in the power region: gamma storage ⇒ up to ~0.234%
   luminance error vs ≤0.098% linear (~1.3 bits better where most content lives). Gamma
   storage also goes subnormal below encoded 2⁻¹⁴ (linear L ≈ 4.7×10⁻⁶), degrading the
   step to ~0.5% relative — while *linear* f16 storage's own subnormal floor is
   ~4.6×10⁻⁹ (0.098% step): gamma goes subnormal **12.9× earlier** (labels corrected
   2026-10-25; the original figure attributed linear's floor value to the wrong
   boundary). All far below the ~1% Weber JND — the *decision* still rests
   on grammar/compositing, but the rationale should carry the real numbers.
6. **Symmetry with the existing `Color32` (gamma paint) ↔ `Rgba` (linear math) split.**
   HDR extends the split instead of forking it. Picker edits `&mut Rgba` (linear,
   scene-referred — the right edit space), converts via `gamma_from_linear` at paint.
   Bonus (reworded 2026-10-25): gamma-space interpolation — what the current gradient
   strips do — is closer to perceptual than linear for lightness but drifts hue on
   saturated colors; hue-stable pickers interpolate in their own space (OkLab/HSL)
   before encode.

Derived API decisions:

- Float image variant (shape decided 2026-10-25; RAM format amended same day — f32-in-RAM
  was justified by a false "no `half` dep" reason, and `half 2.7.1` is already resolved
  in Cargo.lock via wgpu/naga): `ImageData::Float(Arc<FloatImage>)` with
  `FloatImage { size: [usize; 2], primaries: Primaries, pixels: Vec<[f16; 4]> }` where
  `pub enum Primaries { Srgb, DisplayP3 }` (v1 — `Bt2020` is added later, together with
  its pairing row + conversion matrix, per the §7.2 design rule) — **tier-C groundwork
  (2026-10-25 restructure; primaries-generic by design — no specific color space is
  privileged):** the
  *tag* is the mechanism. Content declares whichever primaries it holds; a **pairing
  table** decides which surface color space displays each tag natively (v1 pairs `Srgb`
  only; other tags upload-reject loudly until paired). `DisplayP3` is merely the first
  wide-gamut *instance* to pair — chosen because Metal+web expose
  `EXTENDED_DISPLAY_P3`. `Bt2020` has no native gamma surface in wgpu (BT.2020 comes
  only as PQ/HLG transfer — a separate transfer-function scope), so BT.2020-referred
  content displays via conversion to a paired primaries. New primaries = a new enum
  variant, the cheap growth kind. (Answers the #2712 "what primaries?" flag on new
  API.) Pixels gamma-extended,
  premultiplied in the *encoded* components (contract clarifications above). RAM format
  1:1 with the `Rgba16Float` upload: zero-copy via `bytemuck::cast_slice`, no conversion
  pass; half the memory of f32 (4K frame: 66 MB vs 133 MB); f16 quantization = upload
  quantization, zero loss vs display. `epaint` gains a *direct* `half` dep (changelog
  note) with `serde` + `bytemuck` features — the same surface tier E needs. Helpers
  (`new`, `filled`, `from_rgba_*`, lossy `FloatImage::from_color_image`) take/convert
  f32 at the edges so producers keep writing f32 code. Also requires the `From` impls
  `ImageDelta::full/partial(impl Into<ImageData>)` need; drops `Eq` off
  `ImageData`/`ImageDelta`/`TexturesDelta` (`Rgba` precedent); serde roundtrip tests
  incl. NaN/negative/>1.0 payloads. Sampling/filtering path unchanged.
- `ui.image()` tint: existing `Color32 × texel` multiply just works.
- Tier D picker (decided 2026-10-25, amended after HSV deep-dive): `color_edit_button_hdr(ui, rgba: &mut Rgba, alpha: Alpha)` — parameter parity with siblings. **Decoupled chroma/exposure model:** state edits as `(chroma in [0,1], exposure: f32 stops)`, painted as `chroma × 2^exposure` — the grammar our test app validated. **Chroma via the standard HSV field + hue strip (hue/saturation only — V folds into exposure):** the 0–1 Hsva limitation dissolves because chroma is normalized; **brightness via the stops slider** (log scale — the axis HDR wants; our orange-marker pattern). Editing an existing extended value decomposes cleanly: `exposure = log2(max(|c|))` — real-valued, no band clamp (a whole-stops UI variant would reintroduce one; implementation detail) — and `chroma = color / 2^exposure`. Edges defined: black (max = 0) → exposure 0, chroma = color (identity); negative channels carried by chroma per the odd-symmetric contract (decomposition uses max(|c|)); alpha=0 is safe because the edit state is *straight* — the `to_rgba_unmultiplied` black-at-zero hazard belongs to premultiplied conversions, which the paint-boundary conversion never performs. **Straight (un-premultiplied) slider state**, converted at the paint boundary (`to_rgba_unmultiplied` pattern). Alpha = its own slider. Distinct function from `color_edit_button_rgba` (same state type, different range semantics — extending it would change behavior for 0–1 callers), docs cross-referencing both ways. OkLCh chroma geometry = the follow-up upgrade (perceptually uniform, hue-stable; new math in ecolor) when P3-native editing + wide-gamut demand arrive. HSV stays structurally excluded from carrying *brightness* (0–1 V, no multiplicative axis) and from *negative* (sRGB-referred wide-gamut) components (`rgb_from_hsv` emits only `[0, v]`) — the decoupled model routes around both.
- Tier E vertex colors: f16×4, same gamma-extended contract.
- Issue sentence covering §7.2 simultaneously: "egui's HDR color space is extended sRGB
  (display-referred); gamut conversion is app territory (v1 — wide-gamut output is an
  explicit follow-on, §7.2)."
- Future out: tagged/color-managed textures (à la OCIO) remain possible — implicit
  extended-sRGB is the degenerate case of a tag system.

### 7.2 Gamut scope — IN SCOPE (decided 2026-10-25; supersedes the 2026-09-12 non-goal)

**Decision:** proper wide-gamut support is a plan goal, not a deferral. Extended sRGB
remains the §7.1 v1 pipeline contract (values are sRGB/Bt.709 primaries), but the plan
carries an explicit wide-gamut workstream — **primaries beyond sRGB, generically**
(pairings added as platforms expose them; the first is Display P3, with BT.2020-class
following when a surface pairing exists). HDR is usually *displayed* on wider-than-sRGB
gamuts; an HDR viewer that cannot emit primaries beyond sRGB is a truncated feature, and
saturated >1.0 colors clip at the extended-sRGB surface today.

**Design rule (user directive, 2026-10-25 — treat as binding):** **Display P3 is NOT a
special case.** It is one of many color spaces the tag + pairing mechanism must support,
and nothing in the architecture may name it, branch on it, or assume it. Every P3
statement in this plan is an *instance* of a primaries-generic rule; when a new primaries
needs support, the work is: add the enum variant, add the pairing row, add the
conversion matrix — no design changes.

**Framing (corrected 2026-10-25 — primaries-generic, not P3-specific):** "wide gamut"
means *any* primaries other than sRGB, handled by the tag + pairing-table mechanism
(§7.1 FloatImage) — no specific color space is privileged in the design. Display P3 is
the first wide-gamut *instance* to pair, chosen for platform exposure (Metal+web), not
by ideology. And "color space" = primaries × transfer: egui tags primaries and fixes the
storage transfer to gamma (§7.1); exotic transfers (PQ/HLG) decode app-side into the
storage contract (§7.5 crumb) — which is also why BT.2020 has no native *gamma* surface
to pair: wgpu's BT.2020 spaces come only as PQ/HLG.

**What wgpu 30 already provides (verified):**
- **A distinct non-extended `SurfaceColorSpace::DisplayP3` exists** (added 2026-10-25,
  round-2 factcheck) — sRGB transfer, P3 primaries, SDR clamp (`is_hdr() == false`):
  Metal reports `DISPLAY_P3` for **every** surface format including u8
  (`metal/adapter.rs:406-415` — only the *extended* flags are fp16-gated); web reports
  it for every canvas format (`webgpu.rs:4154-4160`); DX12 reports no P3 of any kind
  (deterministic SDR fallback). **This is what PR 1 pairs — no fp16 required.**
- `SurfaceColorSpaces::EXTENDED_DISPLAY_P3` exists — Metal reports it unconditionally for
  fp16 (wgpu-hal `metal/adapter.rs:409-415` — carries an upstream OS-version TODO);
  web reports it alongside EXTENDED_SRGB on
  fp16 canvases (`webgpu.rs:4163-4164`). DX12 does **not** (only `EXTENDED_SRGB_LINEAR` =
  scRGB: wide gamut via >1/negative components, Bt.709 primaries). Vulkan: driver-dependent
  via `VK_EXT_swapchain_colorspace`. GLES: none.
- Data layer (per §7.5): `DisplayChromaticity` (Windows/DXGI primaries + white point) and
  coarse `DisplayGamut` (sRGB/P3/Rec2020).

**Leads mined from upstream [#2712](https://github.com/emilk/egui/issues/2712) ("Correct
color on wide-gamut displays", open since 2023-02-10):**
- Demand: emilk explicitly asked "Do you know anyone with a wide-gamut display that can
  help fix this?" (2023-02-10).
- Wumpf's architecture direction (2024-10-23) matches this plan's §2 exactly: each backend
  should apply "color-space-conversion + OETF (both!)" per expected output *in-shader*,
  rather than driver-side `EGL_GL_COLORSPACE_KHR` conversions — because egui deliberately
  blends gamma-encoded ("more perceptual", [#2071](https://github.com/emilk/egui/pull/2071)).
  Tier B's explicit `OutputEncoding` machinery is the generalization point for P3/Rec2020
  OETFs; the wide-gamut pairing is an extension of the same machinery (PR 1/2), and the
  OETF ladder is the encoding work (PR 3/4) — not a new architecture.
- `egui::Rgba`'s color space is undocumented — Wumpf: it is Bt.709 primaries, optically
  linear, and "that this is not speced properly can be regarded as a bug that needs
  fixing". §7.1's contract states primaries explicitly; any wide-gamut color type builds
  on that.
- macOS mechanism (jabuwu, 2025-10-17): P3-Mac correctness comes from `CAMetalLayer
  .colorspace` + ColorSync — wgpu sets it correctly today for sRGB content; glow does not
  (too-bright bug; bancek 2023-06-28 traced an EGL lead:
  `EGL_EXT_gl_colorspace_display_p3`, but glutin ignores surface attributes on the macOS
  cgl backend — consistent with our entry-2 "glow stays SDR" scoping).
- Calibrating the goal: on macOS, *correct sRGB on a P3 display* already works via wgpu.
  The goal is not SDR correctness but the ability to *emit* P3/Rec2020 values, which
  extended-sRGB clips.

**Design sketch — re-staged 2026-10-25 (primaries-generic; split ladder per 29b/29c):**
1. **PR 1 (wide gamut end-to-end — SDR range):** `SurfaceColorSpace::DisplayP3` pairing —
   P3-referred values in [0,1], gamma-encoded, on plain `Bgra8Unorm`/`Rgba8Unorm`
   surfaces (wide-gamut SDR; no fp16 — the capture/kittest fp16 cluster stays in PR 3).
   Opt-in: flipping is the app declaring P3-referred intent; sRGB-referred chrome shifts
   unless the app converts (quantified small — neutrals 0.0 ΔE, accents ≤6.2 ΔE;
   app-side conversion in v1; egui-side chrome conversion is a follow-up if demand
   appears). **Pairing rule: the surface pairs only its own primaries tag; mismatched
   tags reject loudly at `update_texture`** (see §5 PR 1). Demonstrated via a
   paint-callback example showing out-of-sRGB saturation.
   Wide-gamut *content* (tagged float textures, tier C/PR 2) pairs with this policy.
2. **PR 2 (tagged float content — tier C):** `Primaries` (`Srgb | DisplayP3`) +
   `FloatImage` + `ImageData::Float` + upload pairing at `update_texture` (unpaired
   tags reject) + **WGSL negative sign-mirror fix** + ecolor contract tests.
3. **PR 3 (HDR — extended range):** `ExtendedSrgb` + `ExtendedDisplayP3` on fp16 —
   values >1.0, EDR brightness; fp16 capture/kittest readback (entry 34).
4. **PR 4:** scRGB-linear (DX12/Windows) + explicit encoding selection (entry 27).
5. **Later (demand-gated):** full primaries-tagged color API (`Rgba`-level tagging —
   the #2712 fix proper), pairwise primaries↔primaries conversion matrices
   (BT.2020→P3→sRGB as the canonical chain), per-texture
   content conversion (sRGB-referred images on a paired-primaries surface — §5 PR 1's
   reject rule makes this the opting-in app's escape hatch), color-managed
   textures (OCIO-style) if demand appears.

### 7.3 Platform matrix — DECIDED (2026-09-11): macOS-only tested

macOS is verified end-to-end (our app). **Decision:** upstream PR(s) ship with macOS as
the only tested platform; Windows and Linux labeled **experimental** — the probe + SDR
fallback path is exercised, real-HDR output is unverified on real hardware. No hardware
testing commitments before merge.

**Manual protocol (added 2026-10-25):** a `MANUAL_TESTING.md` ships in PR 1 —
per-platform steps, deterministic test patterns, and expected readings (`HdrStatus`,
headroom on an EDR iMac / Pro Display XDR / an SDR panel), each marked "CI cannot
validate this." "Tested" = "this checklist passes"; without it, the experimental labels
are unenforceable. The upstreamed test app is the protocol's instrument: snapshot-free
(deterministic pattern menu + live `HdrStatus`/headroom readout), compile-checked by
CI, visually verified only by the manual pass. This matters from PR 1 — DisplayP3's
out-of-sRGB saturation is only confirmable by eyes on a P3 panel. Additionally, a
**windowed CI smoke test** (the macOS runner has a window server): create a real
window, configure the surface with the P3/extended policy, assert no device errors and
a sane `HdrStatus` — verifies *integration* (a real window system accepts the config);
the *payoff* (compositor tone mapping, panel gamut) remains manual.

Cheap research still wanted (non-blocking):

- **Windows — resolved against wgpu source (2026-10-25; PR numbers per the 29c split):**
  neither PR 1 nor PR 2 pairs on DX12 (PR 2 = tier C; the HDR pair is PR 3). `SurfaceColorSpace::DisplayP3` is never reported
  there (PR 1 → deterministic SDR fallback), and `ExtendedSrgb`/`ExtendedDisplayP3` are
  never reported and cannot be mapped (`map_surface_color_space` → `unreachable!()`) —
  fp16 reports `EXTENDED_SRGB_LINEAR` only (scRGB, `RGB_FULL_G10_NONE_P709`). So PR 3's
  HDR pair is macOS/web-only, and **Windows HDR arrives with PR 4's scRGB policy**
  (`ExtendedSrgbLinear` + the linear fragment path; #8703 demonstrates that combination
  working — on Linux). The nits/chromaticity note stays true for the *data* side (§7.5)
  but says nothing about presentation. Open for PR 4: whether `decide_surface_target`
  gains a per-platform fallback (`ExtendedSrgb` → `ExtendedSrgbLinear` on DX12).
  Factcheck nuance (2026-10-25): DX12 *does* report `Rgb10a2Unorm` with `BT2100_PQ` —
  an HDR10 path; it is out of the v1 transfer contract (PQ ≠ gamma storage), so the
  accurate statement is "no *extended-sRGB-family* HDR on DX12"; HDR10/PQ output would
  be its own future pairing.
- **Linux/Wayland — resolved against wgpu 30 source (2026-10-25)**: wgpu speaks no
  Wayland color-management protocol (no `wp_color_management`/frog/xx-color code exists
  anywhere in wgpu-hal/core 30); the only lever is `VK_EXT_swapchain_colorspace`
  (requested unconditionally, `vulkan/instance.rs:280-282`) feeding driver-reported
  caps. Under PR 1's policy (`DISPLAY_P3`) Linux is probe-gated SDR — desktop Vulkan
  drivers essentially never expose P3 swapchain spaces [INFERENCE from driver ecosystem].
  Under PR 4's scRGB policy, Linux **works today on KWin** (#8703 demonstrated: fp16 +
  `EXTENDED_SRGB_LINEAR`, live HDR toggle, ramp to 4.0 brighter than SDR white) — and
  that path, not PR 1/2, carries the "experimental: a compositor may honor scRGB without
  an active HDR mode → wrong brightness" label.
- **Web**: canvas float16 + color spaces — deferred.

Also from wgpu docs: wgpu never writes HDR metadata (`vkSetHdrMetadataEXT` / DXGI
`SetHDRMetaData`); only relevant if a PQ/HDR10 output mode ever enters scope — the
extended-sRGB path needs none.

### 7.4 Reporting/fallback API shape — DECIDED (2026-09-19)

**Reporting:** poll-based. `RenderState` holds the authoritative `HdrStatus` — an enum,
not a bool, carrying the fallback *reason* (`Hdr { .. }` / `Sdr { reason }`, reasons at
least `Disabled`, `NoExtendedSrgb`, and `NoSurface`; aids bug reports).
**Pinned payload (decided 2026-10-25):** `HdrStatus::Hdr { format: TextureFormat,
color_space: SurfaceColorSpace }` — mirroring `decide_surface_target`'s outputs, so a bug report
*shows the decision* without re-running the probe — and `HdrStatus::Sdr { reason:
SdrReason }` with a closed `SdrReason` enum (`Disabled`, `NoPairedColorSpace { requested:
SurfaceColorSpace }`, `NoSurface`; generalized 2026-10-25 — one self-describing variant
that never grows as pairings are added, replacing the P3-era `NoExtendedSrgb`).
Derives `Clone, Copy, Debug, PartialEq, Eq` (matches neighboring egui-wgpu types).

**SurfaceColorSpacePreference shape (decided 2026-10-25; renamed from the round-1
`HdrPreference` sketch by the 29c split):** `{ Off, Auto, DisplayP3, ExtendedSrgb,
ExtendedDisplayP3 }` — unit variants, keeping `Clone + Copy + Debug + PartialEq + Eq +
Hash` cheap. `DisplayP3` = wide gamut, SDR range, on u8 surfaces (PR 1); the
`Extended*` pair = HDR brightness on `Rgba16Float` (PR 3). `Auto` = "probe; best
available (the extended pair when fp16+extended is supported, else `DisplayP3` where
supported), else SDR" (matching the §1 sketch; whether `Auto` ever selects plain
`DisplayP3` — turning the chrome shift on for the endgame default — is deliberately
open, round-2 entry 21) — **not** wgpu's `SurfaceColorSpace::Auto` (which resolves fp16
→ `ExtendedSrgbLinear`); the doc must carry that disclaimer. Defaults: `Off` = upstream
rollout default; `Auto` = endgame default once capture/kittest fp16 infrastructure
exists; our fork/app opts in unconditionally. Fail-hard = recipe, not variant: apps
that require HDR check `hdr_status` after create (the payload above makes the answer
complete); no `Required { .. }` variant in v1. No `#[non_exhaustive]` — egui-wgpu house
style carries it on no enums.
**Reporting
mechanism corrected 2026-10-25 (three defects in the original sentence):**

- **There is no refresh point.** The original "refreshed at reconfigure/recreate (the
  same points `configure_surface` runs)" does not exist: `configure_surface` runs on
  every resize but re-probes nothing — `RenderState::create` runs exactly once
  (`render_state.is_none()` guard, winit.rs:258) and the app-visible `RenderState` is a
  one-shot clone. `HdrStatus` is therefore **create-time-immutable** — same lifetime as
  entry #12's create-time-only policy. Document that; don't promise refresh.
- **No `frame.info()` mirror.** The mirror would live in eframe's `IntegrationInfo` (a
  different crate), contradicting tier B's "egui-wgpu only" and the small-PR budget —
  and it would be a third copy (Painter's, the Frame clone, the mirror) with three
  staleness stories. `Frame::wgpu_render_state()` **already exists** (epi.rs:943), so
  apps read `hdr_status` today with zero new plumbing: `frame.wgpu_render_state()
  .map(|rs| rs.hdr_status)`. Cut the mirror from PR 1.
- **Headroom is per-viewport and does not belong in a global struct.** `Frame::info()`
  returns one global `IntegrationInfo`, but `display_hdr_info()` is per-surface and
  differs across monitors — the exact multi-viewport case egui supports. The future
  `DisplayHdrInfo`/headroom pass-through goes in **its own later PR**, targeting the
  per-viewport channel (`ViewportInfo`, egui/src/data/input/viewport_info.rs:28) — a
  cross-crate design decision (wgpu types can't be mirrored into `egui`), not a side
  item.

No callback/event: HDR *state* changes rarely, immediate-mode per-frame reads are free,
and create-time-immutable status suffices for every plausible consumer (badges, tone-map
switches, slider disabling). Headroom is the live quantity and is polled per frame
through its own (later) channel.

**Per-viewport:** split the question.

- *Format/capability is global.* One `RenderState { target_format, renderer }` is shared
  by all viewports ("all egui viewports share the same painter" — baked pipeline targets).
  An `Rgba16Float + ExtendedSrgb` surface is valid on an SDR display too (values >1.0
  just clip/composite down), so a global fp16 choice is harmless everywhere and the
  shared Renderer stays shared.
- *Headroom is per-surface.* `display_hdr_info()` is queried per surface anyway (§7.5
  `DisplayHdrInfo` pass-through) — that's the live per-display quantity.
- Residual policy — **recorded 2026-10-25** (matches existing code; no behavior change):
  (1) probe input = first surface's caps (`RenderState::create` runs once,
  `render_state.is_none()` guard); later viewports inherit the stored decision; a late
  viewport that cannot configure fails as an uncaptured wgpu device error — accepted,
  per-viewport fallback out of scope (no macOS impact — Metal reports `EXTENDED_SRGB`
  unconditionally; multi-adapter Linux/Windows scenarios documented-only). (2)
  No-surface/headless (kittest): probe cannot fire over synthesized `vec![Rgba8Unorm]`;
  `HdrStatus` = `Sdr { reason: NoSurface }` (new `SdrReason` variant, defined with the
  status types). (3) Err domain unchanged: `NoSurfaceFormatsAvailable` only; HDR
  unavailability is a status (`HdrStatus`), never `Err`.

**Format-selection API (supersedes the earlier "richer sibling" sketch):**

- **One public function, new name** (decided 2026-10-25; replaces the "keeps its name"
  bullet): `decide_surface_target(caps: &SurfaceCapabilities, pref: SurfaceColorSpacePreference) ->
  Result<SurfaceTarget, WgpuError>` with `SurfaceTarget { format, color_space, status }`
  — a named struct, not a tuple (the old internal `decide_target` tuple had the same
  smell). A fn named "framebuffer_format" returning status+color-space fails the
  anti-drift test; and the input *must* change from `&[TextureFormat]` to
  `&SurfaceCapabilities` regardless: wgpu 30 exposes HDR-capable formats only in
  `format_capabilities` with per-format color spaces, which a slice cannot see. Because
  the projection disappears (entry 26 removes `preferred_color_space`),
  `decide_surface_target` **is** the internal decider; `RenderState::create` stores its
  three outputs (`target_format`, `target_color_space`, `hdr_status`) and
  `configure_surface` reads the stored color-space field. Err domain:
  `NoSurfaceFormatsAvailable` only (entry 13). Migration: if this lands as hardening on
  a merged #8703, its `preferred_hdr_framebuffer_format` sibling is deprecated one
  release while `decide_surface_target` lands; if not, the in-place cut follows the
  0.36.0 precedent below. Rationale for the hard cut: egui's 0.x culture is routine
  breaking changes — 0.36.0 shipped the wgpu 30 major bump *and* hard in-place removals
  (`Modifiers` off `RawInput` [#8336](https://github.com/emilk/egui/pull/8336),
  `clip_rect_margin` [#8366](https://github.com/emilk/egui/pull/8366)); a loud compile
  error beats silent semantic drift; and no-overloading means the 1-arg form can't
  coexist anyway. Deprecate-first (CONTRIBUTING.md:92) has a limit here: whatever shape
  survives, the old signature's failure under an HDR preference is *silent* — it keeps
  predicting egui's choice without the HDR input — so if it is kept at all it must be
  deprecated with a doc-visible "does not consider HDR" warning; if replaced, the
  removal follows the 0.36.0 precedent above. The *deceiving* variant is keeping the
  old function untouched beside a sibling — its implicit "this predicts egui's choice"
  identity silently fails under `SurfaceColorSpacePreference::ExtendedSrgb`.
- ~~`preferred_color_space`~~ — **removed 2026-10-25** (entry 25's simplification):
  color-space support is per-format in wgpu 30
  (`SurfaceFormatCapabilities { format, color_spaces }`), so a
  `(caps, hdr) -> SurfaceColorSpace` projection must secretly re-pick a format, and its
  totality masks errors (the sibling returned `Err` on empty caps; this returned `Auto`
  — asymmetric). A color space is never consumed unpaired: internals and hand-rolled
  users both get it from `decide_surface_target`'s coherent `SurfaceTarget`, and
  diagnostics read it from `HdrStatus::Hdr`. `configure_surface` reads the stored
  `color_space` field from `RenderState` — same mechanics as `target_format` today.
- **The decider is the public function** (simplified 2026-10-25):
  `decide_surface_target` (above) is the single source of the decision — no separate
  internal function, no projections. `RenderState::create` stores all three outputs
  (`target_format`, `target_color_space`, `hdr_status`). `configure_surface` reads the
  stored color-space field — no derivation match anywhere; nothing "assumes" a color
  space, the decision *has* one, stored and named. Coherence is by construction, not
  discipline; the probe re-run is free (`caps.color_spaces()` reads an in-memory struct).
- **Invariant unit test**: `decide_surface_target` never returns an incoherent triple
  (`(Rgba16Float, Auto)`, `(u8-format, ExtendedSrgb)`, or the double-grammar pair
  `(Bgra8UnormSrgb, DisplayP3)` — wgpu would *accept* that combo and it violates the
  one-grammar rule) — catches drift in the act (fixtures from `format_capabilities`,
  entry 35).
- **PR-4 evolution**: only `decide_surface_target`'s pairing table grows (the scRGB
  variant, PR 4) and the `HdrStatus` reason set; public signatures survive; *variant
  sets* grow — a 0.x-visible change, normal for egui (reworded 2026-10-25; "survive
  unchanged" oversold API stability).

### 7.5 Calibration: nits vs multipliers — RESOLVED via wgpu source (2026-09-11)

No egui research exists to do — egui has no luminance/HDR API at all. wgpu 30's
`DisplayHdrInfo` (wgpu-types/src/surface.rs:587-639) already surfaces everything each
platform offers, and the split is stark:

- **Windows (DXGI)**: absolute nits — `DisplayLuminance { max_nits,
  max_full_frame_nits, min_nits, sdr_white_nits }`, with `sdr_white_nits` moving with
  the brightness slider. Plus `DisplayChromaticity` (primaries + white point, CIE 1931
  xy). True-nits readouts are feasible on Windows.
- **macOS (Apple EDR)**: relative multipliers only — `DisplayHeadroom { current,
  potential, reference }`, read from AppKit `NSScreen` (via the hosting `NSWindow`).
  **Populated only on macOS**: the iOS/tvOS/visionOS branch is an in-code TODO naming
  `UIScreen.currentEDRHeadroom` as the path (`wgpu-hal metal/surface.rs:180-182`) — an
  acknowledged upstream-wgpu gap, not a platform limit (UIKit exposes the equivalent EDR
  APIs; wgpu's own field docs cite them). wgpu docs are explicit: "Apple exposes no
  absolute-nit equivalent … can't be converted to nits." Nits readouts are
  architecturally unavailable on macOS. Note: on macOS only `headroom` + the coarse bit
  are filled, and `high_dynamic_range` tracks the *live* multiplier (`current > 1.0`) —
  "HDR active right now", **not** "display is HDR-capable".

Consequences:

- The **universal currency for HDR UI is stops / headroom multiplier** (what our test
  app shows). Nits can only be a Windows-only best-effort annotation.
- The egui side-item should pass through the whole `DisplayHdrInfo` via `RenderState`
  (entry 22 cut the `frame.info()` mirror; the per-viewport `ViewportInfo` pass-through
  is its own later PR) — not a distilled headroom `f32`.
- `tone_map_headroom()` precedence (verified `surface.rs:732-757`) is **four steps**:
  Apple multiplier → `Some(1.0)` when the display is definitively SDR
  (`coarse.high_dynamic_range == Some(false)` — phantom-headroom guard) → Windows
  `max_nits / sdr_white_nits` → `None`. Design consequence for `HdrStatus`:
  `Some(1.0)` is a valid "no headroom" steady state; `None` means *unknown* — the two
  must not be conflated.
- `DisplayGamut` is `#[non_exhaustive]` and explicitly **unordered** ("do not rely on
  their declaration order to compare gamut sizes", `surface.rs:696-703`) — pass it
  through; never match or order it in egui-side code (matters for the §7.2 wide-gamut
  workstream).
- Remaining open crumb (app-level, not egui/wgpu): mapping absolute-encoded content
  (PQ / ST 2084 — the future AVIF viewer) onto EDR requires an SDR-white-in-nits
  assumption on macOS since the OS won't report one. **Use 203 nits (ITU-R BT.2408) as
  the anchor** — the current industry convention; the older 100-nit figure renders
  PQ ~2× too bright. On Windows, no constant is needed: use `sdr_white_nits` from
  `DisplayLuminance` exactly (it moves with the brightness slider).

**PQ ingest chain (specified 2026-10-25 — app-implemented; egui ships none of it):**
1. **Decode:** `PQ⁻¹(E')` per BT.2100, in **BT.2020-referred linear light** — never the
   sRGB inverse (≈2.12× brightness error at reference white).
2. **Scale:** decoded values are absolute nits-referred; normalize by the 203-nit
   anchor (1.0 = SDR white; anchor decision above).
3. **Convert primaries in linear light:** BT.2020 → the paired primaries (e.g. P3) via
   the 3×3 matrix — matrices are only valid pre-transfer. This step *emits negative
   components* for BT.2020-beyond-P3 colors (covered by the entry-18 odd-symmetric
   contract).
4. **Encode:** apply the paired primaries' gamma encode — write into the `FloatImage`.
   Do not re-apply any decode (double-encode = washed out).

HLG: out of scope — the `Primaries` tag carries primaries only, not transfer; HLG
content needs an app-side HLG⁻¹ (same chain shape) or a future transfer-carrying tag.

### 7.6 Tier E's tessellator format controversy (deferred, but shapes the debate)

Always-float vertices double memory/bandwidth for every app — upstream won't take that.
Alternatives: dual-format meshes (complexity in every backend), sidecar float color buffer
(ugly), pack RGBA as f16×4 = 8 bytes in the existing layout slot + adjacent u32 (stride
20→24, needs vertex format juggling). Not needed until E, but expect this to be the
stickiest review thread when it lands.
Type decisions to settle before E (added 2026-10-25): (1) `Eq` drops through
`Vertex → Mesh → Shape` — in *both* cfg variants including `unity` (reordered fields:
the packing trick must work in both orders; verified `mesh.rs:8-60`); (2) serde repr of
the color field (Color32 serializes as a `#rrggbb` string — f16×4 needs a chosen repr;
`half/serde` serializes as float); (3) `half`'s `bytemuck` feature becomes epaint's
direct surface (`Pod` for `[f16; 4]`).

### 7.7 Testing / QA story — fp16 mechanism is a PR-3 deliverable (corrected 2026-10-25)

The original text assumed kittest could already test fp16 encoding — false at 0.36.2 and
HEAD, three ways: (1) kittest readback is hardcoded 4 B/px into `RgbaImage` — on fp16 the
copy buffer is half the required size → wgpu validation error → panic (verified
`texture_to_image.rs:81`); (2) the headless probe cannot fire — kittest passes
`compatible_surface: None`, so `RenderState::create` synthesizes `vec![Rgba8Unorm]`
(lib.rs:267) and extended-range preferences silently no-op; (3) `capture` fails *silently*
on fp16 (`read_screen_rgba` logs + returns — no screenshot delivered), not "lies" as §4
claimed.

**PR 3 deliverables (named; kittest must support fp16 before the HDR extension lands —
decided 2026-10-25):**
1. Float readback — **in egui-wgpu**, next to `capture.rs` (sharing #8703's
   f16-decode/padding code — one implementation, two consumers: capture and tests):
   `texture_to_float_image` (f16→f32) for numeric assertions; `egui_kittest` keeps only
   a clamp-to-white u8 path so existing goldens keep working for SDR-region appearance.
   ~60–100 lines; mind 256-byte `bytes_per_row` alignment.
2. Headless format override: `RenderState` fields are `pub` and
   `Harness::builder().renderer(WgpuTestRenderer::from_render_state(...))` **already
   works today** (no builder changes needed — corrected 2026-10-25); the real work is
   making egui_wgpu's no-surface stub consult an explicit format override (doubles as
   the §1 power-user escape hatch). Document the builder hook as the standard test
   recipe.
3. fp16 branch in `read_screen_rgba` + a loud error/`Result` instead of the silent
   early return.
4. MSAA plumbing in the offscreen harness (`sample_count` + `resolve_target` through
   `WgpuTestRenderer::render`, which hardcodes 1/None at `wgpu.rs:202/217`) — without
   it the entry-38 MSAA closure cannot run in CI.

PR 1 (DisplayP3 on Rgba8) is unaffected — u8 readback keeps working, which is part of
why the split works.

**Encoding-contract test matrix (added 2026-10-25; mapped to existing test conventions
— survey: ecolor 14 inline tests, epaint 96, egui 102, kittest snapshots in
`egui_demo_lib/tests/`, egui-wgpu zero runtime tests):**
- **Contract properties → `ecolor` inline tests** (following `test_rgba_roundtrip`,
  `test_color32_blend_vs_gamma_blend`): `gamma_from_linear ∘ linear_from_gamma ≈ id`
  incl. negatives (odd-symmetry, entry 18); premultiply rule (contract clarifications);
  exposure decomposition/chroma roundtrip (entry 30).
- **`decide_surface_target` property tests → `egui-wgpu` unit tests** (the crate's first
  real runtime tests — #8703 sets the precedent; pure functions over synthetic caps, no
  GPU needed): fixtures built from `format_capabilities` (Metal-like: fp16 +
  EXTENDED_SRGB/P3; DXGI-like: LINEAR only; float-without-extended; empty) — HDR formats
  live in `format_capabilities`, NOT `caps.formats` (wgpu-types surface.rs:506-548), so
  `formats`-based fixtures never exercise the probe. Assert: returned pair is supported
  by the input caps; `HdrStatus` reason consistent with the choice; totality on empty.
- **Encoding-contract numeric assertions → `egui-wgpu` tests with the #34 hand-built
  fp16 `RenderState`** (adapter-requiring, like today's kittest tests; runners:
  macos-latest + lavapipe where available; tolerances sized for adapter drift): known
  linear → `gamma_from_linear` within f16 quantization (±2⁻¹¹ relative in the normal
  range — the full-ULP bound is ±2⁻¹⁰ — with an explicit subnormal carve-out:
  relative-in-log-space or an absolute floor, else the test flakes on dark pixels;
  corrected 2026-10-25); 2.0 and
  near-f16-max survive upload→shader→blend; negatives per contract (needs the entry-18
  WGSL fix); dither off on fp16 → bit-identical repeat renders; [0,1] SDR scene matches
  u8-path output within tolerance.
- **Visual coverage → kittest snapshots in `egui_demo_lib/tests/`** (following
  `color_picker.rs`, `image_blending.rs`): HDR swatch + picker UI snapshots —
  SDR-region appearance only. u8 goldens are structurally blind to >1.0 (2.0 and 10.0
  clamp identically — a clamp-to-1.0 regression matches goldens perfectly), so HDR
  *values* are guarded solely by the numeric assertions above.
- **Named low-value (don't add):** single-case decider tests; u8 goldens of HDR values;
  runtime `frame.info()` asserts (needs a window); raising thresholds to make tests pass.
- **Tier E (someday):** `epaint` inline packing/stride tests + mesh serde, per its
  96-test inline pattern.

**CI-vs-manual split (stated once):** CI validates the pure decider + offscreen
pipeline — egui's GPU tests run on macos-latest only (Linux/Windows are clippy/doc),
and kittest prefers CPU adapters (lavapipe) where available, so dev machines and CI
exercise different adapters; fp16 tolerances must absorb that drift. The windowed CI
smoke test (§7.3) verifies *integration* only — a real window accepts the surface
config. The windowed-surface *payoff* — compositor tone mapping, panel gamut, headroom
readings — is manual-matrix only (§7.3 / MANUAL_TESTING).

### 7.8 Feature interactions to verify (not design)

**Resolved 2026-10-25** (these were "verify" items; entry 35 wired the CI path, entry
34 owns capture):
- **MSAA resolve on fp16** — Metal grants `MULTISAMPLE_RESOLVE` for `Rgba16Float` on
  all realistic hardware except `apple1x` (pre-Apple-silicon); egui's intermediate
  target just uses `target_format` — no change needed. CI-testable: kittest
  `with_render_options(msaa_samples: 4)` on an fp16 state (entry 35). Harness
  prerequisite (added 2026-10-25): the offscreen renderer hardcodes `sample_count: 1`
  / `resolve_target: None` (`wgpu.rs:202/217`) — MSAA plumbing is part of entry 34's
  harness deliverables; until it lands, the CI test cannot run.
- **`present_with_transaction` coexistence** — the 0.36.2 jitter fix is a latency clamp
  (`desired_maximum_frame_latency ≥ 2` during live resize) plus a cargo feature; it
  never touches `color_space`, and reconfigure rebuilds from the stored decision — HDR
  survives by construction. Downgrades to a manual-QA checkbox: "jitter-fix + HDR +
  drag-resize."
- **Transparent windows (alpha × HDR)** — no cross-constraint:
  `validate_surface_configuration` checks color space and alpha mode in independent
  blocks; Metal reports `[Opaque, PostMultiplied]` and egui picks `PostMultiplied`;
  valid at the validation level everywhere (DX12's break is its missing `ExtendedSrgb` —
  entry 10 — independent of alpha). Manual-QA checkbox: compositor behavior of
  premultiplied >1.0 content [INFERENCE: fine on macOS].
- **`WgpuSetup::Existing` + probe ordering** — the probe reads whichever compatible
  surface's caps it is given, regardless of who created the device/instance (entry 13's
  first-surface rule). One documented dependence: on Vulkan,
  `VK_EXT_swapchain_colorspace` is an *instance* extension — wgpu requests it
  unconditionally when creating its own instance; a user-supplied instance without it
  reports no extended spaces → correct SDR fallback. [INFERENCE from verified mechanics:
  `instance.rs:280-282` + probe-from-caps.]
Missing from this list (added 2026-10-25; cross-referencing decided entries): (a) the
harness readback path itself (entry 34 — CI-testable); (b) `egui_glow` keeps compiling
and rendering SDR with the new types (entry 2 — compile-gated); (c) wasm compile gate:
kittest excludes BROWSER_WEBGPU and the web job is check-only — "wasm keeps compiling
with the new types," no runtime claims (CI-checkable); (d) multi-viewport × capture:
`screen_capture_state` is one shared `Option<CaptureState>` across viewports with
per-window sizes — needs a test independent of HDR (CI-testable); (e) tier C/D Eq/serde
cascades (entry 28); (f) dither-suppression precedence (entry 20).

### 7.9 Process / strategy

- **Engage with PR [#8703](https://github.com/emilk/egui/pull/8703) instead of competing
  with it** (decided 2026-10-25). JordyUSA's PR already ships tier B:
  `SurfaceConfig::prefer_hdr: bool` + `preferred_hdr_framebuffer_format()`, picking
  `Rgba16Float` + `EXTENDED_SRGB_LINEAR` via `SurfaceColorSpace::Auto`, switching to the
  linear fragment path for fp16 targets, and fixing fp16 capture readback (+105 lines in
  `capture.rs`; ~+200/−12 total across exactly the files this plan names). Tested on
  Linux/Wayland (KWin, ramp to 4.0 verified brighter than SDR white, live HDR toggle
  works); **untested on macOS and Windows**; production use in LightCraft. Open since
  2026-10-12, merge-blocked, zero maintainer comments. Strategy:
  (a) review it *locally* (fetch the branch, run our app against it on macOS EDR) — its
  `Auto`/scRGB-linear choice needs exactly the §2/§7.1 scrutiny; the public review
  comment is deferred with all upstream engagement (see the issue-draft bullet below);
  (b) test the PR branch against our app on macOS EDR — fills its biggest testing gap and
  gives us standing;
  (c) position our `SurfaceColorSpacePreference` policy enum + `HdrStatus` (§7.4) as a *follow-up
  hardening PR* that evolves the merged bool — not a competing first PR;
  (d) re-verify tracker status as a dated sentence before drafting any issue text.
  (e) **How we differ from #8703 (recorded 2026-10-25, for the future issue draft):**
  #8703 turns on brightness (scRGB-linear via `SurfaceColorSpace::Auto`, `prefer_hdr`
  bool on runtime-mutable `SurfaceConfig`, sRGB gamut, Linux-tested); this plan builds
  the color *architecture* — create-time `SurfaceColorSpacePreference` policy, explicit
  named pairings (never `Auto`-resolution), primaries-tagged gamma storage contract,
  wide gamut (P3 first, primaries-generic pairing table), `HdrStatus` diagnostics,
  macOS-first verification. PR 1 is positioned as the follow-up to #8703's brightness
  work, not a competitor for it.
- **All upstream engagement deferred** (decided 2026-10-25): no issue, no PR, no public
  comments — we fix this for ourselves first, on the fork. The material below is the
  *prepared* issue draft for when that changes. Hook order: (1) the #7484-class
  failure + one-paragraph repro (maintainers triage on user-visible breakage, not color
  science); (2) the encoding trap — ExtendedSrgb vs ExtendedSrgbLinear, why wgpu 30 +
  naive format selection burns people; (3) proposed API + non-goals; (4) testing story.
  Anchor each PR to a live thread and reference #8703 in each: PR 1 (wide gamut) →
  [#2712](https://github.com/emilk/egui/issues/2712) (the wide-gamut request — dormant,
  but the only live thread for it), PR 3 (HDR extended range) →
  [#7484](https://github.com/emilk/egui/issues/7484) (recreate semantics + black screen;
  reporter confirmed double gamma, fix "has to be done on the egui side") plus the
  LightCraft production demand from #8703, PR 4 (explicit encoding + scRGB) →
  [#6863](https://github.com/emilk/egui/issues/6863) (removes the "linear (sRGBA aware)
  framebuffer" warning by making encoding explicit). Pre-empt the three predictable
  objections, one line each: scope (egui-wgpu only, opt-in, glow/web unchanged),
  maintenance (invariant-tested decider + per-tier tests per §7.7), CI-without-HDR-
  hardware (encoding testable headless; display interpretation is manual-matrix; macOS
  verified by us, Linux by #8703's author).
- **Build on a fork first; `[patch]` our app onto it; upstream only after the app proves
  the design** (decided 2026-10-25, superseding this bullet's earlier open question). The
  app is the verification vehicle: implement the tier-B hardening (`SurfaceColorSpacePreference`/
  `HdrStatus`, §7.4 — composed with PR #8703's approach per the bullet above) on a fork of
  this repo, switch the app to it via a `[patch]` override, and only open upstream PRs
  once the app demonstrates everything we need. Known cost, accepted: while upstream moves
  (213 commits since 0.36.2 in our HEAD alone; wgpu majors most releases) the fork needs
  rebases, and a `[patch]`ed app can't ship until the fork tracks a tagged release.
  Mitigation: keep the fork's delta small and upstream-shaped so the eventual PRs are a
  fast-forward, not a rewrite.
- **Re-anchor before implementing** (from review, decided 2026-10-25): Appendix anchors
  were verified against 0.36.2, which is *not* an ancestor of this checkout (213/16
  commits diverged). At HEAD, `preferred_framebuffer_format` already returns
  `Result<TextureFormat, WgpuError>` (lib.rs:421), `WgpuError` exists (:46), and
  `WgpuConfiguration::with_surface_config` exists (:376) — §7.4's loud-break rationale
  must be restated (remaining break = caps parameter + richer return, not Result).
  Sequence of re-verification: (1) Appendix anchors → against this checkout's HEAD before
  writing fork code; (2) API surface → against #8703's branch when composing; (3) →
  against upstream main only when drafting PRs.
- Timing (updated 2026-10-25): **0.37 shipped 2026-10-15 (#8725)** — the ladder
  realistically lands 0.38+. Drift note: #8714 (merged 2026-10-19) privatized
  `ColorImage` and renamed `size`→`resolution` — the `FloatImage` sketch in §7.1 is
  shape-stale against main; final shape = constructor-based API per post-#8714 house
  style, re-verified when PR 2 starts (entry 4's re-anchor workstream).

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
- `DisplayHeadroom { current, potential, reference }` — Apple only, macOS-only
  population (iOS/tvOS/visionOS = in-code TODO naming `UIScreen.currentEDRHeadroom`),
  explicitly not convertible to nits — :644-662
- `DisplayChromaticity { red, green, blue, white }` — Windows/DXGI only — :662-673
- `DisplayCoarseRange { high_dynamic_range, gamut: DisplayGamut }` — web + cross-check —
  :675-706
- `tone_map_headroom()` precedence: Apple multiplier → SDR-display `Some(1.0)` guard
  (`coarse.high_dynamic_range == Some(false)`) → Windows `max/sdr_white` → None —
  :732-757
