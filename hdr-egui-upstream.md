# egui HDR and wide-gamut enablement plan

egui cannot display HDR or wide-gamut content: the surface format is hardcoded to 8-bit
sRGB, the color space is never configured, and every color in the pipeline caps at 1.0.
This plan makes HDR and wide gamut first-class on the fork, proves the design against a
real app, then carries it upstream as a sequence of small, fully-consumed PRs.

**Posture:** we build on a fork of this repo and point the app at it via `[patch]`;
upstream engagement is deferred until the app proves the design (§10). Upstream already
has a competing HDR PR (#8703, scRGB-linear, blocked) — this plan is positioned as its
follow-up: #8703 turns on brightness; we build the color architecture (policy, contract,
gamut, diagnostics) that brightness plugs into.

## Architecture at a glance

- **Surface policy** — `SurfaceColorSpacePreference` (create-time, on `WgpuConfiguration`)
  → `decide_surface_target` → `SurfaceTarget { format, color_space, status }`; the
  status is app-readable via `frame.wgpu_render_state()`.
- **Encoding** — `OutputEncoding::of(format, color_space)`, derived at `Renderer::new`,
  never stored.
- **Content contract** — float colors are gamma-encoded in the buffer's declared
  primaries, premultiplied in the encoded components, odd-symmetric for negatives.
- **Wide gamut** — content carries a `Primaries` tag; a pairing table decides which
  surface displays each tag natively; unpaired tags reject loudly.
- **Testing** — kittest fp16 readback (in-repo), numeric encoding assertions, a
  windowed CI smoke test, and a per-platform manual protocol.

## Ladder

| PR | Ships | Consumer | Tests |
|---|---|---|---|
| **1** | Wide-gamut surface: `SurfaceColorSpacePreference {Off, Auto, DisplayP3}`, `decide_surface_target`, `HdrStatus`, configure threading; callback demo with out-of-sRGB saturation | any eframe app opting in | decider property tests, u8 snapshots, windowed smoke |
| **2** | Tagged float content (tier C): `Primaries`, `FloatImage`, `ImageData::Float`, upload pairing, `ui.image()` float display, WGSL negative fix, ecolor contract tests | HDR/wide-gamut images via `ui.image()` | upload roundtrip, ecolor properties, snapshots |
| **3** | HDR extended range: `ExtendedSrgb` + `ExtendedDisplayP3` on `Rgba16Float` (>1.0, EDR); `Auto`; dither off on fp16 | brightness above SDR white | fp16 readback numeric tests, negative GPU round-trip, capture branch, MSAA |
| **4** | scRGB-linear policy (Windows/DX12) + explicit encoding selection + #7484 semantics | Windows HDR | per-platform (manual-heavy) |
| **5** | Tier D: `color_edit_button_hdr` + swatch widgets | HDR editing UI | picker snapshots, decomposition tests |
| — | Side items: `DisplayHdrInfo` per-viewport pass-through, docs, test app | adoption | compile-only + manual |
| — | Deferred: tier E (float vertex colors) | vector-shaped HDR | — |

**Binding rules:** every PR ships consumed, testable functionality — nothing dead,
nothing untestable at merge. Display P3 is not a special case: the mechanism is
primaries-generic, and adding a primaries = enum variant + pairing row + conversion
matrix, in one PR, with no design changes.

## 1. Background and evidence base

The app-side workaround this plan replaces lives in `../hdr-test/rust`: a hand-rolled
winit loop + `egui` + `egui-winit` + `egui_wgpu::Renderer` owning the
`SurfaceConfiguration`, because eframe cannot create an HDR surface (§2). It renders a
2×2 quad driven 0–4 stops above SDR white by a `CallbackTrait` shader, an orange marker
where EDR headroom falls, and a live headroom label polled on the main thread
(250 ms idle repaint). Choices it validated working end-to-end on macOS M1:
`Rgba16Float` + `SurfaceColorSpace::ExtendedSrgb` (not `ExtendedSrgbLinear` — §3), a
custom shader with the sRGB curve unclamped above 1.0, dithering off, SDR fallback to
the first reported format, and no runtime HDR toggle (a format change requires
recreating `egui::Context` + `egui_winit::State` + `egui_wgpu::Renderer` together).

Upstream facts the plan builds on:

- **#7484** (open): switching a surface to `Rgba16Float` at runtime and recreating the
  renderer turns the app black — the *partial-recreate* failure (stale `Context`
  caches; the reporter confirmed double gamma and that the fix belongs in egui).
- **#8703** (open, blocked, no maintainer review): opt-in `prefer_hdr: bool` on the
  runtime-mutable `SurfaceConfig`, picking `Rgba16Float` + `EXTENDED_SRGB_LINEAR` via
  `SurfaceColorSpace::Auto`, linear fragment path, fp16 capture fix (+105 lines).
  Hardware-tested on Linux/Wayland (KWin); untested on macOS/Windows; production use in
  LightCraft. PR 1 is positioned as the follow-up to this brightness work.
- **#2712** (open, dormant): wide-gamut color request; emilk asked for wide-gamut-display
  help; Wumpf documented that `Rgba`'s primaries are Bt.709/sRGB and that leaving them
  unspecified "can be regarded as a bug that needs fixing".
- **#6863** (open): the "linear (sRGBA aware) framebuffer" warning is stale — egui
  handles sRGB attachments by skipping the shader decode.
- **kittest layout**: the external `kittest` crate (sibling checkout, 0.5.0) is an
  accesskit query layer only; `Harness`, the builder, snapshot machinery, the wgpu
  renderer, and readback all live in-repo in `crates/egui_kittest`.
- **egui_glow** exists and is functional (opt-in eframe feature, kept for binary size).
  HDR is wgpu-only by capability: OpenGL/GLES has no extended-sRGB swapchain concept and
  wgpu's GLES backend reports `SRGB`-only color spaces. The glow path stays SDR by
  design.

## 2. Surface policy (PR 1 + PR 3)

Three hard-coded decisions block HDR today, all in `egui-wgpu`: the format picker only
accepts 8-bit formats, `SurfaceConfig` exposes no format or color space, and
`configure_surface` leaves `color_space` at `Auto` — which wgpu resolves to
`ExtendedSrgbLinear` (scRGB) for fp16, so a smuggled fp16 surface is extended *linear*,
misinterpreting egui's gamma-encoded output. #7484's black screen is the separate
*partial-recreate* failure. Explicit policy beats accidental behavior.

**Policy type** — on `WgpuConfiguration`, create-time-only:

```rust
pub enum SurfaceColorSpacePreference {
    Off,               // today's behavior — the rollout default
    Auto,              // probe; best available, else SDR — the endgame default
    DisplayP3,         // PR 1 — wide gamut, SDR range (u8 surface, P3-referred)
    ExtendedSrgb,      // PR 3 — HDR brightness (Rgba16Float + EXTENDED_SRGB)
    ExtendedDisplayP3, // PR 3 — wide gamut + HDR brightness (Rgba16Float)
}
```

Why create-time-only, and not on the runtime-mutable `SurfaceConfig`: a format change
invalidates baked pipeline targets, and the only safe mid-run recreate is impossible as
an eframe API — apps hold `egui::Context` by value. A renderer-only recreate loses
`Renderer.textures` (font atlas, uploads, `register_native_texture` bind groups) while
egui ships per-frame *deltas*, so meshes reference missing textures until every image is
re-touched; re-registering user native textures has no recipe. Half-measures fail
silently — worse than #7484's visible failure. Full recreate remains an app-level
technique (the workaround app does exactly this); it is not an upstream deliverable.
Runtime HDR changes are therefore a documented non-goal.

Why a policy enum names targets instead of exposing `format` + `color_space` knobs:
naive pairing lands in #7484 territory (double gamma, black screens). `Auto` means
"probe; best available (the extended pair when fp16+extended is supported, else
`DisplayP3` where supported), else SDR" — it is *not* wgpu's `SurfaceColorSpace::Auto`,
which resolves fp16 → `ExtendedSrgbLinear`; the doc carries that disclaimer. Whether
`Auto` may select plain `DisplayP3` (turning the chrome shift on for the endgame
default) is deliberately open (§11). Unit variants keep the derive set cheap. Defaults:
`Off` is the upstream rollout default; `Auto` is the endgame default once the fp16
testing infrastructure exists; our fork opts in unconditionally.

**Decider** — one public function, no projections:

```rust
decide_surface_target(caps: &SurfaceCapabilities, pref: SurfaceColorSpacePreference)
    -> Result<SurfaceTarget, WgpuError>
// SurfaceTarget { format, color_space, status }
```

The input must be `&SurfaceCapabilities`: wgpu 30 exposes HDR-capable formats only in
`format_capabilities` with per-format color spaces — a `&[TextureFormat]` cannot see
them. `decide_surface_target` is the single source of the decision; `RenderState` stores
its three outputs and `configure_surface` reads the stored fields. Err domain:
`NoSurfaceFormatsAvailable` only; unavailability of a color space is a *status*, never
an error. Probe input = the first surface's capabilities (`RenderState::create` runs
once); later viewports inherit the stored decision, and a viewport that cannot configure
it fails as an uncaptured device error — documented behavior, not a fallback path.
Headless (no surface): `Sdr { NoSurface }`.

**Invariant tests:** `decide_surface_target` never returns an incoherent target
(`(Rgba16Float, Auto)`, `(u8-format, ExtendedSrgb)`, or the double-grammar pair
`(Bgra8UnormSrgb, DisplayP3)` — wgpu would accept that combo and it violates the
one-grammar rule), over synthetic `format_capabilities` fixtures (Metal-like, DXGI-like,
float-without-extended, empty); and the built `SurfaceConfiguration` is asserted
coherent — `configure_surface` must set `color_space` explicitly, never leave `Auto`
(wgpu's `get_default_config` seeds `Auto`).

**Status** — `HdrStatus`, stored on `RenderState`, create-time-immutable, read by apps
via the existing `frame.wgpu_render_state()`:

```rust
HdrStatus::Hdr { format: TextureFormat, color_space: SurfaceColorSpace }
HdrStatus::Sdr { reason: SdrReason }
// SdrReason: Disabled | NoPairedColorSpace { requested: SurfaceColorSpace } | NoSurface
```

`NoPairedColorSpace` carries the request so bug reports are self-describing and the enum
never grows as pairings land. Headroom is a different quantity: live, per-surface — it
is *not* on `RenderState` (§7).

**Extended-gamma vs scRGB-linear.** Two shader-correct combinations exist: gamma-encoded
values on an `ExtendedSrgb` surface, and linear values on an `ExtendedSrgbLinear`
surface (the existing linearize entry point; #8703 ships the latter). The choice is
*continuity*: (a) formula-identical rendering for SDR chrome — same blend grammar, same
u8 semantics; bit-exact on u8 targets, ≤½-code drift on fp16 targets where the u8 path
quantized at store (the fp16 path is the more accurate side); (b) one texture grammar
for u8 and float content; (c) `ExtendedSrgb` and `ExtendedDisplayP3` cover web —
`ExtendedSrgbLinear` is native-only. The permanent cost, stated once: HDR-over-HDR
compositing and filtering happen in gamma (50%-alpha white over black composites to
linear 0.214, not 0.25), fixable only by a future linear-surface pairing. Egui's own
house style is gamma blending (`egui/src/lib.rs:147`).

**Sizing:** PR 1 is ~300–450 lines including tests, `MANUAL_TESTING.md`, and the example
— no shader changes, no `epaint` changes, no eframe changes. Re-measure against upstream
`main` before drafting.

**Migration:** if #8703 merges first, PR 1 becomes an evolution PR against freshly
merged API (field relocation off `SurfaceConfig`, encoding supersession for their
adopters, `capture.rs` conflicts). That cost is accepted; see §10.

## 3. Output encoding and dithering

`Renderer::new` picks its fragment entry point by `output_color_format.is_srgb()` — a
derivation that exists but is color-space-blind. PR 1 replaces the sniff with an
explicit, derived selection (never stored — storing a derivable value next to its inputs
re-introduces drift):

```rust
enum OutputEncoding { GammaEncoded, Linear }
// OutputEncoding::of(format, color_space) — total function
```

Table: u8 formats → `GammaEncoded`; u8 + `DisplayP3` → `GammaEncoded`; fp16 +
`ExtendedSrgb` → `GammaEncoded`; fp16 + `ExtendedDisplayP3` → `GammaEncoded`; fp16 +
`ExtendedSrgbLinear` → `Linear`. Consequences: PR 1 needs no encoding change at all
(fp16 + `ExtendedSrgb` → gamma passthrough is exactly today's `is_srgb()`-false path);
the color-space input reaches `RendererOptions` only in PR 4, when the scRGB pairing
needs the linear path. The invariant tests cover the whole table.

**Dithering** defaults to on and its doc admits it assumes an sRGB output. On fp16
there is no 8-bit quantizer to hide the noise in, and near black the ±1/255 encoded
noise maps to ×12.92 relative-luminance grain — actively harmful, not "meaningless".
On float targets dithering is forced off regardless of the knob (which stays untouched
for 8-bit targets); the doc updates to "assumes an 8-bit sRGB target".

## 4. Content model: the float color contract (PR 2)

The u8 ceiling is why float content needs a contract: `Color32` packs 4 u8 bytes per
vertex and `ImageData` holds only `ColorImage` — nothing in the paint path can exceed
1.0. PR 2 adds the float path.

**Contract.** A float color/texel in egui's HDR path is **gamma-encoded** in the
buffer's declared `primaries`, **premultiplied in the encoded components**
(`stored = a × enc(L)` — exactly Color32's convention, not `enc(a·L)`), **unclamped
above 1.0**, **odd-symmetric for negatives**; 1.0 = SDR reference white. Linear
(`ecolor::Rgba`) stays the app-side math space, converted at the boundary.

- **Negative extension:** odd-symmetric everywhere. ecolor's Rust functions comply;
  the WGSL gamma functions currently route negatives into the linear branch and
  evaluate `pow(negative, …)` (WGSL-undefined, NaN-prone) in a discarded lane. PR 2
  ships the `sign(x)·f(|x|)` fix before any negative-capable float content, plus a
  CPU↔GPU round-trip test with negative inputs.
- **Premultiply cost:** encoded-space premultiply attenuates HDR translucency by
  `a^2.4` — 50% alpha on L=4 ≈ −2.4 stops vs a photometric composite. That is the price
  of encoded-space blending (the alternative overshoots over non-black destinations);
  photometric alpha work belongs to the future linear-surface pairing. Forcing:
  continuity with Color32's convention + the compositor's `PreMultiplied` contract over
  stored values — the blend itself is convention-agnostic. Contract test: swatch at
  a=0.5, L=4 composites per this rule.
- **Filtering/blending occur in gamma (stored space)** — hardware bilinear on
  non-sRGB float formats has no decode; inherited from the u8 path. A linear-surface
  pairing changes filtering semantics too.
- **Multiply cost:** gamma-space multiplies are formula-identical to today's
  `in.color × tex` path (no regression), but branch-crossing: products whose decoded
  value crosses the encode knee take the wrong branch's scale — in-toe products compose
  to exactly `12.92·ab` (12.9× off), mid-tones ±1.5%, dark factors far worse. PQ ingest
  avoids this by decoding to linear first (§7).
- **Precision:** linear f16 storage is ~2.4× finer in relative luminance terms (≤0.098%
  vs up to ~0.234% for gamma, from the 2.4 decode slope) and its subnormal floor
  (~4.6×10⁻⁹) is 12.9× darker than gamma's (~4.7×10⁻⁶). Both are far below the ~1%
  Weber JND; the decision rests on continuity, not precision.

**Type shape.** `ImageData::Float(Arc<FloatImage>)` with
`FloatImage { primaries: Primaries, pixels: Vec<[f16; 4]> }`, where
`pub enum Primaries { Srgb, DisplayP3 }` (v1 — a new primaries lands as variant +
pairing row + conversion matrix, in one PR). Pixels are gamma-encoded, premultiplied in
the encoded components; RAM format is f16 — 1:1 with the `Rgba16Float` upload
(zero-copy via `bytemuck::cast_slice`), half the memory of f32 (a 4K frame: 66 MB vs
133 MB), zero quantization loss vs display. Helpers (`new`, `filled`, `from_rgba_*`,
lossy `from_color_image`) take f32 at the edges. Consequences: `Eq` drops from
`ImageData`/`ImageDelta`/`TexturesDelta` (f32 — matching `Rgba`'s PartialEq-only
precedent), the closed serde enum extends (changelog note), and the sketch's field
shape must be re-verified against upstream `main` — #8714 privatized `ColorImage` and
renamed `size`→`resolution`, so the constructor-based house style applies.

**Upload pairing.** `update_texture` uploads paired tags (`Rgba16Float` for
`DisplayP3`-tagged content on a DisplayP3 surface, `Srgb`-tagged on `ExtendedSrgb`) and
rejects unpaired tags loudly. The mismatch rule is strict by design: the most-trafficked
mismatch (every implicitly-Srgb-tagged `ColorImage` on a P3 policy) must fail visibly,
not shift colors — conversion is the opting-in app's job in v1 (§5).

**`ui.image()` tint:** the existing `Color32 × texel` multiply just works.

## 5. Wide gamut: the primaries-generic mechanism

**Decision:** wide gamut is in scope, generically. The mechanism: content declares its
primaries; a pairing table decides which surface color space displays each tag
natively; unpaired tags reject loudly. **Design rule (binding): no specific color space
is privileged — nothing in the architecture may name or branch on a specific primaries.
** Display P3 is the first wide-gamut pairing, chosen because Metal and web expose it —
not by ideology. "Color space" = primaries × transfer: egui tags primaries and fixes
storage transfer to gamma; PQ/HLG decode app-side (§7). A new primaries arrives as
enum variant + pairing row + conversion matrix, in one PR.

**Platform exposure (verified in wgpu 30):** the non-extended `DisplayP3` space is
reported for u8 surfaces on Metal and web (sRGB transfer, P3 primaries, SDR clamp,
`is_hdr() == false`) — PR 1's pairing, no fp16 required. The extended pair
(`EXTENDED_SRGB`, `EXTENDED_DISPLAY_P3`) is fp16-gated on Metal and web (PR 3).
DX12 reports none of these — only `EXTENDED_SRGB_LINEAR` (scRGB) and an out-of-contract
HDR10 combination (`Rgb10a2Unorm` + `BT2100_PQ`); Vulkan is driver-dependent via
`VK_EXT_swapchain_colorspace`; GLES reports sRGB only.

**Opt-in chrome shift:** a P3-referred surface reinterprets sRGB-referred UI bytes.
Quantified (CIEDE2000): neutrals shift exactly 0.0 (shared D65 white); saturated
accents 1.4–6.2 (chroma +8–36%); re-encoding the u8 style palette into P3-referred
values is near-lossless (≤0.19). So the opting-in app converts its palette app-side in
v1; egui-side chrome conversion is a follow-up if demand appears. Per-texture content
conversion is the opting-in app's escape hatch for existing sRGB-referred images.

**Demand-gated future work:** full primaries-tagged color API (`Rgba`-level tagging —
the #2712 fix proper), pairwise primaries↔primaries conversion matrices (BT.2020→P3→
sRGB as the canonical chain), per-texture content conversion, color-managed textures
(OCIO-style).

## 6. What tier D cannot do (and why tier E stays deferred)

Tier D's float *textures* give HDR color as a function of position inside a rectangle;
tier E's float *vertex colors* give HDR color as a property of geometry. What stays
impossible at tier D:

1. **Stroked paths** — stroke color lives in vertices: no antialiased rotated geometry,
   no joins/caps, no polylines, no plot traces. (Rotated HDR *rects* work via
   `Image::rotate` — unantialiased.)
2. **Text** — glyph color is vertex-borne, capped at 1.0. No *general* tier-D mechanism
   exists; per-widget float-texture caching is viable for short labels (a galley-keyed
   cache of atlas coverage × HDR color, ~16 KB fp16 for a readout label). The
   `hdr_text` helper is the first thing to prototype for the demo app's nits readout —
   it either validates deferring E or becomes the evidence E is needed.
3. **Gradients above 1.0 across arbitrary shapes** — per-vertex interpolation is the
   only masking mechanism the tessellator offers; textures fake axis-aligned only.

Who needs E: video scopes at frame rate, design tools with HDR gradient meshes,
plot-heavy UIs over HDR canvases, HDR text as a first-class need. Tier E's cost:
`Vertex` color u32 → f16×4 ripples through every mesh consumer (egui-wgpu pipelines,
egui_glow or explicit scoping-out, kittest, capture, mesh serde + the `Eq` cascade
through `Mesh`/`Shape` in both cfg variants) and doubles vertex color memory for every
app — upstream will resist; §11 records the pre-E type decisions.

## 7. Lifecycle: switching, transitions, headroom

**Runtime switching: non-goal.** See §2 — the full rationale lives there. A surface
never needs to change for a *display* change: the fp16+extended-sRGB configuration is
display-agnostic, valid on SDR and HDR panels alike. SDR content renders identically
(everything in [0,1]); values above 1.0 clip at the panel's white; the only quantity
that changes is `tone_map_headroom()` — on an SDR display it returns `Some(1.0)` (Apple
reports 1.0 on non-EDR screens, and the coarse `high_dynamic_range == Some(false)` guard
pins it), verified in `wgpu-types surface.rs:732-757`. It is polled per frame on the
main thread (Metal reads `NSScreen`; off-thread returns `None` with a warn-once), and
EDR headroom ramps over ~1–2 s, so idle repaints run at 250 ms. `Some(1.0)` is a valid
"no headroom" steady state; `None` means *unknown* — the status design keeps them
distinct.

**Capture:** the `capture` feature matches only Rgba8 surfaces and silently produces
nothing on fp16 (logs + returns). The fp16 branch is a PR 3 deliverable (§8), sharing
the f16 decode code with the test readback.

**Calibration facts** (verified in wgpu 30): `DisplayLuminance` (absolute nits incl.
`sdr_white_nits`, moving with the brightness slider) is Windows/DXGI-only;
`DisplayHeadroom` (relative multipliers) is macOS-only — the iOS/tvOS/visionOS branch is
an in-code TODO naming `UIScreen.currentEDRHeadroom` as the path (an upstream-wgpu gap,
not a platform limit). `tone_map_headroom()` precedence: Apple multiplier → `Some(1.0)`
for definitively-SDR displays → `max_nits / sdr_white_nits` → `None`. `DisplayGamut` is
`#[non_exhaustive]` and unordered — pass through, never match on it. On macOS, only
`headroom` + the coarse bit are populated, and the coarse bit means "HDR active right
now" (live multiplier > 1.0), not "panel is HDR-capable". Universal currency for UI =
stops/headroom multiplier; nits are a Windows-only annotation. On macOS the compositor's
per-layer model makes gamma-encoded content native; on Windows the DWM composites in
linear scRGB — the PR 4 pairing exists because the platform dictates the space
(platform-divergent blend grammar is accepted; a linear intermediate render pass is the
cross-platform escape hatch if egui ever wants uniform photometric blending).

**PQ ingest chain** (app-implemented; egui ships none of it): decode `PQ⁻¹(E')` per
BT.2100 in BT.2020-referred linear light (never the sRGB inverse — ≈2.12× brightness
error at reference white); normalize by the 203-nit anchor (ITU-R BT.2408 — the current
convention; the older 100-nit figure renders PQ ~2× too bright; on Windows use
`sdr_white_nits` directly); convert primaries via the 3×3 matrix *in linear light*
(emits negative components — the odd-symmetric contract covers them); apply the paired
primaries' gamma encode. Never re-apply a decode (double-encode). HLG: out of scope
until a transfer-carrying tag exists.

## 8. Platform matrix and testing

**Platforms.** macOS: verified end-to-end (the app); ships as the only runtime-tested
platform. Web: compile-checked; the browser reports `DISPLAY_P3` for u8 canvases and
the extended pair for fp16 canvases, so pairings transfer — but wasm keeps compiling
with the new types only (no runtime claims; kittest excludes the browser backend).
Windows: PR 1/PR 3 pairings are never reported on DX12 → deterministic SDR fallback;
HDR arrives with PR 4's scRGB pairing (KWin demonstrates it on Linux; Windows is
manual-verified only). Linux: KWin works with scRGB today; everything else is
probe-gated SDR. iOS: same EDR layer model as macOS in principle — compile-only, untested,
headroom unpopulated by wgpu (upstream TODO).

**Manual protocol.** `MANUAL_TESTING.md` ships in PR 1: per-platform steps,
deterministic test patterns, expected readings (`HdrStatus`, headroom on an EDR iMac /
Pro Display XDR / an SDR panel), each marked "CI cannot validate this". "Tested" =
"this checklist passes". The upstreamed test app is the protocol's instrument:
snapshot-free (deterministic pattern menu + live status/headroom readout),
compile-checked by CI.

**CI.** egui's GPU tests run on macos-latest only (Linux/Windows are clippy/doc);
kittest prefers CPU adapters (lavapipe) where available, so dev machines and CI exercise
different adapters and fp16 tolerances must absorb the drift. A windowed CI smoke test
(macos runners have a window server) verifies *integration*: real window, surface
configured with the policy, no device errors, sane `HdrStatus`. The *payoff* —
compositor tone mapping, panel gamut — is manual-only.

**Test matrix** (mapped to existing conventions: ecolor 14 inline tests, epaint 96,
egui ~99, kittest snapshots in `egui_demo_lib/tests/`, egui-wgpu's first runtime tests
arrive with this work):

- **Contract properties → `ecolor` inline tests** (following
  `test_color32_blend_vs_gamma_blend`): `gamma_from_linear ∘ linear_from_gamma ≈ id`
  including negatives; premultiply rule; exposure decomposition round-trip.
- **Decider property tests → `egui-wgpu` unit tests** (pure functions, no GPU):
  synthetic `format_capabilities` fixtures — Metal-like (u8 + `DISPLAY_P3`, fp16 +
  extended pair), DXGI-like (LINEAR only), float-without-extended, empty. HDR formats
  live in `format_capabilities`, not `caps.formats`; `formats`-based fixtures never
  exercise the probe. Assert: returned target is supported by the input caps, status
  reason consistent, totality on empty.
- **Encoding numeric assertions → `egui-wgpu` tests with the hand-built fp16
  `RenderState`** (adapter-requiring; tolerances sized for lavapipe/Metal drift): known
  linear → `gamma_from_linear` within f16 quantization (±2⁻¹¹ relative in the normal
  range, subnormal carve-out — a flat relative tolerance flakes at linear 1e-6); 2.0 and
  near-f16-max survive upload→shader→blend; negatives per contract; dither off →
  bit-identical repeats; [0,1] SDR scene matches the u8-path output within tolerance.
- **Visual coverage → kittest snapshots** (following `color_picker.rs`,
  `image_blending.rs`): SDR-region appearance only — u8 goldens are structurally blind
  to >1.0 (2.0 and 10.0 clamp identically), so HDR *values* are guarded solely by the
  numeric assertions.
- **Named low-value (don't add):** single-case decider tests, u8 goldens of HDR values,
  runtime `frame.info()` asserts, raised thresholds.

**PR 3 deliverables (fp16 mechanism — kittest must support fp16 before the HDR
extension lands):**

1. Float readback in **egui-wgpu**, next to `capture.rs` (sharing #8703's
   f16-decode/padding code): `texture_to_float_image` for numeric assertions;
   `egui_kittest` keeps a clamp-to-white u8 path for goldens. Mind 256-byte
   `bytes_per_row` alignment.
2. Headless format override: the no-surface stub consults an explicit format (doubles
   as the power-user escape hatch); the test recipe is the existing
   `Harness::builder().renderer(WgpuTestRenderer::from_render_state(...))` hook —
   no builder changes needed.
3. fp16 branch in `read_screen_rgba` + a loud error/`Result` instead of the silent
   early return.
4. MSAA plumbing in the offscreen harness (`sample_count` + `resolve_target` —
   hardcoded to 1/None today), without which the MSAA-on-fp16 test cannot run.

u8 readback keeps working throughout — PR 1/PR 2 are unaffected.

## 9. Known interactions

- **MSAA on fp16** — resolved: Metal grants resolve on all realistic hardware except
  `apple1x`; the intermediate target uses `target_format`. CI-testable once the harness
  supports MSAA (PR 3 deliverable 4).
- **`present_with_transaction`** — the jitter fix is a latency clamp plus a cargo
  feature; it never touches `color_space`, and reconfigure rebuilds from the stored
  decision. Manual checkbox: jitter-fix + HDR + drag-resize.
- **Transparency** — color space and alpha mode validate independently; Metal reports
  `[Opaque, PostMultiplied]` and egui picks `PostMultiplied`. Manual checkbox: compositor
  behavior of premultiplied >1.0 content.
- **`WgpuSetup::Existing`** — the probe reads the compatible surface's caps regardless
  of instance provenance; a user instance lacking `VK_EXT_swapchain_colorspace` reports
  no extended spaces → correct SDR fallback.
- **Multi-viewport × capture** — one shared `Option<CaptureState>` across viewports with
  per-window sizes; needs a test independent of HDR.
- **glow / wasm** — both stay compile-gated with the new types; no runtime claims.

## 10. Upstream strategy and process

**Engagement is deferred.** No issue, no PR, no public comments: we fix this on the
fork first, upstream only after the app proves the design. The prepared issue draft:
(1) the #7484-class failure + repro; (2) the encoding trap (why naive pairing burns
people); (3) proposed API + non-goals; (4) testing story. Anchors: PR 1 → #2712, PR 3 →
#7484 + the LightCraft demand from #8703, PR 4 → #6863. Pre-empted objections, one line
each: scope (egui-wgpu only, opt-in, glow/web unchanged), maintenance (invariant-tested
decider + per-tier tests), CI-without-HDR-hardware (encoding testable headless;
display interpretation manual; macOS verified by us, Linux by #8703's author).

**#8703 posture.** It occupies the brightness slot (scRGB-linear, blocked since
2026-10-12, zero maintainer review). If it merges first, PR 1 becomes an evolution PR
against freshly merged API — field relocation, encoding supersession for its adopters,
`capture.rs` conflicts; that cost is accepted. If it stalls, PR 1 stands alone. We do
not interact with that work.

**Fork-first workflow.** The app is the verification vehicle: implement on the fork,
`[patch]` the app onto it, upstream only after the app demonstrates everything. Known
cost, accepted: upstream churn forces rebases, and a `[patch]`ed app cannot ship until
the fork tracks a tagged release. Mitigation: keep the delta small and upstream-shaped.

**Re-anchor protocol.** Before writing fork code: re-verify every anchor against the
working tree. Before composing with #8703: against its branch. Before drafting PRs:
against upstream `main` — known drift already: #8714 privatized `ColorImage` and
renamed `size`→`resolution` (§4), and 0.37 shipped 2026-10-15, so the ladder lands
**0.38+**.

## 11. Tier E outlook

`Vertex` color u32 → f16×4 ripples through every mesh consumer: egui-wgpu pipelines,
egui_glow (or explicit scoping-out), kittest, capture, mesh serde + the `Eq` cascade
(`Vertex → Mesh → Shape`, both cfg variants including `unity` — the packing trick must
work in both field orders), and `Context::tessellate` consumers. The layout fight
(stride 20→24 vs dual-format meshes vs sidecar buffers) is the sticky review thread.
Pre-E type decisions: the `Eq` drop, the serde repr of the color field (Color32
serializes as `#rrggbb`; f16×4 needs a chosen repr via `half/serde`), and `half`'s
`bytemuck` feature as epaint's direct surface. Trigger to revisit: animated HDR vector
content (scopes at frame rate), HDR text as a first-class need. The intermediate-linear
render pass (§7) is the alternative for photometric blending without tier E.

## Open questions

1. **Headroom exposure before the per-viewport pass-through lands:** `tone_map_headroom()`
   on `RenderState` (distilled), the whole `DisplayHdrInfo`, or nothing? — the only
   undecided decision in this plan; must be picked before PR 1.
2. Does `Auto` ever select plain `DisplayP3` (turning the chrome shift on for the
   endgame default), or is `Auto` restricted to the extended pair?
3. PR 4: does `decide_surface_target` gain a per-platform DX12 fallback
   (`ExtendedSrgb` → `ExtendedSrgbLinear`)?
4. Web/wasm wiring — deferred for scope (eframe web plumbing + fp16-canvas request).
5. Future work, framed: transfer-carrying tags (PQ/HLG), OkLCh picker geometry,
   tier E vertex format, per-texture content conversion.

## Appendix A — source anchors

Anchors marked ✓ were re-verified against the working tree at HEAD (`hdr_enablement`,
2026-10-26); the rest are 0.36.2-era and get re-swept by the entry-4 protocol before
implementation.

- ✓ `egui-wgpu/src/lib.rs`: no-surface stub `vec![Rgba8Unorm]` (:267);
  `WgpuError` (:46); `SurfaceConfig` runtime-mutable doc (:67);
  `preferred_framebuffer_format` Result-returning (:421); create-once guard
  `render_state.is_none()` (winit.rs:258); no-derivation configure threading.
- ✓ `egui-wgpu/src/renderer.rs`: dithering default true (:230); blend
  `One/OneMinusSrcAlpha` (:415-428); texture creation hardcoded Rgba8Unorm;
  `capture.rs` silent `_ =>` return (:196-212).
- ✓ `egui-wgpu/src/egui.wgsl`: gamma fns route negatives to the linear branch (:52-57);
  "dithering is performed on the gamma encoded values" (:180); entry points unclamped
  above 1.0.
- ✓ `ecolor`: odd-symmetric gamma fns (lib.rs:155-180); Color32 gamma-space
  premultiplied (color32.rs:8); `Rgba` PartialEq-only (rgba.rs:8-12); `Hsva` clamps
  saturation and `rgb_from_hsv` emits `[0, v]` (hsva.rs:186-221).
- ✓ `egui`: gamma-blend house style (lib.rs:147); `ViewportInfo`
  (data/input/viewport_info.rs:28); `Frame::wgpu_render_state()` (eframe epi.rs:943);
  `NativeOptions::dithering` default true (epi.rs:500); iOS CI job (rust.yml:201).
- ✓ `egui_kittest`: u8 readback hardcode (texture_to_image.rs:81); no compatible
  surface headless; silent capture return; `with_render_options` exists
  (builder.rs:210); snapshot diff is u8-only.
- ✓ wgpu-types 30.0.1: `Auto` resolution (core surface_config.rs:24-40);
  `DisplayHeadroom` macOS-only (:644); `tone_map_headroom` 4-step precedence
  (:732-757); `DisplayGamut` non_exhaustive/unordered (:696-703); DX12 mapping
  (hal dx12/mod.rs:1455-1466); GLES SRGB-only (hal gles/adapter.rs:1347);
  Vulkan colorspace extension (hal vulkan/instance.rs:280-282); web P3 (webgpu.rs).
- 0.36.2-era (re-sweep before implementation): §7.4 status payload field list,
  §7.2 pairing-table enforcement points, §7.5 calibration field list.

## Appendix B — design history

This plan went through two independent review rounds on 2025-10-25 (~60 decisions:
six specialist reviewers per round, all findings decided and applied). The complete
decision log with per-decision rationale lives in `reviews/00-consolidated.md`;
raw reviewer documents are in git history (commits `01f0735aa`, `4c553b2f3`).
