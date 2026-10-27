# Manual testing — wide-gamut surface (PR 1)

**"Tested" means "this checklist passes."** Everything below exercises the
parts of the wide-gamut surface work that a machine cannot judge: what a
compositor does with the surface, and what a panel does with the colors.
Each step is marked **CI cannot validate this.**

## Instruments

| Command | What it shows |
|---|---|
| `cargo run -p eframe --example hdr_smoke` | Windowed smoke: opens a window with `SurfacePreference::WideGamut`, asserts the surfaced decision is coherent, logs the pairing, exits 0 after a few frames. |
| `cargo run -p eframe --example hdr_wide_gamut` | Live readout: `ViewportInfo::surface_status` / `surface_target`, a callback quad holding the pure Display P3 primaries (sRGB-encoded), and an "sRGB red" vs "P3 red" swatch row. |

The log line `Surface decision: Bgra8Unorm + DisplayP3 (Granted)` (or the
example's on-screen readout) is the primary reading throughout.

Expected pairings on this PR, by panel:

| Panel | Expected `surface_status` | Expected `surface_target` |
|---|---|---|
| P3-capable (EDR iMac, Pro Display XDR, recent MacBook panels) | `Granted` | `Bgra8Unorm + DisplayP3` |
| SDR panel reporting SRGB | `FellBack(NoPairedColorSpace)` | `Bgra8Unorm + Srgb` |
| Headless / no surface (offscreen harness) | `None` (fields unset) | `None` |

Caveat: macOS reports what the *screen mode* offers, not what the panel can
show — some SDR displays still get `DisplayP3`. That grant is correct; the
*out-of-sRGB saturation* then needs an actually-P3-capable panel to be
visible. Headroom readout (`hdr_headroom`) arrives with PR 3 and is
deliberately absent from this checklist.

## macOS — the only runtime-verified platform

### A. SDR panel (fallback path)

1. Run `cargo run -p eframe --example hdr_wide_gamut` on an SDR display.
   Expected: `Surface status: FellBack(NoPairedColorSpace)`, `Surface target:
   Bgra8Unorm + Srgb`; the quad reads as ordinary orange/green/blue (the P3
   bytes re-read as sRGB), and the quad's red band matches the "P3 red"
   swatch, not the "sRGB red" one. **CI cannot validate this.**
2. Run `cargo run -p eframe --example hdr_smoke`. Expected: exits 0 within a
   few seconds and logs the `FellBack` decision. (CI runs this step on
   macos-latest, but confirming it on *your* SDR hardware is manual.)
   **CI cannot validate this.**

### B. P3-capable panel (granted path)

1. Run `hdr_wide_gamut` on an EDR iMac or Pro Display XDR.
   Expected: `Surface status: Granted`, `Surface target:
   Bgra8Unorm + DisplayP3`. The quad's bands are visibly more saturated than
   the sRGB primaries; the "P3 red" swatch matches the quad's red band.
   **CI cannot validate this.**
2. Instrumented check (screenshot piped through a color-managed viewer or a
   colorimeter): the quad's red band measures inside Display P3, outside
   sRGB. **CI cannot validate this.**
3. Run `hdr_smoke` on the same display. Expected: exit 0, log shows the
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

- **`present_with_transaction` jitter-fix × wide gamut × drag-resize**:
  run `cargo run -p eframe --example hdr_wide_gamut --features
  egui-wgpu/macos-window-resize-jitter-fix` on a P3 display and drag-resize
  the window continuously for ~10 s. Expected: no jitter, no
  wgpu validation errors in the log (reconfigures rebuild from the viewport's
  stored decision). The HDR leg of this checkbox is *not testable in PR 1* —
  it activates with PR 3 (fp16 surface); wide gamut is the PR 1 stand-in.
  **CI cannot validate this.**
- **Transparency: compositor behavior of premultiplied >1.0 content**: *not
  testable in PR 1* — content above 1.0 arrives with PR 3 (float content).
  The checkbox is recorded here for PR 3: with a transparent window on a
  compositor that honors extended-range alpha, premultiplied >1.0 content
  must composite without halos or clamping artifacts. **CI cannot validate
  this.**

## Windows — manual-only

PR 1 pairings are never reported on DX12; the surface must fall back
deterministically.

1. Run `hdr_wide_gamut`. Expected: `FellBack(NoPairedColorSpace)` with a
   plain sRGB target, on every GPU vendor. **CI cannot validate this.**
   (CI compile-checks the platform; nothing else.)

## Linux — manual-only

Vulkan exposes wide color spaces only via `VK_EXT_swapchain_colorspace`, so
the outcome is driver- and compositor-dependent.

1. On KWin with the extension: `hdr_wide_gamut` may report `Granted` with a
   wide pairing. Without it: deterministic `FellBack`. Record what your
   driver reports. **CI cannot validate this.**

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
