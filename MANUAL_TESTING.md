# Manual testing — wide-gamut + HDR surface (PR 1 + PR 3)

**"Tested" means "this checklist passes."** Everything below exercises the
parts of the surface work that a machine cannot judge: what a compositor does
with the surface, and what a panel does with the colors. Each step is marked
**CI cannot validate this.**

## Instruments

| Command | What it shows |
|---|---|
| `cargo run -p eframe --example hdr_smoke` | Windowed smoke: opens a window with `SurfacePreference::WideGamutHdr`, asserts the surfaced decision is coherent **and that display headroom is reported from the first frame** (`None` fails), logs the pairing + headroom, exits 0 after a few frames. |
| `cargo run -p eframe --example hdr_wide_gamut` | Live readout: `ViewportInfo::surface_status` / `surface_target` and a full-width 2×3 primaries grid canvas (bold "sRGB primaries" / "display primaries" labels sandwich it) — top row the sRGB primaries converted at runtime into the granted color space's encoding, bottom row the display's native primaries as full-range bytes ((255, 0, 0), (0, 255, 0), (0, 0, 255)). |
| `cargo run -p eframe --example hdr_float_image` | Float content end-to-end: `FloatImage` swatches (values above 1.0) through the real upload→pipeline→surface path, plus the HDR editing UI (`color_edit_button_hdr` + `swatch_hdr` on a linear color with exposure). Requests `WideGamutHdr`: with `SurfacePreference::Hdr`/`WideGamutHdr` the extended surface shows them as brightness, on SDR surfaces they clip at white. |

The log line `Surface decision: Rgba16Float + ExtendedDisplayP3 (Granted),
headroom 1.2×` (or the example's on-screen readout) is the primary reading
throughout.

Expected readings on this PR, by instrument and panel (the preference differs
per instrument: the smoke asks for `WideGamutHdr`, the other two examples for
`WideGamut`):

| Instrument / panel | Expected `surface_status` | Expected `surface_target` | Expected `hdr_headroom` |
|---|---|---|---|
| `hdr_smoke` on an EDR panel (EDR iMac, Pro Display XDR, recent MacBook panels) | `Granted` | `Rgba16Float + ExtendedDisplayP3` | the live EDR multiplier (`> 1.0`; ramps over ~1–2 s after a brightness change, then rests) |
| `hdr_smoke` on an SDR panel reporting SRGB | `FellBack(NoPairedColorSpace)` | `Bgra8Unorm + Srgb` | `Some(1.0)` (the definitively-SDR guard pins it) |
| `hdr_wide_gamut` / `hdr_float_image` (WideGamut) on any P3-reporting panel | `Granted` | `Bgra8Unorm + DisplayP3` | same headroom column as above (it is a display property, not a preference property) |
| Headless / no surface (offscreen harness) | `None` (fields unset) | `None` | `None` (unknown — never fabricated) |

Caveat: macOS reports what the *screen mode* offers, not what the panel can
show — some SDR displays still get `DisplayP3`. That grant is correct; the
*out-of-sRGB saturation* then needs an actually-P3-capable panel to be
visible.

## How to interpret PNG dumps

Every PNG this branch's tooling writes now carries a **cICP chunk** (PNG
third edition; code points per ITU-T H.273) stating how its bytes must be
interpreted. Read it with `pngcheck -v <file>` or by decoding the four bytes
after the `cICP` header (primaries, transfer, matrix, full-range):

| cICP (p, t, m, r) | Meaning | Written by |
|---|---|---|
| absent | sRGB-referred by convention (see below) | all goldens predating this branch |
| `(1, 13, 0, 1)` | sRGB: BT.709 primaries, sRGB transfer | kittest snapshots (default), 8-bit sRGB-surface captures |
| `(12, 13, 0, 1)` | Display P3 primaries, sRGB transfer | kittest with `SnapshotOptions::color_space(DisplayP3)`, captures from a `DisplayP3` surface |
| `(1, 8, 0, 1)` | linear-light sRGB | kittest with `SnapshotOptions::color_space(LinearSrgb)`; `ExtendedSrgbLinear` surface captures |
| `(9, 16, 0, 1)` | BT.2020 primaries, PQ (ST 2084) transfer | future (no BT.2020 surface pairing is granted yet) |
| `(9, 18, 0, 1)` | BT.2020 primaries, HLG (ARIB STD-B67) transfer | future (no BT.2100 HLG surface pairing is granted yet) |

Rules of thumb:

- **Untagged legacy goldens are sRGB-referred.** A decoder without cICP
  support assumes sRGB, and everything written before this branch *is* sRGB,
  so old files decode correctly either way.
- **A tagged file is not retroactively convertible.** The tag says how the
  bytes were encoded; it doesn't widen them. A P3-tagged dump viewed in an
  sRGB-only viewer looks oversaturated — that is the mustard-vs-red trap this
  branch hit: the bytes were P3, the tag was missing, and everyone read them
  as sRGB.
- **Capture tags reflect the surface, not the window.** egui-wgpu never
  writes PNGs itself; pixels leave through `ScreenshotCallback`. Save them
  with `egui_wgpu::capture::write_capture_png(image, path, target)`, passing
  the viewport's `surface_decision(..)` target, and the tag matches what the
  presentation engine interpreted.
- cICP cannot express *extended* range (values beyond `[0, 1]`). 8-bit
  captures are clamped anyway; extended-range (`Rgba16Float`) surfaces are
  **not capturable through the 8-bit `ScreenshotCallback` path** — it now
  fails loudly (`CaptureReadbackError::UnsupportedColorFormat`) instead of
  silently producing nothing. Lossless inspection of such a capture goes
  through `CaptureState::read_screen_floats` (float texels, values above 1.0
  preserved).

## macOS — the only runtime-verified platform

### A. SDR panel (fallback path)

1. Run `cargo run -p eframe --example hdr_wide_gamut` on an SDR display.
   Expected: `Surface status: FellBack(NoPairedColorSpace)`, `Surface target:
   Bgra8Unorm + Srgb`; the `Srgb` conversion is the identity, so the grid's
   two rows are byte-identical ((255, 0, 0) / (0, 255, 0) / (0, 0, 255) on
   both) — identical rows are the tell for the fallback path. **CI cannot
   validate this.**
2. Run `cargo run -p eframe --example hdr_smoke`. Expected: exits 0 within a
   few seconds and logs the `FellBack` decision. (CI runs this step on
   macos-latest, but confirming it on *your* SDR hardware is manual.)
   **CI cannot validate this.**

### B. P3-capable panel (granted path)

1. Run `hdr_wide_gamut` on an EDR iMac or Pro Display XDR.
   Expected: `Surface status: Granted`, `Surface target:
   Bgra8Unorm + DisplayP3`. The grid's top row reads as the familiar sRGB
   primaries while the bottom row is visibly deeper and more saturated —
   outside the sRGB gamut. **CI cannot validate this.**
2. Instrumented check (screenshot piped through a color-managed viewer or a
   colorimeter): the bottom-row red patch measures
   inside Display P3, outside sRGB; the top-row red patch measures as
   ordinary sRGB red. **CI cannot validate this.**
3. Compositor-capture discriminator: answers whether the granted
   `DisplayP3` surface is actually P3-referred at the compositor, or wgpu
   merely reports the pairing. Capture the window with Screen Recording
   permission (`screencapture -x -o -l<windowid>`), convert the capture to
   sRGB (`sips --matchTo '/System/Library/ColorSync/Profiles/sRGB
   Profile.icc'`), and decode the top-row red patch (bytes (234, 51, 35),
   physically ordinary sRGB red): reading ≈ (255, 0, 0)
   means the P3-referred encoding round-tripped back to sRGB red — the layer
   is P3-referred; reading ≈ (234, 51, 35) means the bytes passed through
   untouched — the layer is still sRGB-referred despite the pairing. The
   bottom-row (native) red patch reads (255, 0, 0) either way (the P3 primary
   lies outside the sRGB gamut and clips on conversion), so it is not
   discriminative on its own. Tolerances ±3 per channel for capture and
   quantization noise. **CI cannot validate this.**
4. Run `hdr_smoke` on the same display. Expected: exit 0, log shows the
   `Granted` pairing. **CI cannot validate this.**

### C. Monitor drag re-decide

1. Machine with two displays of different gamuts (one P3-capable, one SDR).
2. Run `hdr_wide_gamut` on the first display and note the decision.
3. Drag the window to the other display.
   Expected: within a frame or two the readout shows the other display's
   decision, with no new window, no renderer recreation, and no visual
   breakage beyond the reconfigure; dragging back restores the original
   decision. **CI cannot validate this.**

### D. Interaction checkboxes (plan §9)

- **`present_with_transaction` jitter-fix × HDR × drag-resize**: run
  `cargo run -p eframe --example hdr_wide_gamut --features
  egui-wgpu/macos-window-resize-jitter-fix` on a P3 display and drag-resize
  the window continuously for ~10 s. Expected: no jitter, no wgpu validation
  errors in the log (reconfigures rebuild from the viewport's stored
  decision). **CI cannot validate this.**
- **HDR surface × drag-resize** (the fp16 leg activates with PR 3): run
  `hdr_smoke`-style `WideGamutHdr` (e.g. temporarily point `hdr_wide_gamut`
  at `SurfacePreference::WideGamutHdr`) on an EDR display and drag-resize for
  ~10 s. Expected: no jitter, no validation errors; the fp16 surface
  reconfigures from the stored decision. **CI cannot validate this.**
- **Transparency: compositor behavior of premultiplied >1.0 content** (the
  fp16 surface and >1.0 content exist since PR 2b/PR 3): with a transparent
  window on a compositor that honors extended-range alpha, premultiplied
  >1.0 content must composite without halos or clamping artifacts.
  **CI cannot validate this.**
- **Headroom reactivity**: on an EDR Mac, run `hdr_wide_gamut` and drag the
  display-brightness slider (or toggle an HDR video next to it). Expected:
  the `HDR headroom` readout follows the ~1–2 s EDR ramp (the painter
  schedules ~100 ms repaints while it moves) and then rests — the process
  idles in `ControlFlow::Wait` at 0% CPU once the value is steady.
  **CI cannot validate this.**

### E. HDR color editing widgets (PR 5)

The `color_edit_button_hdr` + `swatch_hdr` row at the bottom of
`hdr_float_image`. The edits are linear RGB (NOT gamma-encoded) in 0..=1 and
an exposure in stops (−6..=+6, ¼-step); the effective color is
`rgb × 2^stops`, 1.0 = SDR reference white.

1. Run `hdr_float_image` (it requests `WideGamutHdr`) on an EDR Mac.
   Expected: `Granted` with an fp16 extended pairing
   (`Rgba16Float + ExtendedDisplayP3`). The big `swatch_hdr` swatch shows
   the effective color as **brightness above SDR white** — drag the
   exposure slider past 0 stops and the swatch visibly glows above the
   surrounding UI, peaking at 1.0 × 2⁶ = 64× SDR white. In the opened
   picker, the two preview swatches differ above SDR white: the left one
   (float-texture path) is brighter, the right one (clamped `Color32`)
   sits at SDR white. **CI cannot validate this.**
2. Same command on an SDR panel (deterministic `FellBack` onto the sRGB
   target). Expected: the swatch clamps at white once the effective value
   exceeds 1.0 — pushing the exposure slider runs it into flat white and
   stopping there — and in the picker the two preview swatches coincide at
   the clamp. That is the correct §7 behavior, made visible by the
   pair. **CI cannot validate this.**
3. Cache sanity (any panel): scrub a channel slider back and forth — the
   swatch updates without texture churn or flicker (swatch textures are
   cached per 8-bit encoded color bucket, 256 entries, cleared wholesale
   at the cap; a cap overflow is invisible beyond a one-frame re-upload).
   **CI cannot validate this.**

## Windows — manual-only

PR 1's `WideGamut` pairing (u8 + `DisplayP3`) is never reported on DX12; that
surface must fall back deterministically.

1. Run `hdr_wide_gamut`. Expected: `FellBack(NoPairedColorSpace)` with a
   plain sRGB target, on every GPU vendor. **CI cannot validate this.**
   (CI compile-checks the platform; nothing else.)

### scRGB-linear (PR 4): HDR brightness + a wide-gamut-capable surface

Windows 11 22H2+ with **Advanced Color / HDR turned on in Windows display
settings**. This is the pairing the platform dictates (§7): the DWM
composites in linear scRGB, so egui renders through its linear path — the
shader gamma-decodes and scales by `sdr_white_nits / 80` so egui's 1.0 stays
the SDR reference white. **CI cannot validate this.**

1. Run `cargo run -p eframe --example hdr_smoke`. Expected: exit 0, log
   shows `Granted` with `Rgba16Float + ExtendedSrgbLinear` (the
   `WideGamutHdr` request rides the platform's scRGB pairing) and
   `hdr_headroom` reported as the nits ratio (`max_nits / sdr_white_nits`).
   **CI cannot validate this.**
2. Brightness of SDR chrome (the SDR-white anchor): with `hdr_smoke` (the
   scRGB pairing) on screen, drag the Windows **SDR content brightness**
   slider (Settings → System → Display → HDR). Expected: egui's chrome
   brightens/dims together with surrounding SDR apps — the slider's
   `sdr_white_nits` reaches the linear path as a per-frame uniform
   (`scale = nits / 80`), while out-of-SDR content (values above 1.0) keeps
   its headroom above the moving white. (`hdr_wide_gamut` deterministically
   falls back to `(Bgra8Unorm, Srgb)` on Windows: its chrome also tracks the
   slider, but through the DWM's SDR scaling of the plain sRGB surface — not
   through egui's uniform.) **CI cannot validate this.**
3. Float content end-to-end: run `hdr_float_image` (it requests
   `SurfacePreference::WideGamutHdr`) on the scRGB surface.
   Expected: the >1.0 swatches render as brightness above SDR white; the
   swatches' SDR values look identical to their appearance in an sRGB app.
   **CI cannot validate this.**
4. Fallback cross-check: turn HDR/Advanced Color off in Windows settings and
   rerun `hdr_smoke`. Expected: deterministic
   `FellBack(NoPairedColorSpace)` onto the plain sRGB target (the platform
   no longer reports the extended space — nothing is assumed). Expected
   visual: identical SDR rendering to pre-PR frames. **CI cannot validate
   this.**

## Linux (KWin) — manual-only

Vulkan exposes wide color spaces only via `VK_EXT_swapchain_colorspace`, so
the outcome is driver- and compositor-dependent.

1. On KWin with the extension: `hdr_wide_gamut` may report `Granted` with a
   wide pairing. Without it: deterministic `FellBack`. Record what your
   driver reports. **CI cannot validate this.**
2. scRGB-linear (PR 4): on KWin (Wayland, HDR enabled) with a driver that
   reports `HDR_EXTENDED_SRGB_LINEAR` on fp16, `hdr_smoke` may now grant
   `Rgba16Float + ExtendedSrgbLinear` and exit 0. Expected nits behavior:
   **no** slider tracking — `DisplayLuminance` (and with it
   `sdr_white_nits`) is Windows/DXGI-only, so the linear path keeps the
   standard 80-nit anchor (scale 1.0); egui's SDR white sits at 80 nits
   regardless of the compositor's SDR-white setting, which may read darker
   or lighter than surrounding apps. That is the honest no-report default,
   not a bug. **CI cannot validate this.**

## Web — compile-checked only

PR 1 makes no web runtime claims: wasm keeps compiling with the new types
(`cargo check -p egui-wgpu --target wasm32-unknown-unknown --features
winit`), and the decider/builder have no web runtime call sites yet. When
the decider runs on web (future work), browsers reporting `DISPLAY_P3` for
u8 canvases will pair it the same way — until then there is no manual
protocol for web and nothing to checkbox.

## What "passing" means for the PR

All macOS steps green on both a P3-capable panel and an SDR panel, plus the
platform-specific steps for every platform the PR claims (Windows: fallback
determinism; Linux: whatever the driver reports, recorded). CI covers the
smoke's integration assertions; it cannot see a panel.
