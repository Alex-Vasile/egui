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

- **Surface policy** — `SurfacePreference` (app-level, create-time on
  `WgpuConfiguration`) → `decide_surface_target` per viewport, re-run on monitor
  change → per-viewport `SurfaceTarget` + `SurfaceStatus` (+ `hdr_headroom`, PR 3),
  surfaced via `ViewportInfo`; shared `RenderState` keeps device/queue/renderer and
  the format-keyed pipeline cache.
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
| **1** | Wide-gamut surface: `SurfacePreference {Off, WideGamut}`, per-viewport `decide_surface_target` (re-run on monitor change), format-keyed pipeline cache, per-viewport `SurfaceStatus`; callback demo with out-of-sRGB saturation | any eframe app opting in | decider property tests, pipeline-cache test, u8 snapshots, windowed smoke |
| **2a** | WGSL negative fix (`sign(x)·f(|x|)`) + ecolor contract tests | correctness floor for all float content | negative CPU↔GPU round-trip, ecolor properties |
| **2b** | Tagged float content (tier C): `Primaries`, `FloatImage`, `ImageData::Float`, upload pairing, `ui.image()` float display | HDR/wide-gamut images via `ui.image()` | upload roundtrip, ecolor properties, snapshots |
| **3** | HDR extended range: extended pair on `Rgba16Float` (>1.0, EDR); `Hdr`, `WideGamutHdr`; headroom pass-through (`ViewportInfo.hdr_headroom`, painter-fed); dither off on fp16 | brightness above SDR white, adaptive to live headroom | fp16 readback numeric tests, negative GPU round-trip, capture branch, MSAA, windowed smoke asserts the headroom field |
| **4** | scRGB-linear pairing (Windows/DX12) — HDR brightness **and** wide gamut via signed values — activates the derived encoding path | Windows HDR + wide gamut | per-platform (manual-heavy) |
| **5** | Tier D: `color_edit_button_hdr` + swatch widgets; the picker edits linear `Rgba` (the app-side math space), converting to encoded at paint | HDR editing UI | picker snapshots, decomposition tests |
| — | Side items: docs, test app | adoption | compile-only + manual |
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
pub enum SurfacePreference {
    Off,          // today's behavior — the rollout default
    WideGamut,    // PR 1 — wider gamut, SDR range
    Hdr,          // PR 3 — brightness, sRGB gamut pinned
    WideGamutHdr, // PR 3 — both axes, all-or-nothing
}
```

Why the preference sits on `WgpuConfiguration` (create-time app policy) rather than
the runtime-mutable `SurfaceConfig`: naive per-frame pairing lands in #7484 territory
(double gamma, black screens). What the per-viewport machinery deliberately never
does is recreate the renderer or context: a renderer-only recreation loses
`Renderer.textures` (font atlas, uploads, `register_native_texture` bind groups)
while egui ships per-frame *deltas*, so meshes reference missing textures until every
image is re-touched; re-registering user native textures has no recipe; full
recreation is impossible as an eframe API (apps hold `egui::Context` by value). Full
recreate remains an app-level technique (the workaround app does exactly this), not
an upstream deliverable. App-driven preference changes at runtime stay out of v1
scope — they reduce to the same re-decide path as a monitor change (§7) if demand
appears.

**Capability requests, not color-space names.** Each variant names a capability the
app opts into; the decider maps it to a concrete surface pairing per platform. Two
independent bools (`want_hdr`, `want_wide_gamut`) were rejected: the same four
combinations exist either way, and only the enum binds each combination to one
documented fallback policy instead of leaving the join semantics implicit. Every
variant is all-or-nothing on its own definition — ungrantable means plain SDR with a
`FellBack` status, never a partial grant. An app that prioritizes brightness picks
`Hdr`; saturation, `WideGamut`; needs both together, `WideGamutHdr`. Naming primaries
in requests was rejected for the same reason: the choice can't bind (no platform
reports more than one in-contract wide space today), it would go stale the day a
second appears, and primaries precision already lives where it's actionable — the
content tags (§5).

**The probe is primaries-blind.** Each variant's decider leg walks a priority-ordered
pairing table and takes the first space the platform reports on a supported format:

```rust
// WideGamut leg (PR 1): non-extended spaces on u8 formats
for cs in WIDE_GAMUT_ORDER {              // widest first — table data; +1 row per new primaries
    for cap in &caps.format_capabilities {
        if is_u8(cap.format) && cap.color_spaces.contains(cs) {
            return Granted(cap.format, cs);
        }
    }
}
FellBack { NoPairedColorSpace }           // nothing wide reported
```

`Hdr` walks the extended-gamma table the same way; `WideGamutHdr` requires the combined
pairing. Concrete primaries exist only in these table rows and the content-tag
pairings — never in the request API (§5's rule). Today the walk has one entry on
Metal and web (`DISPLAY_P3`) and none on DX12/GLES; a platform reporting BT.2020
extends the table with zero API change. Unit variants keep the derive set cheap.
Defaults: `Off` is the default and stays the default — HDR and wide gamut are opt-in,
one line each. A default flip (`Off` → `Hdr`) is upstream's potential call
post-adoption, explicitly not planned here: an fp16 swapchain doubles scanout
bandwidth for every window, and imposing capability — however visually inert — on
apps that never asked repeats the silent-decision class this design avoids. Our fork
opts in unconditionally (an app-level choice).

**Decider** — one public function, no projections:

```rust
decide_surface_target(caps: &SurfaceCapabilities, pref: SurfacePreference)
    -> Result<(SurfaceTarget, SurfaceStatus), WgpuError>
// SurfaceTarget { format, color_space }  ·  SurfaceStatus: granted or fell back
```

**Per-viewport decisioning.** The preference is app-level policy (create-time on
`WgpuConfiguration`); the *decision* is per-viewport. The painter runs
`decide_surface_target` against each viewport's own surface capabilities — at viewport
creation, and again whenever the viewport's monitor changes — stores the resulting
`(SurfaceTarget, SurfaceStatus)` in that viewport's state, and surfaces both through
per-viewport `ViewportInfo` alongside `hdr_headroom` (§7). A monitor change that
alters the capability re-configures that viewport's surface and swaps the pipeline;
no renderer or context is ever recreated, so #7484's texture-loss trap is not on this
path — by construction. The input must be `&SurfaceCapabilities`: wgpu 30 exposes
HDR-capable formats only in `format_capabilities` with per-format color spaces — a
`&[TextureFormat]` cannot see them. The `Off` leg delegates to the existing
`preferred_framebuffer_format` (which stays public as the Off-leg implementation), so
the default path is byte-identical to today's by construction — including its
`formats.first()` fallback, which can return `Rgba16Float` on float-only surfaces;
that behavior is inherited deliberately, never silently changed. Err domain:
`NoSurfaceFormatsAvailable` only; unavailability of a capability is a *status*, never
an error — and the worst case is an honest SDR viewport: the uncaptured-error/panic
path of the one-target model is gone by construction. Headless (no surface):
`FellBack { NoSurface }`.

**Invariant tests (all GPU-free; run in CI on every platform).** `decide_surface_target`
never returns an incoherent target
(`(fp16 format, Auto)`, `(u8 format, extended space)`, or the double-grammar pair
`(Bgra8UnormSrgb, DisplayP3)`). *Grammar* = whether the format applies a hardware sRGB
decode (`*Srgb` view formats) or stores raw encoded values (`*Unorm` views); a surface
must use exactly one — the flagship PR 1 pairing `Bgra8Unorm + DisplayP3` is
raw-view + encoded and valid, while `Bgra8UnormSrgb + DisplayP3` would decode and then
re-interpret, double-encoding. The enforcement point is a pure builder —
`build_configuration(caps, target) -> SurfaceConfiguration`, no device, no surface —
called by the per-viewport configure path, with a `debug_assert!` at the construction
site: never `Auto` (wgpu's `get_default_config` seeds `Auto`, so the explicit set is
load-bearing), coherent pair, matches the granted target. Tests run the builder
directly over synthetic `format_capabilities` fixtures (Metal-like, DXGI-like,
float-without-extended, empty): the `Off` leg returns exactly
`preferred_framebuffer_format`'s choice on every fixture (equality test), every built
configuration is coherent, and the pipeline-cache lookup resolves each granted format
to the correct pipeline (one entry per format through PR 3).

**Status + target** — per-viewport, decided at viewport creation and on monitor
change, surfaced to apps through `ViewportInfo` (next to `hdr_headroom`, §7); the
shared `RenderState` keeps device/queue/renderer and the format-keyed pipeline cache:

```rust
pub struct SurfaceTarget {
    pub format: TextureFormat,
    pub color_space: SurfaceColorSpace,
}
impl SurfaceTarget {
    /// Can this surface display values above 1.0? (Derived — never stored.)
    pub fn is_extended(&self) -> bool;
}

pub enum SurfaceStatus {
    Granted,                                   // the preference was honored
    FellBack { reason: FallbackReason },
}

pub enum FallbackReason {
    NoPairedColorSpace { requested: SurfacePreference },  // self-describing bug reports
    NoSurface,                                            // headless
}
```

Two questions, two types: `surface_target` answers *what is configured*; `status`
answers *was the request honored*. Every state is truthful — a granted u8 + `DisplayP3`
pairing is `Granted` with `is_extended() == false` (wide gamut, SDR range), not a
mislabel. `NoPairedColorSpace` carries the request so bug reports are self-describing.
`Disabled` dropped: it existed only to shove `Off` into the old enum — Off is simply
`Granted`. Headroom is a different quantity: live, per-surface — it is *not* on
`RenderState` (§7).

**Extended-gamma vs scRGB-linear.** Two shader-correct combinations exist: gamma-encoded
values on an `ExtendedSrgb` surface, and linear values on an `ExtendedSrgbLinear`
surface (the existing linearize entry point; #8703 ships the latter). The choice is
*continuity*: (a) formula-identical rendering for SDR chrome — same blend grammar, same
u8 semantics; bit-exact on u8 targets, ≤½-code drift on fp16 targets where the u8 path
quantized at store (the fp16 path is the more accurate side); (b) one texture grammar
for u8 and float content; (c) `ExtendedSrgb` and `ExtendedDisplayP3` cover web —
`ExtendedSrgbLinear` is native-only. The permanent cost, stated once: HDR-over-HDR
compositing and filtering happen in gamma — 50%-alpha white over black composites to
linear 0.214 instead of the photometric 0.5 — and the attenuation is baked into the
stored values (§4), so a linear-surface pairing can display them linearly but cannot
repair the compositing; that requires a content-contract change (photometric
premultiply). Egui's own
house style is gamma blending (`egui/src/lib.rs:147`).

**Pipelines.** A color-space change is a swapchain reconfigure — no pipeline impact;
only the format bakes a render pipeline. Through PR 3 the ladder uses exactly two
formats (`Bgra8Unorm`, `Rgba16Float`) and both take the same gamma-passthrough
fragment path (the `OutputEncoding` split arrives with PR 4's scRGB leg), so the
renderer's pipeline cache holds two entries per preference, keyed by format — the
renderer's only change.

**Sizing:** PR 1 is ~500–700 lines including tests, `MANUAL_TESTING.md`, and the
example — no shader changes, no `epaint` changes; eframe changes are confined to the
painter's per-viewport state; the renderer gains the format-keyed pipeline cache.
Re-measure against upstream `main` before drafting.

**Migration:** if #8703 merges first, PR 1 becomes an evolution PR against freshly
merged API (field relocation off `SurfaceConfig`, encoding supersession for their
adopters, `capture.rs` conflicts). That cost is accepted; see §10.

## 3. Output encoding and dithering

`Renderer::new` picks its fragment entry point by `output_color_format.is_srgb()` — a
derivation that exists but is color-space-blind. **PR 4** replaces the sniff with an
explicit, derived selection (never stored — storing a derivable value next to its inputs
re-introduces drift); PR 1 ships none of this — with no color-space input to
`RendererOptions`, its surfaces keep today's `is_srgb()` path, which the table below
shows is already correct for every PR 1/PR 3 pairing:

```rust
enum OutputEncoding { GammaEncoded, Linear }
// OutputEncoding::of(format, color_space) — total function
```

Table: u8 formats → `GammaEncoded`; u8 + `DisplayP3` → `GammaEncoded`; fp16 with any
non-extended space (`Srgb` included — legal in wgpu, so the table must cover it) →
`GammaEncoded`; fp16 + `ExtendedSrgb` → `GammaEncoded`; fp16 + `ExtendedDisplayP3` →
`GammaEncoded`; fp16 + `ExtendedSrgbLinear` → `Linear`. Consequences: PR 1 needs no encoding change at all
(fp16 + `ExtendedSrgb` → gamma passthrough is exactly today's `is_srgb()`-false path);
the color-space input reaches `RendererOptions` only in PR 4, when the scRGB pairing
needs the linear path. The table's totality (every legal pairing maps to an encoding)
is unit-tested when `of` lands in PR 4 — PR 1/PR 3 exercise only the passthrough row
implicitly.

**Dithering** defaults to on and its doc admits it assumes an sRGB output. On fp16
there is no 8-bit quantizer to hide the noise in, and near black the ±1/255 encoded
noise maps to ×12.92 relative-luminance grain — actively harmful, not "meaningless".
On float targets dithering is forced off regardless of the knob (which stays untouched
for 8-bit targets); the doc updates to "assumes an 8-bit sRGB target". Plumbing note:
the dither seed derives without format input at the renderer site today — PR 3 threads
the target format through so the float force-off is structural, not a knob value.

## 4. Content model: the float color contract (PR 2)

The u8 ceiling is why float content needs a contract: `Color32` packs 4 u8 bytes per
vertex and `ImageData` holds only `ColorImage` — nothing in the paint path can exceed
1.0. PR 2 adds the float path.

**Contract.** A float color/texel in egui's HDR path is **gamma-encoded** in the
buffer's declared `primaries`, **premultiplied in the encoded components**
(`stored = a × enc(L)` — exactly Color32's convention, not `enc(a·L)`), **unclamped
above 1.0**, **odd-symmetric for negatives** — the same extension convention as IEC
61966-2-1's extended range and CSS Color 4; 1.0 = SDR reference white (§7 keeps this
true on Windows scRGB by rescaling to the display's SDR white). Linear
(`ecolor::Rgba`) stays the app-side math space, converted at the boundary.

- **Negative extension:** odd-symmetric everywhere. ecolor's Rust functions comply;
  the WGSL gamma functions currently route negatives into the linear branch and
  evaluate `pow(negative, …)` (WGSL-undefined, NaN-prone) in a discarded lane. PR 2
  ships the `sign(x)·f(|x|)` fix before any negative-capable float content, plus a
  CPU↔GPU round-trip test with negative inputs.
- **Premultiply cost:** encoded-space premultiply attenuates HDR translucency —
  50% alpha on L=4 displays linear 0.81 instead of the photometric 2.0 (−1.3 stops;
  the a^2.4 ≈ −2.4-stops figure is attenuation vs the *unattenuated* value, a
  different basis). That is the price of encoded-space blending (the alternative
  overshoots over non-black destinations). It is also baked into *storage*: the
  attenuation lives in the stored values, so a future linear-surface pairing can
  display them linearly but cannot recover the unattenuated product — repairing it is
  a content-contract change (photometric premultiply), not a surface change. Forcing:
  continuity with Color32's convention; the compositor's `PreMultiplied` behavior for
  >1.0 content is a named manual experiment (§9), not load-bearing here — the blend
  itself is convention-agnostic. Contract test: swatch at a=0.5, L=4 composites per
  this rule.
- **Filtering/blending occur in gamma (stored space)** — hardware bilinear on
  non-sRGB float formats has no decode; inherited from the u8 path. A linear-surface
  pairing changes filtering semantics too. And wgpu exposes no mipmap generation, so
  heavily minified textures shimmer at fit-to-window sizes — a documented limitation,
  not a v1 work item.
- **Multiply cost:** gamma-space multiplies are formula-identical to today's
  `in.color × tex` path (no regression), but branch-crossing: products whose decoded
  value crosses the encode knee take the wrong branch's scale — in-toe products compose
  to exactly `12.92·ab` (12.9× off), mid-tones ±1.5%, dark factors far worse. PQ ingest
  avoids this by decoding to linear first (§7).
- **Precision:** three regions. In the power region (above the toe break), gamma
  storage is ~2.4× coarser in relative luminance (≈0.12% vs ≈0.05% step — both far
  below the ~1% Weber JND). In the deep toe the roles reverse: gamma's subnormal
  onset sits 12.9× deeper in linear terms (≈4.7×10⁻⁶ vs 6.1×10⁻⁵), where linear's
  steps degrade past ~1.2% while gamma holds ≈0.1%. Absolute floors: gamma encodes
  down to linear ≈4.6×10⁻⁹ (via the toe's 12.92 slope), 12.9× below linear's
  ≈6.0×10⁻⁸. Neither difference is perceptible on content; the decision rests on
  continuity — with the honest note that each curve wins in its own region.

**Type shape.** `ImageData::Float(Arc<FloatImage>)` with
`FloatImage { primaries: Primaries, pixels: Vec<[f16; 4]> }`, where
`pub enum Primaries { Srgb, DisplayP3 }` (v1 — a new primaries lands as variant +
pairing row + conversion matrix, in one PR). Pixels are gamma-encoded, premultiplied in
the encoded components; RAM format is f16 — 1:1 with the `Rgba16Float` upload
(zero-copy via `bytemuck::cast_slice`), half the memory of f32 (a 4K frame: 66 MB vs
133 MB), zero quantization loss vs display. Upload mechanics: egui-wgpu's
`bytes_per_row` is hardcoded `4 * width` today and becomes f16-aware (`8 * width`,
padded to wgpu's 256-byte alignment); and wgpu's `write_texture` stages through a
buffer whenever row pitch misses that alignment — "zero-copy" is the CPU-side map, the
GPU copy is a real blit (fast, but budget for it in the exposure/re-upload path).
Helpers (`new`, `filled`, `from_rgba_*`,
lossy `from_color_image`) take f32 at the edges. Consequences: `Eq` drops from
`ImageData`/`ImageDelta`/`TexturesDelta` (f32 — matching `Rgba`'s PartialEq-only
precedent), the closed serde enum extends (changelog note), and the sketch's field
shape must be re-verified against upstream `main` — #8714 privatized `ColorImage` and
renamed `size`→`resolution`, so the constructor-based house style applies.

**Upload pairing.** Content carries a `Primaries` tag — `Srgb` is the implicit default
(every existing `ColorImage`) — and a pairing table decides what each upload does on
each granted surface:

| Content tag | Granted surface space | Behavior |
|---|---|---|
| `Srgb` (implicit or explicit) | sRGB space (any format) | native |
| `Srgb` (implicit) | `DisplayP3` | native upload — shifts like the chrome (§5) |
| `DisplayP3` (explicit) | `DisplayP3` (± extended) | native |
| `DisplayP3` (explicit) | sRGB / extended-sRGB / scRGB | **loud reject** |
| BT.2020 (future) | anything non-BT.2020 | loud reject until conversion matrices land |

The asymmetry is deliberate and follows the chrome: untagged sRGB bytes on a P3 surface
shift — the same interpretation every vertex color on that surface gets — so an
opting-in app's UI and images move together, and the §5 re-encode (≤0.21 ΔE) is the
escape hatch for either. Loud rejection is reserved for an explicit claim the surface
cannot honor: P3-tagged content on a surface without P3 is gamut-clipped, and no
silent guess is honest there. Mechanically, rejection reuses egui's existing
missing-texture convention: the upload is not performed, a warning logs once per
texture id, and the standard placeholder renders — no panic, no `update_texture`
signature change (a `Result`-returning upload is future work if demand appears).

**`ui.image()` tint:** the existing `Color32 × texel` multiply just works.

## 5. Wide gamut: the primaries-generic mechanism

**Decision:** wide gamut is in scope, generically. The mechanism: content declares its
primaries; a pairing table decides what each (tag, surface) combination does — explicit
tags the surface cannot honor reject loudly (§4), while untagged content follows the
surface's interpretation, the same shift the chrome gets. **Design rule (binding): no
specific color space
is privileged — nothing in the architecture may name or branch on a specific primaries.
** The rule is structural, not aspirational: the request API names capabilities, not
primaries (§2); the decider's probe is a table walk over reported capabilities; and
concrete primaries live only in pairing-table rows. Display P3 is the first wide-gamut
pairing, chosen because Metal and web expose it —
not by ideology. "Color space" = primaries × transfer: egui tags primaries and fixes
storage transfer to gamma; PQ/HLG decode app-side (§7). A new primaries arrives as
enum variant + pairing row + conversion matrix, in one PR.

**Platform exposure (verified in wgpu 30):** the non-extended `DisplayP3` space is
reported for u8 surfaces on Metal and web (sRGB transfer, P3 primaries, SDR clamp,
`is_hdr() == false`) — PR 1's pairing, no fp16 required. The extended pair
(`EXTENDED_SRGB`, `EXTENDED_DISPLAY_P3`) is fp16-gated on Metal and web (PR 3), with a
carried caveat: wgpu-hal's Metal backend has an in-code TODO that extended-P3 caps
"need protection … like the BT.2100 cases" (`wgpu-hal metal/surface.rs:299`), so the
pairing is probe-gated, never assumed — and a failed configure follows the
multi-viewport failure policy (§7), never a silent sRGB reinterpretation.
DX12 reports none of these — only `EXTENDED_SRGB_LINEAR` (scRGB) and an out-of-contract
HDR10 combination (`Rgb10a2Unorm` + `BT2100_PQ`); Vulkan is driver-dependent via
`VK_EXT_swapchain_colorspace`; GLES reports sRGB only.

**Opt-in chrome shift:** a P3-referred surface reinterprets sRGB-referred UI bytes.
Quantified over egui 0.36.2's default style palette (hyperlink, warn, error,
selection-stroke, accent-fill in both themes; CIEDE2000, D65, no chromatic
adaptation): neutrals shift exactly 0.0 (shared D65 white); saturated accents
2.2–6.8 (chroma +8–36%); re-encoding the u8 palette into P3-referred values is
near-lossless (≤0.21, u8 quantization). So the opting-in app converts its palette
app-side in
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

**Runtime switching.** Three distinct mechanisms, decided separately:

- **Monitor-driven re-decide: shipped (PR 1).** A viewport moved to a different
  monitor re-runs the decider against that monitor's capabilities and reconfigures —
  capability follows the display; per-viewport `SurfaceStatus` and `hdr_headroom`
  update with it. No renderer or context recreation is involved (§2), so #7484's
  partial-recreate trap is not on this path.
- **App-driven preference changes: non-goal in v1.** The preference is create-time
  app policy. The per-viewport machinery makes runtime preference mutation the same
  code path as a monitor change; demand-gated future work.
- **Renderer/context recreation: non-goal (unchanged).** Renderer-only recreation
  loses `Renderer.textures` (#7484, §2); full recreation is impossible as an eframe
  API (apps hold `egui::Context` by value). Full recreate remains an app-level
  technique (the workaround app does exactly this).

A surface itself never needs to change for a *display* change — the fp16+extended
configuration is display-agnostic, valid on SDR and HDR panels alike. SDR content
renders identically (everything in [0,1]); values above 1.0 clip at the panel's white;
the only quantity that changes with the display is `tone_map_headroom()` — on an SDR
display it returns `Some(1.0)` (Apple reports 1.0 on non-EDR screens, and the coarse
`high_dynamic_range == Some(false)` guard pins it), verified in `wgpu-types
surface.rs:732-757`. It is polled per frame on the main thread (Metal reads
`NSScreen`; off-thread returns `None` with a warn-once), and EDR headroom ramps over
~1–2 s (the repaint cadence is the adaptive recipe below). `Some(1.0)` is a valid "no
headroom" steady
state; `None` means *unknown* — the status design keeps them distinct.

**Exposure (decided; ships in PR 3).** Headroom is a per-viewport quantity on a
per-viewport surface, so it cannot live on the shared `RenderState` — structurally, not
by preference. The eframe wgpu painter, which owns each viewport's surface, polls
`tone_map_headroom()` per frame and writes it into per-viewport
`ViewportInfo.hdr_headroom` (Option<f32>, multiplier — the universal currency). It
ships in PR 3, alongside the first capability that can consume it: extended-range
rendering without headroom is blind (the same 2.0 is clipped white on an SDR display
and comfortable at headroom 8.0). Until PR 3, apps holding their own surface poll
out-of-band — the workaround-app pattern. Repaint reactivity is painter-owned and
adaptive: while headroom is moving (`|Δ| > ε` between frames) or unsettled, the painter
schedules `request_repaint_after(100 ms)` — ~10 fps for the 1–2 s a ramp lasts; once
steady (stable value or `Some(1.0)` on SDR), it stops scheduling and egui rests in
`ControlFlow::Wait` at zero cost. Apps wanting faster can repaint themselves.

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

**Windows scRGB anchor (PR 4).** scRGB is scene-referred: `1.0` = 80 nits, fixed — the
sRGB standard's reference white (IEC 61966-2-1) — unlike macOS EDR's display-referred
`1.0` (the display's current SDR white, moving with brightness). Windows' SDR-content
slider does not touch Advanced-Color output at all. egui therefore auto-scales
scRGB-surface output by `sdr_white_nits / 80` (per-frame uniform on the linear path;
`DisplayLuminance.sdr_white_nits` tracks the user's slider), so the §4 contract —
1.0 = SDR reference white — holds identically on both platforms and chrome matches
surrounding SDR apps. Apps that want absolute nits divide by `sdr_white_nits`
themselves. The three anchors in play, one table: **80 nits** (scRGB/sRGB standard,
Windows) · **203 nits** (BT.2408, PQ ingest below) · **display-relative** (macOS EDR;
the Windows slider).

**Windows wide gamut (PR 4).** scRGB's sRGB primaries are basis vectors, not a
boundary: signed values make the space gamut-unbounded. DWM's canonical composition
space carries values outside [0,1] (Microsoft, Advanced Color docs); clipping happens
only at the display kernel, against the physical panel's gamut. The same
`(Rgba16Float, ExtendedSrgbLinear)` pairing therefore delivers Windows HDR brightness
**and** wide gamut: P3/BT.2020-referred content converts via the 3×3 matrix in linear
light — emitting negatives, which the §4 odd-symmetric contract covers — and on
Windows 11 22H2+ the same path reaches provisioned SDR wide-gamut displays. Two
consequences: the `WideGamut` variant (u8 + non-extended space) still falls back on
DX12 — Windows wide gamut arrives only through this fp16-linear pairing; and the
`Rgb10a2Unorm + BT2100_PQ` HDR10 pairing demotes to optional future work (its value
would be HDR-metadata signaling, which wgpu 30 never writes, at the cost of a third
content contract: PQ transfer + 10-bit unorm).

**PQ/HLG ingest — future work; nothing in v1 consumes BT.2020.** BT.2100 PQ content
decodes to BT.2020-referred linear light, and v1's content model has no BT.2020
pairing — so the ingest chain defers as a whole to the transfer-carrying-tags work
(§ Open questions). The chain's shape, recorded so the future implementer doesn't
re-derive it: decode `PQ⁻¹(E')` per BT.2100 in BT.2020-referred linear light — never
the sRGB inverse (PQ code 0.58 — reference white — decodes to linear 0.30 instead of
1.0 under the 203-nit anchor, ≈3.4× too dark; keeping the 10k-nit PQ output scale
instead makes it ≈14.6× too bright — the EOTFs are not interchangeable); normalize by
the 203-nit anchor (ITU-R BT.2408 — the current convention; the older 100-nit figure
renders PQ ~2× too bright; on Windows use `sdr_white_nits` directly); convert
primaries via the 3×3 matrix *in linear light* (emits negative components — the §4
odd-symmetric contract covers them); apply the paired primaries' gamma encode. Never
re-apply a decode (double-encode).

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
deterministic test patterns, expected readings (`SurfaceStatus`, headroom on an EDR iMac /
Pro Display XDR / an SDR panel), each marked "CI cannot validate this". "Tested" =
"this checklist passes". The upstreamed test app is the protocol's instrument:
snapshot-free (deterministic pattern menu + live status/headroom readout),
compile-checked by CI.

**CI.** egui's GPU tests run on macos-latest only (Linux/Windows are clippy/doc);
kittest prefers CPU adapters (lavapipe) where available, so dev machines and CI exercise
different adapters and fp16 tolerances must absorb the drift. A windowed CI smoke test
(macos runners have a window server) verifies *integration*: real window, surface
configured with the policy, no device errors, sane `SurfaceStatus`. The smoke is a
plain `eframe` example binary (`examples/hdr_smoke.rs`) — example bins run on the main
thread by construction (kittest's eframe-spawning helper is deprecated/asserted-against
and is not involved), and example dev-dependencies keep the crates' dep trees clean.
It runs `run_native` with `WideGamut`, asserts on first frame that `SurfaceStatus` is
coherent — `Granted` or `FellBack`, never a panic — draws one label, exits after a few
frames; CI fails on nonzero exit. PR 3 extends the same example with the headroom-field
assertion. The *payoff* —
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

1. Web/wasm wiring — deferred for scope (eframe web plumbing + fp16-canvas request).
2. Future work, framed: transfer-carrying tags (PQ/HLG), OkLCh picker geometry,
   tier E vertex format, per-texture content conversion, runtime preference changes
   (the same re-decide path as a monitor change).

## Appendix A — source anchors

Anchors marked ✓ were re-verified against the working tree at HEAD (`hdr_enablement`,
2026-10-26); the rest are 0.36.2-era and get re-swept by the re-anchor protocol (§10)
before implementation.

- ✓ `egui-wgpu/src/lib.rs`: no-surface stub `vec![Rgba8Unorm]` (:267);
  `WgpuError` (:46); `SurfaceConfig` runtime-mutable doc (:67);
  `preferred_framebuffer_format` Result-returning (:421); create-once guard
  `render_state.is_none()` (winit.rs:258); no-derivation configure threading.
- ✓ `egui-wgpu/src/renderer.rs`: dithering default true (:230); blend
  `One/OneMinusSrcAlpha` (:443-455); texture creation hardcoded Rgba8Unorm;
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
- 0.36.2-era (re-sweep before implementation): status payload field list (§2),
  pairing-table enforcement points (§4/§5), calibration field list (§7).

## Appendix B — design history

This plan went through two independent review rounds on 2026-10-25 and a third
user-walked round on 2026-10-26 (~90 decisions: six specialist reviewers per round,
every finding decided and applied). The complete decision logs with per-decision
rationale live in `reviews/00-consolidated.md` and `reviews2/00-consolidated.md`;
raw reviewer documents are in git history (commits `01f0735aa`, `1f3949f99`,
`14cb87899`).
