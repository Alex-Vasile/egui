# Changelog for egui-wgpu
All notable changes to the `egui-wgpu` integration will be noted in this file.


This file is updated upon each release.
Changes since the last release can be found at <https://github.com/emilk/egui/compare/latest...HEAD> or by running the `scripts/generate_changelog.py` script.


## Unreleased

### ⚠️ Breaking changes
* `Renderer::new` no longer takes an output color format: render pipelines are created lazily, keyed by the render-target format
* `Renderer::update_buffers`, `Renderer::render` and `CallbackTrait::prepare`/`CallbackTrait::paint` now receive the actual `wgpu::TextureFormat` of the render pass they paint into (it is decided per viewport and can change when a window moves between monitors)
* Removed `RenderState::target_format` — there is no shared format anymore; each viewport stores its own decision
* `CaptureState::read_screen_rgba` now returns `Result<(), CaptureReadbackError>`: captures of formats the 8-bit path cannot decode — today `Rgba16Float` — fail loudly with `CaptureReadbackError::UnsupportedColorFormat` instead of silently logging and producing nothing (read those back losslessly with the new `CaptureState::read_screen_floats`)

### ⭐ Added
* Add `SurfacePreference` (a new field on `WgpuConfiguration`): ask for `WideGamut` at create time and each viewport gets the best wide-gamut pairing the platform reports, or a plain SDR surface with an explanatory status — never an error
* The `SurfacePreference` enum is complete: `Hdr` (extended-range sRGB, gamut pinned — `(Rgba16Float, ExtendedSrgb)` when reported) and `WideGamutHdr` (both axes, all-or-nothing — `(Rgba16Float, ExtendedDisplayP3)`), each walking its own priority table (`HDR_ORDER`, `WIDE_GAMUT_HDR_ORDER`) over the reported capabilities and falling back to plain SDR with a `FellBack` status when ungrantable
* Add `SurfaceTarget` (format + color space), `SurfaceStatus` (`Granted` / `FellBack`), `FallbackReason`, `WIDE_GAMUT_ORDER`, and the `decide_surface_target` / `build_configuration` API that turns a preference into a validated `wgpu::SurfaceConfiguration`
* Per-viewport surface decisions are stored by the winit painter and re-decided when a window moves to another monitor; the painter exposes the decision per viewport (surfaced to apps via `egui::ViewportInfo`)
* Display headroom pass-through: the winit painter polls the surface's tone-map headroom at surface creation and once per frame (main thread), stores it per viewport, exposes it via the new `Painter::surface_headroom`, and schedules ~100 ms adaptive repaints while the value is moving or unsettled (an EDR ramp lasts ~1–2 s; once steady — including the `Some(1.0)` SDR steady state — egui rests in `ControlFlow::Wait`). Surfaced to apps as `egui::ViewportInfo::hdr_headroom`
* Float readback: the new `readback` module decodes `Rgba16Float` frames to float texels (`texture_to_float_image`, single shared binary16 decoder), preserving values above 1.0, negatives and subnormals; `CaptureState::read_screen_floats` applies it to extended-range captures
* Tagged float content (`ImageData::Float`) now uploads: float textures are created as `Rgba16Float` (zero-copy from the f16 pixels), and `bytes_per_row` for the float path is f16-aware (`8 * width`, padded to wgpu's 256-byte alignment with rows repacked to match)
* Add `Renderer::set_active_surface_color_space`: the painter (winit / web) feeds each frame's granted surface color space in under its renderer lock, before the per-frame `update_texture` calls. Unset (the default) keeps today's sRGB semantics, so app-driven/headless uploads (kittest, custom integrations) are unchanged
* The content↔surface upload pairing (plan §4) gates float uploads inside `update_texture` without a signature change: sRGB-tagged content is native on sRGB spaces and shifts on DisplayP3 like the chrome; DisplayP3-tagged content is native only on DisplayP3 (± extended) and is **loudly rejected** on sRGB spaces (and on the scRGB-linear pairing, which arrives with the linear stage) — the upload is not performed, a warning logs once per texture id (carrying the evaluated content primaries and active surface color space), and egui's standard placeholder renders. Partial patches whose content kind does not match the texture's format (Color onto float, float onto Color) are likewise skipped with a once-per-id warning — the previous content stays. The p…
* `Renderer::update_texture`-created textures now carry `COPY_SRC` so uploads can be read back for verification (plan §8 row 2b)

### 🐛 Fixed
* Dithering is now structurally off on float targets: pipelines for `Rgba16Float` are built from a shader module with the dither application compiled out (a float target has no 8-bit quantizer for the noise to hide in — near black the ±1/255 encoded noise maps to ×12.92 luminance grain), regardless of `RendererOptions::dithering`. 8-bit targets keep today's behavior bit-for-bit; the `dithering` knob's doc now says it assumes an 8-bit sRGB target
* WGSL transfer functions no longer evaluate `pow` on possibly-negative values (WGSL-undefined, NaN-prone on drivers that lower `select` to arithmetic blends): negatives take the odd-symmetric `sign(x)·f(|x|)` extension, matching `ecolor`; results for positive values are bit-identical (at `-0.0` the zero's sign bit can differ; the values compare equal)
* Web now honors `WgpuConfiguration.surface.desired_maximum_frame_latency` instead of silently using wgpu's default (observable as changed frame latency on web)
* The screen-capture state is recreated when the render-target format changes (e.g. after a monitor re-decide) instead of capturing into a stale-format texture

### 🔧 Changed
* `pollster` is now a dev-dependency (drives the crate's first runtime test, the pipeline-cache test)
* `half` (already locked at 2.7.1, `bytemuck` feature) is a direct dependency of `epaint` (the f16 content type) and a dev-dependency here (exact-bit round-trip assertions)


## 0.37.0 - 2026-10-15
* Fix `Frame::set_wgpu_surface_config` being ignored [#8442](https://github.com/emilk/egui/pull/8442) by [@chicoferreira](https://github.com/chicoferreira)


## 0.36.2 - 2026-09-08
* Fix `egui-wgpu` build on `wasm32-unknown-emscripten` [#8454](https://github.com/emilk/egui/pull/8454) by [@bfeip](https://github.com/bfeip)


## 0.36.1 - 2026-08-07
Nothing new


## 0.36.0 - 2026-08-05
* Upgrade wgpu to v30 [#8289](https://github.com/emilk/egui/pull/8289) by [@akx](https://github.com/akx)
* Fix: ensure mapped range is dropped before unmapping buffer in capture [#8337](https://github.com/emilk/egui/pull/8337) by [@MagicCrazyMan](https://github.com/MagicCrazyMan)
* Make wgpu Instance public [#8321](https://github.com/emilk/egui/pull/8321) by [@oleflb](https://github.com/oleflb)


## 0.35.0 - 2026-06-25
* Call `pre_present_notify` before presenting [#8089](https://github.com/emilk/egui/pull/8089) by [@dimtpap](https://github.com/dimtpap)
* Wgpu: Allow configuring VSync and frame latency at runtime [#8114](https://github.com/emilk/egui/pull/8114) by [@emilk](https://github.com/emilk)
* Configure wgpu to be low-latency by default [#8203](https://github.com/emilk/egui/pull/8203) by [@lucasmerlin](https://github.com/lucasmerlin)
* Fix "drunk text" bug [#8250](https://github.com/emilk/egui/pull/8250) by [@emilk](https://github.com/emilk)


## 0.34.3 - 2026-05-27
* Fix random hangs by improving `wgpu::Surface` lifecycle handling [#8171](https://github.com/emilk/egui/pull/8171) by [@grtlr](https://github.com/grtlr)


## 0.34.2 - 2026-05-04
* Update to wgpu 29.0.1 [#8073](https://github.com/emilk/egui/pull/8073) by [@emilk](https://github.com/emilk)
* Warn if using a software rasterizer [#8101](https://github.com/emilk/egui/pull/8101) by [@emilk](https://github.com/emilk)


## 0.34.1 - 2026-03-27
* `wgpu` backend: Enable WebGL fallback [#8038](https://github.com/emilk/egui/pull/8038) by [@emilk](https://github.com/emilk)


## 0.34.0 - 2026-03-26
### ⭐ Added
* Add error message when calling `.render()` without `.update_buffers()` [#8005](https://github.com/emilk/egui/pull/8005) by [@emilk](https://github.com/emilk)

### 🔧 Changed
* Put the `capture` module behind a feature flag, make the `egui` dependency optional [#7698](https://github.com/emilk/egui/pull/7698) by [@StT191](https://github.com/StT191)
* Attach stencil buffer [#7702](https://github.com/emilk/egui/pull/7702) by [@jgraef](https://github.com/jgraef)
* Update wgpu to 28.0.0 [#7853](https://github.com/emilk/egui/pull/7853) by [@SuchAFuriousDeath](https://github.com/SuchAFuriousDeath)
* Update to wgpu 29 [#7990](https://github.com/emilk/egui/pull/7990) by [@cwfitzgerald](https://github.com/cwfitzgerald)

### 🐛 Fixed
* Fix wgpu memory leak leading to panic when window is minimized (#7434) [#7928](https://github.com/emilk/egui/pull/7928) by [@landaire](https://github.com/landaire)


## 0.33.3 - 2025-12-11
Nothing new


## 0.33.2 - 2025-11-13
* Fix jittering during window resize on MacOS for WGPU/Metal [#7641](https://github.com/emilk/egui/pull/7641) by [@aspcartman](https://github.com/aspcartman)


## 0.33.0 - 2025-10-09
### 🔧 Changed
* Update wgpu to 26 and wasm-bindgen to 0.2.100 [#7540](https://github.com/emilk/egui/pull/7540) by [@Kumpelinus](https://github.com/Kumpelinus)
* Warn if `DYLD_LIBRARY_PATH` is set and we find no wgpu adapter [#7572](https://github.com/emilk/egui/pull/7572) by [@emilk](https://github.com/emilk)
* Update MSRV from 1.86 to 1.88 [#7579](https://github.com/emilk/egui/pull/7579) by [@Wumpf](https://github.com/Wumpf)
* Update wgpu to 27.0.0 [#7580](https://github.com/emilk/egui/pull/7580) by [@Wumpf](https://github.com/Wumpf)
* Create `egui_wgpu::RendererOptions` [#7601](https://github.com/emilk/egui/pull/7601) by [@emilk](https://github.com/emilk)
* Use software texture filtering in kittest [#7602](https://github.com/emilk/egui/pull/7602) by [@emilk](https://github.com/emilk)


## 0.32.3 - 2025-09-12
Nothing new


## 0.32.2 - 2025-09-04
Nothing new


## 0.32.1 - 2025-08-15
* Enable wgpu default features in eframe / egui_wgpu default features [#7344](https://github.com/emilk/egui/pull/7344) by [@lucasmerlin](https://github.com/lucasmerlin)


## 0.32.0 - 2025-07-10
* Update to wgpu 25 [#6744](https://github.com/emilk/egui/pull/6744) by [@torokati44](https://github.com/torokati44)
* Free textures after submitting queue instead of before with wgpu renderer on Web [#7291](https://github.com/emilk/egui/pull/7291) by [@Wumpf](https://github.com/Wumpf)
* Improve texture filtering by doing it in gamma space [#7311](https://github.com/emilk/egui/pull/7311) by [@emilk](https://github.com/emilk)


## 0.31.1 - 2025-03-05
Nothing new


## 0.31.0 - 2025-02-04
* Upgrade to wgpu 24 [#5610](https://github.com/emilk/egui/pull/5610) by [@torokati44](https://github.com/torokati44)
* Extend `WgpuSetup`, `egui_kittest` now prefers software rasterizers for testing [#5506](https://github.com/emilk/egui/pull/5506) by [@Wumpf](https://github.com/Wumpf)
* Wgpu resources are no longer wrapped in `Arc` (since they are now `Clone`) [#5612](https://github.com/emilk/egui/pull/5612) by [@Wumpf](https://github.com/Wumpf)


## 0.30.0 - 2024-12-16
* Fix docs.rs build [#5204](https://github.com/emilk/egui/pull/5204) by [@lucasmerlin](https://github.com/lucasmerlin)
* Free textures after submitting queue instead of before with wgpu renderer [#5226](https://github.com/emilk/egui/pull/5226) by [@Rusty-Cube](https://github.com/Rusty-Cube)
* Add option to initialize with existing wgpu instance/adapter/device/queue [#5319](https://github.com/emilk/egui/pull/5319) by [@ArthurBrussee](https://github.com/ArthurBrussee)
* Updare to `wgpu` 23.0.0 and `wasm-bindgen` to 0.2.95 [#5330](https://github.com/emilk/egui/pull/5330) by [@torokati44](https://github.com/torokati44)
* Support wgpu-tracing with same mechanism as wgpu examples [#5450](https://github.com/emilk/egui/pull/5450) by [@EriKWDev](https://github.com/EriKWDev)


## 0.29.1 - 2024-10-01
Nothing new


## 0.29.0 - 2024-09-26 - `wgpu` 22.0
### ⭐ Added
* Add opt-out `fragile-send-sync-non-atomic-wasm` feature for wgpu [#5098](https://github.com/emilk/egui/pull/5098) by [@9SMTM6](https://github.com/9SMTM6)

### 🔧 Changed
* Upgrade to wgpu 22.0.0 [#4847](https://github.com/emilk/egui/pull/4847) by [@KeKsBoTer](https://github.com/KeKsBoTer)
* Introduce dithering to reduce banding [#4497](https://github.com/emilk/egui/pull/4497) by [@jwagner](https://github.com/jwagner)
* Ensure that `WgpuConfiguration` is `Send + Sync` [#4803](https://github.com/emilk/egui/pull/4803) by [@murl-digital](https://github.com/murl-digital)
* Wgpu render pass on paint callback has now `'static` lifetime [#5149](https://github.com/emilk/egui/pull/5149) by [@Wumpf](https://github.com/Wumpf)

### 🐛 Fixed
* Update sampler along with texture on wgpu backend [#5122](https://github.com/emilk/egui/pull/5122) by [@valadaptive](https://github.com/valadaptive)


## 0.28.1 - 2024-07-05
Nothing new


## 0.28.0 - 2024-07-03
* Update to wgpu 0.20 [#4433](https://github.com/emilk/egui/pull/4433) by [@KeKsBoTer](https://github.com/KeKsBoTer)
* Fix doclinks in egui-wgpu docs [#4677](https://github.com/emilk/egui/pull/4677) by [@emilk](https://github.com/emilk)


## 0.27.2 - 2024-04-02
* Nothing new


## 0.27.1 - 2024-03-29
* Nothing new


## 0.27.0 - 2024-03-26
* Improve panic message in egui-wgpu when failing to create buffers [#3986](https://github.com/emilk/egui/pull/3986)


## 0.26.2 - 2024-02-14
* Nothing new


## 0.26.1 - 2024-02-11
* Improve panic message in egui-wgpu when failing to create buffers [#3986](https://github.com/emilk/egui/pull/3986)


## 0.26.0 - 2024-02-05
* Update wgpu to 0.19 [#3824](https://github.com/emilk/egui/pull/3824)
* Add `WgpuConfiguration::desired_maximum_frame_latency` [#3874](https://github.com/emilk/egui/pull/3874)
* Disable the default features of `wgpu` [#3875](https://github.com/emilk/egui/pull/3875)
* If WebGPU fails, re-try adapter creation with WebGL [#3895](https://github.com/emilk/egui/pull/3895) (thanks [@Wumpf](https://github.com/Wumpf)!)
* Delay call to `get_current_texture` (possible small performance win) [#3914](https://github.com/emilk/egui/pull/3914)
* Add `x11` and `wayland` features [#3909](https://github.com/emilk/egui/pull/3909) (thanks [@YgorSouza](https://github.com/YgorSouza)!)
* Pass `ScreenDescriptor` to `egui_wgpu::CallbackTrait::prepare` [#3960](https://github.com/emilk/egui/pull/3960) (thanks [@StratusFearMe21](https://github.com/StratusFearMe21)!)
* Make `egui_wgpu::renderer` a private module [#3979](https://github.com/emilk/egui/pull/3979)


## 0.25.0 - 2024-01-08
* Only call wgpu paint callback if viewport is positive [#3778](https://github.com/emilk/egui/pull/3778) (thanks [@msparkles](https://github.com/msparkles)!)


## 0.24.1 - 2023-11-30
* Add a few `puffin` profile scopes


## 0.24.0 - 2023-11-23
* Updated to wgpu 0.18 [#3505](https://github.com/emilk/egui/pull/3505) (thanks [@Wumpf](https://github.com/Wumpf)!)
* Update MSRV to Rust 1.72 [#3595](https://github.com/emilk/egui/pull/3595)
* Properly clamp and round viewport values, preventing rare warnings [#3604](https://github.com/emilk/egui/pull/3604) (thanks [@Wumpf](https://github.com/Wumpf)!)


## 0.23.0 - 2023-09-27
* Update to `wgpu` 0.17.0 [#3170](https://github.com/emilk/egui/pull/3170) (thanks [@Aaron1011](https://github.com/Aaron1011)!)
* Improved wgpu callbacks [#3253](https://github.com/emilk/egui/pull/3253) (thanks [@Wumpf](https://github.com/Wumpf)!)
* Fix depth texture init with multisampling [#3207](https://github.com/emilk/egui/pull/3207) (thanks [@mauliu](https://github.com/mauliu)!)
* Fix panic on wgpu GL backend due to new screenshot capability [#3078](https://github.com/emilk/egui/pull/3078) (thanks [@amfaber](https://github.com/amfaber)!)


## 0.22.0 - 2023-05-23
* Update to wgpu 0.16 [#2884](https://github.com/emilk/egui/pull/2884) (thanks [@niklaskorz](https://github.com/niklaskorz)!)
* Device configuration is now dependent on adapter [#2951](https://github.com/emilk/egui/pull/2951) (thanks [@Wumpf](https://github.com/Wumpf)!)
* Expose `wgpu::Adapter` via `RenderState` [#2954](https://github.com/emilk/egui/pull/2954) (thanks [@Wumpf](https://github.com/Wumpf)!)
* Add `read_screen_rgba` to the egui-wgpu `Painter`, to allow for capturing the current frame when using wgpu. Used in conjunction with `Frame::request_screenshot` [#2676](https://github.com/emilk/egui/pull/2676)
* Improve performance of `update_buffers` [#2820](https://github.com/emilk/egui/pull/2820) (thanks [@Wumpf](https://github.com/Wumpf)!)
* Added support for multisampling (MSAA) [#2878](https://github.com/emilk/egui/pull/2878) (thanks [@PPakalns](https://github.com/PPakalns)!)


## 0.21.0 - 2023-02-08
* Update to `wgpu` 0.15 ([#2629](https://github.com/emilk/egui/pull/2629))
* Return `Err` instead of panic if we can't find a device ([#2428](https://github.com/emilk/egui/pull/2428)).
* `winit::Painter::set_window` is now `async` ([#2434](https://github.com/emilk/egui/pull/2434)).
* `egui-wgpu` now only depends on `epaint` instead of the entire `egui` ([#2438](https://github.com/emilk/egui/pull/2438)).
* `winit::Painter` now supports transparent backbuffer ([#2684](https://github.com/emilk/egui/pull/2684)).


## 0.20.0 - 2022-12-08 - web support
* Renamed `RenderPass` to `Renderer`.
* Renamed `RenderPass::execute` to `RenderPass::render`.
* Renamed `RenderPass::execute_with_renderpass` to `Renderer::render` (replacing existing `Renderer::render`)
* Reexported `Renderer`.
* You can now use `egui-wgpu` on web, using WebGL ([#2107](https://github.com/emilk/egui/pull/2107)).
* `Renderer` no longer handles pass creation and depth buffer creation ([#2136](https://github.com/emilk/egui/pull/2136))
* `PrepareCallback` now passes `wgpu::CommandEncoder` ([#2136](https://github.com/emilk/egui/pull/2136))
* `PrepareCallback` can now returns `wgpu::CommandBuffer` that are bundled into a single `wgpu::Queue::submit` call ([#2230](https://github.com/emilk/egui/pull/2230))
* Only a single vertex & index buffer is now created and resized when necessary (previously, vertex/index buffers were allocated for every mesh) ([#2148](https://github.com/emilk/egui/pull/2148)).
* `Renderer::update_texture` no longer creates a new `wgpu::Sampler` with every new texture ([#2198](https://github.com/emilk/egui/pull/2198))
* `Painter`'s instance/device/adapter/surface creation is now configurable via `WgpuConfiguration` ([#2207](https://github.com/emilk/egui/pull/2207))
* Fix panic on using a depth buffer ([#2316](https://github.com/emilk/egui/pull/2316))


## 0.19.0 - 2022-08-20
* Enables deferred render + surface state initialization for Android ([#1634](https://github.com/emilk/egui/pull/1634)).
* Make `RenderPass` `Send` and `Sync` ([#1883](https://github.com/emilk/egui/pull/1883)).


## 0.18.0 - 2022-05-15
First published version since moving the code into the `egui` repository from <https://github.com/LU15W1R7H/eww>.
