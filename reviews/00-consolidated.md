# Consolidated review — hdr-egui-upstream.md

Deduplicated merge of six independent reviews (`reviews/01`–`reviews/06`): upstream
acceptance, color science, wgpu/platform ground truth, API design, sequencing/scope,
testing/CI. 78 raw findings → 41 distinct entries; duplicates merged with "raised by"
attribution. Full detail stays in the per-reviewer files.

**Verified facts every entry builds on:** tag `0.36.2` exists in this fork and
`crates/egui_glow` IS present and functional at that tag; `0.36.2` is *not* an ancestor of
HEAD (213/16 commits diverged); upstream `main` has moved further (a live HDR PR exists).
Reviewers verified plan anchors via `git show 0.36.2:` and against
`~/.cargo/registry/.../wgpu-types-30.0.1` (and `wgpu-core/hal-30.0.1`).

**Severity totals:** 6 critical · 17 major · 15 minor · 3 question.

**Suggested walkthrough order:** the numbered sections below follow the plan's own
structure (strategy → presentation/lifecycle → color contract → API → tiers → testing →
sequencing) so plan updates land section by section. Start with §A/§B criticals.

---

## A. Strategy & upstream process

### 1. Upstream tracker premise is stale: live PR #8703 already occupies tier B — critical
- **Raised by:** 01 (stale-tracker-premise, demand-case-inverted)
- **Finding:** The plan assumes a vacant upstream ("no open egui issue or PR requests HDR",
  checked 2026-09-11). As of 2026-10-12, PR
  [#8703](https://github.com/emilk/egui/pull/8703) (`SurfaceConfig::prefer_hdr: bool` +
  `preferred_hdr_framebuffer_format()`) ships tier B with ~+200/−12 lines across exactly
  the plan's files (lib.rs, winit.rs, renderer.rs, capture.rs): Rgba16Float +
  `EXTENDED_SRGB_LINEAR`, fp16 entry-point switch, fp16 capture fix. Hardware-tested on
  Linux/Wayland, cites production use (LightCraft), currently BLOCKED with zero maintainer
  comments. Filing PR 1 as written would be duplicate work by egui's own etiquette
  (CONTRIBUTING.md:30). The absent-demand weakness has inverted: two independent parties
  now need tier B — that is exactly the ecosystem demand emilk asks for.
- **Evidence:** plan header + §7.9; PR #8703 (live check 2026-10-24); CONTRIBUTING.md:30.
- **Suggested decision:** Rewrite §7.9 around #8703: review it (the plan's encoding
  analysis is exactly the review its `Auto`/scRGB choice needs), test it against our app on
  macOS, position `HdrPreference` + `HdrStatus` as a hardening follow-up that evolves the
  merged bool — not a competing first PR. Re-verify tracker status as a dated sentence.

### 2. "egui_glow does not exist at 0.36" is false; the one-backend premise needs replacing — critical
- **Raised by:** 01, 03, 04 (glow-type-home), 05, 06
- **Finding:** `crates/egui_glow` is a workspace member at 0.36.2 *and* HEAD, an opt-in
  `eframe` feature demoted from default in 0.34 (#7615), actively maintained and positively
  documented for binary-size savings. There are two live backends; "exactly one backend to
  fix" is wrong in the first paragraph an upstream reviewer reads. The plan draws its
  "scandalously cheap" conclusion, tier E's consumer list, and the §7.4 breaking-change
  citation ("glow removed") from the false count. The defensible argument is
  capability-based: glow targets OpenGL/GLES and wgpu's GLES backend reports
  `SurfaceColorSpaces::SRGB` only — no extended range exists in that stack, so HDR is
  wgpu-only *by construction*, and glow apps stay SDR with zero breakage. Kittest has only
  a `WgpuTestRenderer`, so glow stays CI-untestable even for SDR.
- **Evidence:** `git ls-tree 0.36.2 crates/`; eframe Cargo.toml:56-61, :97-101; eframe
  CHANGELOG:80; wgpu-hal gles/adapter.rs:1345-1347; Wumpf #2712 ("each individually").
- **Suggested decision:** Correct the header. Add an explicit scope sentence: "HdrPreference
  is egui-wgpu-only; the glow backend remains SDR by design (GLES reports sRGB-only color
  spaces)." Drop "glow removed" from the 0.x-precedent citation (→ entry 30). Scope tier E
  to wgpu explicitly or add the glow painter to its consumer list (→ entry 27).

### 3. The `[patch]`-fork strategy is the riskiest path, not derisking — major
- **Raised by:** 05 (fork-patch-strategy)
- **Finding:** §7.9 floats building PR 1 on a fork and pinning the app via `[patch]`. If PR
  1 stalls a cycle (the modal outcome given review latency — #8703: 12 days, no human
  reply), the fork must chase upstream churn (213 commits since the tag; wgpu major bumps
  nearly every release; review-driven API renames) — the hand-rolled-loop maintenance
  burden the plan exists to eliminate, recreated with less control. The app's existing tier
  A workaround keeps functioning untouched. A `[patch]`ed app also can't ship until the
  fork tracks a tagged release.
- **Evidence:** plan §1, §7.9 items 2-3; divergence numbers above.
- **Suggested decision:** Invert: (1) file the upstream issue + early *draft* PR against
  upstream main (free design review, better derisking than a private fork); (2) keep the
  app on tier A until a merge lands in a release; (3) permit the fork only as a local
  scratch testbed. Pick 0.37-vs-0.38 after maintainer response.

### 4. Plan is anchored to 0.36.2, which is stale against both HEAD and upstream main — major
- **Raised by:** 05 (anchor-drift), 01 (api-shape-vs-8703 detail), 06 (glow footnote)
- **Finding:** `0.36.2` is not an ancestor of HEAD, and upstream main has moved further.
  Working-tree HEAD already has `preferred_framebuffer_format` returning
  `Result<TextureFormat, WgpuError>` (lib.rs:267-271, 419-427) — so §7.4's "disruptive
  Result-returning in-place replacement" argument is half-moot; plus
  `WgpuConfiguration::with_surface_config`, a `WgpuError` enum, kittest
  `predictable_texture_filtering` — none mentioned in the plan. Every Appendix line-number
  anchor is 0.36.2-relative. Drafting the issue from stale anchors wastes the first
  impression. Also: the plan cites the current signature as taking `&SurfaceCapabilities`
  (§7.4) when 0.36.2 takes `&[TextureFormat]` — the proposed change alters the input type,
  not just appends a parameter.
- **Evidence:** `git merge-base --is-ancestor 0.36.2 HEAD` fails; lib.rs anchors; 01's live
  upstream check.
- **Suggested decision:** Add a pre-drafting workstream: re-verify every Appendix anchor
  against upstream `main` (not the tag, not this fork) and restate §7.4's rationale against
  the current Result-returning signature.

### 5. Breaking-change justification cites wrong precedent; deprecation-first policy unaddressed — major
- **Raised by:** 01 (breaking-change-evidence), 05, 04
- **Finding:** The plan justifies the in-place 0.x break with "(0.35→0.36: wgpu major bump,
  glow removed, Renderer::new params → struct)". Verification: wgpu bump true;
  "glow removed" false; `RendererOptions` landed in 0.33 (#7601), and `Renderer::new`
  still takes it positionally at 0.36.2 — 2/3 of the citation is wrong. Meanwhile
  CONTRIBUTING.md:92 states the real policy: breaking changes tolerated, but "we try to
  first deprecate old code using `#[deprecated]`" — and the plan never engages the
  deprecation objection for a `pub fn` third parties call. 0.36.0 did ship hard removals
  (`Modifiers`, `clip_rect_margin`), so in-place breaks do happen.
- **Evidence:** 0.36.2 CHANGELOG Removed section; renderer.rs:268-272; CONTRIBUTING.md:92.
- **Suggested decision:** Replace the parenthetical with verified precedent (wgpu-30 bump;
  `Modifiers` removal). Add one sentence answering deprecation: either deprecate the 1-arg
  fn one release while `decide_target` lands, or argue the loud break (semantic failure of
  the old signature under `Prefer`).

### 6. Issue draft structure: lead with the live bug, anchor each PR to existing threads — minor
- **Raised by:** 01 (issue-hook-order, issue-anchors-missing)
- **Finding:** §7.9 proposes leading with the shader-correctness analysis. Maintainers
  triage on user-visible breakage: #7484 (black screen / double gamma, open 14 months) is
  the hook. Also, the ladder ignores free anchors: #6863 (open since 2025-04-26) already
  asks to remove the "linear (sRGBA aware) framebuffer" warning and is precisely PR 2's
  landing anchor; #7484 anchors PR 1. Referencing them is etiquette (CONTRIBUTING.md:23)
  and turns "speculative feature" into "fixes live bugs".
- **Suggested decision:** Restructure the issue draft: (1) #7484-class failure + repro,
  (2) the encoding trap, (3) proposed API + non-goals, (4) testing story. Map PR 1 →
  #7484, PR 2 → #6863, mention #8703 in both.

### 7. Non-goals need scope sentences for glow and wasm, plus the #2712 dormancy citation — minor
- **Raised by:** 01 (non-goals-hold), 03 (web-deferral-stale)
- **Finding:** The deferrals survive scrutiny, but two need rewritten *rationale*: glow
  (capability argument — entry 2) and web: wgpu 30's browser backend **already** reports
  `EXTENDED_SRGB | EXTENDED_DISPLAY_P3` for fp16 canvases, "gated on fp16-canvas support
  alone" — the plan's reason ("until wgpu's web surface color spaces settle") is outdated
  and will be corrected in review. Wide gamut: cite #2712's 3-year dormancy as evidence
  maintainers won't block HDR on gamut.
- **Suggested decision:** Keep the non-goals; reword web as "deferred for scope (egui-side
  web plumbing), wgpu 30 already supports the encoded-extended canvas path"; note web
  comes nearly free if the probe is format-agnostic.

### 8. Foundational reference `hdr-egui-port-plan.md` does not exist in the repo — minor
- **Raised by:** 05 (dangling-provenance)
- **Finding:** Line 4 links `hdr-egui-port-plan.md` as the origin of all findings; the file
  is absent and has no deletion commit in history. The upstream issue and the
  `[patch]`-fork debate inherit context from an unreadable document.
- **Suggested decision:** Restore the file or inline a summary of the workaround's
  architecture before drafting the issue; if intentionally dropped, delete the link.

---

## B. Presentation & lifecycle (§1, §4, §7.3)

### 9. wgpu 30 `Auto` + fp16 resolves to `ExtendedSrgbLinear` — the §1 failure-mode story is wrong — major
- **Raised by:** 02 (auto-colorspace-claim), 03 (auto-is-not-sdr)
- **Finding:** §1 claims an untouched `color_space` yields "an SDR-interpreted fp16
  surface". wgpu 30 documents `Auto` as resolving `Rgba16Float` → **`ExtendedSrgbLinear`**
  (scRGB/EDR), falling back to `Srgb` only if linear is unsupported
  (wgpu-core surface_config.rs `resolve_auto_color_space`). So the *status quo* on macOS is
  already an extended **linear** surface misinterpreting gamma-encoded output — washed
  mid-tones and broken headroom semantics, not SDR clipping; the §1 "black screen" story
  only occurs where fp16 is unsupported. Concrete PR-1 hazard: egui's `configure_surface`
  builds config via `..get_default_config(...)` and wgpu's default sets
  `color_space: Auto` — plumbed-but-forgotten fp16 in PR 1 ships the exact #7484-class bug
  the policy exists to prevent. Silver lining: `Auto`+fp16 already produces an EDR surface,
  which *strengthens* the case for an explicit, named policy.
- **Suggested decision:** Rewrite §1's failure-mode paragraph against the `Auto` doc; make
  "configure_surface sets `color_space` explicitly from the stored decision" a named PR-1
  invariant, and extend the §7.4 unit test to assert the built `SurfaceConfiguration` is
  coherent, not just `decide_target`'s return.

### 10. DX12 never reports `ExtendedSrgb` — Windows HDR is impossible under PR 1's policy — critical
- **Raised by:** 03 (dx12-no-extended-srgb)
- **Finding:** §7.3's `[INFERENCE]` (ExtendedSrgb likely maps to scRGB FP16 on Windows) is
  falsified: on DX12, `ExtendedSrgb` is never reported and not mappable at all
  (`map_surface_color_space` hits `unreachable!()`); the adapter reports fp16 →
  `EXTENDED_SRGB_LINEAR` only (scRGB). Consequence: PR 1's probe deterministically fails on
  every Windows machine → permanent SDR fallback. The honest label is not "Windows
  experimental" but "no HDR on Windows until the scRGB policy variant (plan PR 2) ships".
  §7.3's enthusiasm about Windows nits/chromaticity conflates the data side with the
  presentation side.
- **Evidence:** wgpu-hal dx12/adapter.rs:1313-1355; dx12/mod.rs:1452-1466; wgpu-types
  surface.rs:388-390 ("Not available on DX12…").
- **Suggested decision:** Rewrite §7.3 as a per-backend table read from capability code
  (Metal: EXTENDED_SRGB|EXTENDED_SRGB_LINEAR|P3; DX12: LINEAR only; Vulkan: driver-reported
  via VK_EXT_swapchain_colorspace; GLES: SRGB only; web: EXTENDED_SRGB gated on fp16
  canvas). State that PR 1 ships HDR on macOS (+web, +rare Vulkan drivers) and Windows
  requires the PR-2 scRGB variant; decide then whether `decide_target` gains a
  per-platform fallback (ExtendedSrgb→Linear on DX12).

### 11. `Painter::set_hdr()` full-recreate would drop the font atlas and all user TextureIds — major
- **Raised by:** 03 (set-hdr-recreate-loses-textures)
- **Finding:** §4's remedy "create-time-only plus an explicit `Painter::set_hdr()` that
  does the full recreate" under-specifies the second half. A renderer-only recreate loses
  `Renderer.textures` (font atlas, uploads) and all `register_native_texture` bind groups;
  egui's TextureManager sends *deltas* per frame, so meshes reference missing textures
  until every texture is re-touched, and user native-texture re-registration has no recipe
  (`next_user_texture_id` restarts; app-held TextureIds dangle). Recreating the egui
  Context mid-run is not implementable — apps hold `egui::Context` by value. "Context +
  state + renderer together" is not a deliverable.
- **Evidence:** renderer.rs:251-253, :545-561; context.rs:2417; winit.rs delta-only
  texture updates.
- **Suggested decision:** Drop `set_hdr()` from the plan; declare format create-time-only
  in PR 1 (matches upstream appetite and the plan's own evidence). If kept, spec it fully:
  rebuild RenderState + re-push every managed texture + return a texture-id remapping for
  user textures, documented as breaking for `register_native_texture` users.

### 12. Policy-home contradiction: §1 puts HDR on runtime-mutable `SurfaceConfig`, §4 decides create-time-only — major
- **Raised by:** 04 (surfaceconfig-create-only), 05 (runtime-vs-createtime-knob)
- **Finding:** `SurfaceConfig` is documented "Runtime-mutable subset … Edit any field to
  have the surface reconfigured on the next paint", with a real mutation flow
  (`Frame::set_wgpu_surface_config` → per-paint diff). Putting `HdrPreference` there
  advertises runtime flips that the probe silently ignores — or half-applies (reconfigure
  without pipeline rebuild = black screen, the exact #7484 failure §1 cites). §1 and §4
  disagree on which struct owns the policy; this unresolved contradiction is PR 1's most
  likely review blocker.
- **Suggested decision:** One decision, stated once: policy lives in create-time config
  (`WgpuConfiguration`/`WgpuSetupCreateNew` or a dedicated field documented
  create-time-only); `SurfaceConfig` stays runtime-mutable-only; runtime HDR changes are
  explicitly a non-goal in PR 1 (or ride `set_hdr()` per entry 11 — but pick one).

### 13. PR-1 blockers the plan defers as footnotes: probe policy, no-surface answer, Err semantics — major
- **Raised by:** 04 (pr1-undecided), 03 (probe-sees-first-surface-only)
- **Finding:** §7.4 defers "which surface's probe decides" — but 0.36.2 code already
  answers it: `RenderState::create` runs once on the first `set_window` with only that
  surface's caps; later viewports inherit the global format; a later viewport that cannot
  configure surfaces the failure as an *uncaptured device error* (default panic), not a
  recoverable `SurfaceErrorAction`. Additionally the headless/no-surface path synthesizes
  `vec![Rgba8Unorm]` (lib.rs:271-278) so the probe never fires — `HdrStatus`'s answer there
  is undefined. PR 1's core function cannot be written from the plan as it stands.
- **Suggested decision:** Write the three sentences into the plan: (1) primary/first
  surface's caps decide; late viewports inherit; per-viewport mismatch is a documented
  validation error by design, not per-viewport fallback; (2) no-surface → `Sdr { NoSurface
  }` (new reason) or normalized `Off`; (3) Err semantics of the public fn (likely
  `NoSurfaceFormatsAvailable` only, unchanged).

### 14. §7.5 corrections: iOS has no headroom; `tone_map_headroom()` has a fourth (SDR) step — minor
- **Raised by:** 03 (hdr-info-platform-nits)
- **Finding:** (a) `DisplayHeadroom` is macOS-only; `None` on iOS/tvOS/visionOS — the plan's
  "maps to NSScreen/UIScreen" implies iOS coverage it lacks. (b) Precedence is four steps:
  Apple → **`Some(1.0)` when `coarse.high_dynamic_range == Some(false)`** →
  `max_nits/sdr_white_nits` → `None`; an SDR Windows display reports `Some(1.0)`, so
  HdrStatus design should treat 1.0 as "no headroom", not probe failure. (c) `DisplayGamut`
  is `#[non_exhaustive]` and unordered — pass-through only; egui-side matching is a
  forward-compat hazard.
- **Suggested decision:** Patch §7.5's bullets with (a) and (b); add the
  `Some(1.0)`-is-valid steady-state sentence to the HdrStatus design.

### 15. Wayland has no color management in wgpu 30; Linux HDR is Vulkan-extension-rare — minor
- **Raised by:** 03 (wayland-no-cm)
- **Finding:** No wp_color-management/frog/xx code exists anywhere in wgpu-hal/core 30; the
  Linux story is `VK_EXT_swapchain_colorspace` (requested unconditionally) feeding surface
  caps. `EXTENDED_SRGB` is essentially driver-absent on desktop Linux [INFERENCE from
  driver ecosystem], so Linux is probe-gated SDR in practice under PR 1. The
  probe-succeeds-but-wrong-output footgun lives in PR 2's ExtendedSrgbLinear-on-Wayland
  case (compositor-dependent) — the experimental label belongs *there*.
- **Suggested decision:** Rewrite the §7.3 Linux bullet accordingly; move "experimental"
  scRGB-on-Wayland caveats to the PR-2 section.

---

## C. Color contract (§2, §7.1)

### 16. "The one combination that is shader-correct" is false — two exist; argue continuity, not correctness — major
- **Raised by:** 01 (one-correct-combination-overclaim), 02 (blending-premise-circular),
  02 (linearize-shader-not-precise)
- **Finding:** ExtendedSrgbLinear + the existing `fs_main_linear_framebuffer` is equally
  shader-correct today (PR #8703 demonstrates it in the wild, "UI looks the same"); §7.1's
  rejection of linear ("SDR UI washout") conflates linear *content* on a gamma surface
  with gamma input *linearized by the correct shader*. The plan's gamma choice is still the
  right pitch — but on grounds it never states: (a) bit-exact status-quo parity for SDR
  chrome (gamma-blend grammar unchanged — egui "prefers gamma color spaces for all
  blending", egui/src/lib.rs:147; emilk's gamma-filtering lineage #7311/#2071), (b) one
  texture grammar for u8 and float content, (c) web: `ExtendedSrgbLinear` is **native-only**
  — ExtendedSrgb is the only HDR choice that also covers web. Honest cost to state: HDR
  alpha compositing and filtering happen in gamma permanently (50%-alpha white over black →
  linear 0.214, not 0.25). Related overstatement: `fs_main_linear_framebuffer` is the
  correct *existing* entry point, "precisely correct" oversells it (dither placement before
  linearization; negative handling wrong — entry 18).
- **Suggested decision:** Rewrite §1's "why a policy enum" paragraph and §7.1 point 2 on
  continuity + web grounds; drop "one combination"; add the gamma-compositing residual
  cost explicitly.

### 17. §7.1 rationale contains three quantitative/factual errors — major
- **Raised by:** 02 (gamma-multiply-5pct, fp16-precision-not-a-wash)
- **Finding:** (a) "Gamma-space multiply skews luminance ~5.5% per multiply" does not
  reproduce: measured +37.7% luminance error at dark factors (enc 0.35), +1.5% at mid-gray,
  exactly zero in the linear toe; the dominant term is the −0.055 intercept, not the 1.055
  slope, and "power laws compose correctly for large values" has the direction backwards
  (pure power laws compose exactly; affine terms break it everywhere). (b) "Precision is a
  wash" is false as stated: linear fp16 storage is ~2.4× finer in relative *luminance*
  terms (the 2.4 derivative factor was ignored) and avoids f16 subnormals below L≈4.7e-6;
  both effects are far below JND, so the *decision* still rests on grammar — but the
  rationale is half-wrong. Upstream reviewers with color-science background will find both
  in minutes.
- **Suggested decision:** Replace the multiply sentence with the measured shape (or drop
  quantification and argue bit-parity with today's `in.color × tex` path, which is the
  argument that holds). Restate precision as "linear is ~2.4× finer; both ≪ JND; grammar
  decides".

### 18. "Odd-symmetric for negatives" is unimplemented in the shaders; CPU and GPU would diverge 12.9× — major
- **Raised by:** 02 (neg-contract-mismatch), 06 (wgsl-negative-nan)
- **Finding:** ecolor's Rust fns are odd-symmetric (`-f(-x)`), but the WGSL twins have no
  negative branch: `cutoff = rgb < 0.0031308` routes all negatives to the linear branch
  (`gamma_from_linear_rgb(-0.5)` → −6.46 vs odd-symmetric −0.735). Both self-invert
  individually, but the CPU picker and GPU decode disagree 12.9× for negative float
  texels/vertex colors — exactly what tiers C/E introduce. Additionally WGSL
  `pow(negative, …)` is spec-undefined (typically NaN); it's currently suppressed by
  `select` and one refactor away from propagating.
- **Evidence:** egui.wgsl:44-56; ecolor lib.rs:157-178; WGSL spec.
- **Suggested decision:** Pick one negative extension and write it into the contract:
  linear extension (matches WGSL; then fix or document ecolor's divergence) or
  odd-symmetric (then fix the WGSL to `sign(x)·f(|x|)`). Add sign-mirroring before `pow`
  regardless, plus a CPU↔GPU round-trip test with negative inputs in the tier-C PR.

### 19. Contract under-specification: premultiply semantics, gamma filtering, picker claim — major
- **Raised by:** 02 (premultiply-under-specified, gamma-filtering-claims)
- **Finding:** (a) "Premultiplied (matching ColorImage storage)" is ambiguous for extended
  colors: premultiply the *gamma-encoded* components (`a × encoded`), or premultiply linear
  then encode (`enc(a·L)`)? They diverge for every semi-transparent HDR texel, and only
  encoded-space matches the fixed-function `One/OneMinusSrcAlpha` blend in a gamma
  framebuffer — the WGSL multiply silently assumes one. (b) Hardware bilinear on float
  textures filters *stored gamma* values (highlight-energy shift, dark halos) — an unstated
  contract property that constrains the future linear-surface variant. (c) "Gradient
  interpolation in gamma = perceptual, what pickers want" overclaims: gamma-RGB drifts hue
  for saturated colors; pickers needing hue-stable paths interpolate in their own space.
- **Suggested decision:** Add two bullets to §7.1: "premultiplication applies to
  gamma-encoded components (matching Color32)" (with a contract test: a=0.5, L=4 swatch);
  "filtering and blending occur in gamma (stored space)". Soften the picker sentence.

### 20. Dithering on float targets: "meaningless" is wrong — it is harmful, default-ON, and already a user-facing knob — major
- **Raised by:** 02 (sdr-by-construction-gotchas), 05 (pr2-dithering-precedent)
- **Finding:** Dithering defaults to **true** (`RendererOptions::default`) and the field's
  own doc admits it "assumes an sRGB output". On fp16 there is no 8-bit quantizer to hide
  in; near black the ±1/255 encoded noise maps to ×12.92 relative linear noise — shadow
  grain every eframe app gets the moment tier B lands. §2's "meaningless" mischaracterizes
  it. Additionally dithering is a public three-layer knob (`NativeOptions::dithering`,
  `RendererOptions::dithering`, shader uniform) — PR 2's auto-suppression must define
  precedence or it silently overrides a user option.
- **Suggested decision:** Reword §2: "actively harmful in shadows on float targets"; PR 2
  flips the effective default for float targets and states precedence ("forced off on
  float targets regardless of the knob; doc updated to say it assumes an 8-bit sRGB
  target"). Add the compositor-space question (EDR layer blending) to §7.8 explicitly.

### 21. PQ ingest convention: 100 nits is outdated; BT.2408 says 203 — minor
- **Raised by:** 02 (pq-100-nits-convention)
- **Finding:** §7.5's "nominal 100 nits is the common convention" is stale: industry moved
  to 203 nits (ITU-R BT.2408, maintained through 2024; Apple's EDR mapping follows the
  203-nit anchor). Mapping at 100 renders SDR-anchored PQ content ~2× too bright.
- **Suggested decision:** Default to 203 nits (BT.2408), documented; use Windows
  `sdr_white_nits` exactly where available (§7.5 already passes through `DisplayHdrInfo`).

---

## D. Reporting & API design (§7.4, §2)

### 22. HdrStatus reporting has no refresh path; the `frame.info()` mirror is cross-crate, redundant, and wrong for headroom — critical
- **Raised by:** 04 (hdr-refresh-nonexistent), 05 (frame-info-mirage), 03
  (headroom-is-per-viewport)
- **Finding:** Three related defects. (a) §7.4's "refreshed at reconfigure/recreate (the
  same points `configure_surface` runs)" does not exist: `configure_surface` runs on every
  resize but re-probes nothing — `RenderState::create` runs exactly once, guarded by
  `if self.render_state.is_none()` (winit.rs:245-249), and the app-visible RenderState is a
  one-shot clone (`wgpu_render_state()`, which *already exists* at epi.rs:802-806). Either
  HdrStatus is create-time-immutable (then "refreshed" is false and the mirror is
  pointless) or the plan needs a Painter→eframe publish path it never sketches. (b) The
  mirror lands in eframe's `IntegrationInfo` — a different crate — contradicting tier B's
  "egui-wgpu only" and the ~150-line budget; it would be a *third* copy (Painter's, Frame's
  clone, the mirror) with three staleness stories. (c) Per §7.4's own split, headroom is
  per-*surface*, but `Frame::info()` returns one global struct — wrong for the exact
  multi-viewport case egui supports (SDR + HDR monitors); the natural per-viewport home is
  `ViewportInfo`, and the DisplayHdrInfo pass-through is a cross-crate design decision (wgpu
  types can't be mirrored into `egui`), not a "side item".
- **Suggested decision:** Cut the mirror from PR 1. HdrStatus = `RenderState` field only,
  documented create-time-immutable; apps read it via the existing
  `frame.wgpu_render_state()`. Move DisplayHdrInfo/headroom to its own later PR with the
  per-viewport (`ViewportInfo`) home decided.

### 23. `HdrStatus` payload is blank and §1 contradicts §7.4 (bool vs enum) — critical
- **Raised by:** 04 (hdr-status-payload)
- **Finding:** §7.4 writes `Hdr { .. }` — the payload is unspecified, so PR 1 literally
  cannot be written from the plan. Worse, §1 says `RenderState` records `hdr_active: bool`
  while §7.4 (later, unmarked as superseding) says "an enum, not a bool" — two fields for
  the same concept.
- **Suggested decision:** Delete the §1 `hdr_active` sentence. Pin the payload, e.g.
  `Hdr { format: TextureFormat, color_space: SurfaceColorSpace }` (mirrors `decide_target`'s
  outputs — bug reports become self-describing) and `Sdr { reason: SdrReason }` with a
  closed `SdrReason` enum (at least `Disabled`, `NoExtendedSrgb`, `NoSurface`). Derive
  `Clone, Copy, Debug, PartialEq, Eq`.

### 24. `HdrPreference` names a mechanism, misses the variants users will ask for, and has a derive constraint — major
- **Raised by:** 04 (hdrpreference-variants), 01 (hdrstatus-evolution-promise)
- **Finding:** `PreferExtendedSrgb` contradicts the plan's own policy-not-knobs argument;
  egui's convention is intent-space (`ThemePreference::Light/Dark/System`). An `Auto`
  variant is the first review demand (default-`Off` means the ecosystem stays SDR forever).
  Apps that *require* HDR (video scopes) get only post-hoc detection — answer the
  fail-hard question explicitly (a `Required`-style variant *or* a documented
  "check `hdr_status` after create" recipe). Mechanical constraint: `SurfaceConfig` derives
  `Clone, Copy, PartialEq, Eq, Hash`, so `HdrPreference` must stay unit-variant-only or the
  struct loses derives. Also don't oversell stability: future policies extend the enum
  (consider `#[non_exhaustive]`, matching egui-wgpu house style); drop §7.4's "public
  signatures survive unchanged" absolutism.
- **Suggested decision:** `HdrPreference::{ Off, Auto, ExtendedSrgb }` (unit variants),
  document `Auto` as the likely future default, answer Required-or-recipe in the issue, one
  sentence on future extension.

### 25. The new `preferred_framebuffer_format` contract is under-defined: Err domain, kept name, tuple return — major
- **Raised by:** 04 (format-fn-contract)
- **Finding:** The `caps`-typed parameter is well-motivated — wgpu 30 puts HDR-only formats
  exclusively in `SurfaceCapabilities::format_capabilities`, *not* `formats` (a
  `&[TextureFormat]` slice cannot even see `Rgba16Float` there). But: (a) the Err case is
  never stated (does it ever fail under `Off`? under `ExtendedSrgb`?); (b) keeping the name
  while returning `(TextureFormat, HdrStatus)` fails the plan's own anti-drift test — the
  name says "format", the function decides a surface target; (c) §7.4's PR-2 promise
  ("public signatures survive unchanged") is softer than it reads (entry 24).
- **Suggested decision:** One public function, new name:
  `decide_surface_target(caps, hdr) -> Result<SurfaceTarget, WgpuError>` with
  `SurfaceTarget { format, color_space, status }` (named struct, not 3-tuple); state the
  Err condition in one sentence (likely `NoSurfaceFormatsAvailable` only).

### 26. `preferred_color_space` symmetry is cosmetic and its totality hides an error — major
- **Raised by:** 04 (colorspace-projection)
- **Finding:** Color-space support is per-*format* in wgpu 30 (`SurfaceFormatCapabilities`
  pairs) — there is no well-defined "the surface's color space" independent of the chosen
  format, so the projection either re-derives a format internally (duplicating the picker)
  or assumes one. And the claimed symmetry is broken anyway: the sibling returns `Err` on
  empty formats, this returns `Auto`. A caller pairing its output with their own format
  choice can build `(format_without_extended_srgb, ExtendedSrgb)` and only wgpu validation
  catches it.
- **Suggested decision:** Make it private (or delete): `configure_surface` reads the stored
  `color_space` field (already mandated); hand-rolled users call the single
  `decide_surface_target`.

### 27. Storing `OutputEncoding` re-introduces the incoherence §7.4 claims to eliminate — major
- **Raised by:** 04 (output-encoding-stored)
- **Finding:** Encoding is a total function of `(target_format, target_color_space)`;
  storing a derivable value next to its inputs is the drift-on-discount `decide_target` was
  built to kill. Naming mixes categories (`Gamma` noun / `Linearize` verb) where peers use
  adjectives; placement is unstated — the natural public spot is `RendererOptions`, which
  standalone `Renderer` users construct by hand, so its default needs a defined derivation.
- **Suggested decision:** Either compute `OutputEncoding::of(format, color_space)` at
  `Renderer::new` (zero stored state) or make it the fourth `decide_target` output covered
  by the invariant test. Rename variants `{ GammaEncoded, Linear }`; add to
  `RendererOptions` with a documented default.

---

## E. Tier C / D / E data model (§5, §6, §7.1, §7.6)

### 28. Tier C is not "purely additive": Eq cascade, variant shape, helper surface — major
- **Raised by:** 04 (imagedata-eq-cascade), 05 (tier-c-additive-caveat), 06
  (tier-c-eq-serde)
- **Finding:** `ImageData`, `ImageDelta`, `TexturesDelta` all derive `Eq` (+ serde); any
  float pixel type kills `Eq` through all three — a visible breaking change to name in the
  changelog, not discover by CI. The variant needs a proper shape the plan never gives:
  `Float(Arc<FloatImage>)` with `FloatImage { size, pixels: Vec<[f32;4]> }` (a bare pixel
  vec can't implement `ImageData::size()`), plus the ColorImage-parity helpers (`new`,
  `filled`, `from_*`, `FloatImage::from_color_image` lossy) and the `From` impls
  `ImageDelta::full/partial` require. `ImageData` is also a closed serde-derived enum —
  exhaustive matches downstream break; serde roundtrip tests (NaN/negative/>1.0) are
  required.
- **Suggested decision:** Mark tier C "additive + one breaking enum variant (Eq removed,
  serde note)". Spec `Float(Arc<FloatImage>)` + helper set in the plan.

### 29. RAM format: `half` is already in the tree; decide f16-now-vs-f32-v1 explicitly — minor
- **Raised by:** 04 (float-ram-half)
- **Finding:** The "no `half` dep" rationale is weak: `half 2.7.1` is already resolved in
  Cargo.lock (via wgpu/naga), so epaint gains a direct dep but no new vendored code, and
  its `serde`/`bytemuck` features are what tier E needs anyway. RAM-f16 makes storage 1:1
  with the upload and halves the 132 MB a 4K frame costs at f32 (16 B/px vs ColorImage's
  4 B/px — a multiplier worth stating to reviewers). Also affirmatively state
  `TextureOptions` needs *no* changes (fp16 is filterable; existing filter/wrap/mipmap
  knobs suffice) — only the Rgba8Unorm-only *docs* on `register_native_texture` and the
  hardcoded texture creation change.
- **Suggested decision:** Decide now: `[f16;4]` in RAM behind the existing `half` dep, or
  f32-for-v1 with the issue sentence "RAM format is deliberately f32; fp16 is an upload
  detail" so it isn't relitigated in review.

### 30. Tier D picker sketch collides with existing machinery: HSV clamps, missing Alpha param, name collision, premultiplied state — major
- **Raised by:** 04 (hdr-picker-hsva)
- **Finding:** (a) Every `color_edit_button_*` round-trips through `Hsva`, which is
  documented 0-1 and clamps saturation; on extended/negative input hue/sat are garbage —
  tier D needs a new edit model (extended RGB + stops sliders, no HSV) or a defined
  extended-Hsva. (b) The sketch drops the `Alpha` parameter every sibling carries. (c)
  `color_edit_button_rgba(&mut Rgba, Alpha)` already exists — two public functions over the
  identical state type with different range semantics is a type-system failure; upstream
  will ask "why not extend the existing one?" and the plan has no answer. (d) `Rgba` is
  documented premultiplied 0-1 linear; slider-authoring over premultiplied state misbehaves
  when alpha < 1 — existing widgets un-premultiply for editing; the plan's edit state
  (premultiplied vs straight) is undecided and silently rewrites `Rgba`'s documented
  contract.
- **Suggested decision:** Spec tier D as: `color_edit_button_hdr(ui, rgba: &mut Rgba,
  alpha: Alpha)`, straight (un-premultiplied) slider state converted at the paint boundary,
  RGB+stops sliders only (explicitly no HSV in v1), plus a paragraph on coexistence with
  `color_edit_button_rgba`.

### 31. Tier E blast radius is mispriced in both directions; type-level ripple underweighted — major
- **Raised by:** 04 (vertex-f16-ripple), 05 (phantom-software-renderer)
- **Finding:** §5's consumer list names a software renderer that doesn't exist in the repo
  and omits the glow painter that does (cross-ref entry 2), plus `Context::tessellate`
  consumers. Type ripple not priced: `Vertex` derives `PartialEq, Eq` + serde +
  `bytemuck::Pod/Zeroable` across *two* cfg variants (including `unity` with reordered
  fields — the §7.6 packing trick must work in both field orders); f16 color drops `Eq`
  through `Vertex → Mesh → Shape`; serde representation of the color field is undecided
  (Color32 serializes as `#rrggbb` string; what does f16×4 become?); `[f16;4]` needs half's
  `bytemuck` feature.
- **Suggested decision:** Rewrite the tier-E consumer list from `git grep` at the target
  ref: {egui-wgpu pipelines, egui_glow painter or explicit scoping-out, kittest, capture,
  mesh serde, tessellate consumers}. Add the three type decisions (Eq removal, serde repr,
  half features) to §7.6 so the future PR doesn't reopen them cold.

### 32. §6's D-limitation claims overreach: text story and rotated line are falsifiable — minor
- **Raised by:** 05 (text-at-d-story, rotated-quad-claim)
- **Finding:** (a) "No clean D-tier story for text at all" overstates: a galley-keyed
  float-texture label cache (atlas coverage × HDR color, cached per string/color/DPI, ~16
  KB fp16 for a readout label) is a bounded, additive D mechanism — §6 even concedes "static
  HDR label → baked text … works" two paragraphs earlier. (b) "You can't even draw a
  rotated solid HDR line" is wrong: `Image::rotate` exists today (image.rs:243-247,
  867-880); at tier C that's a rotated HDR bar (unantialiased). The strategic conclusion
  (defer E) survives; the overclaims are exactly what an E-advocate uses to discount it.
- **Suggested decision:** Restate: "no *antialiased* rotated geometry, no joins/caps, no
  polylines; text has no *general* mechanism — per-widget texture caching is viable for
  short labels." If the demo app needs the nits readout, prototype the D-tier `hdr_text`
  helper: it either validates the deferral or becomes the evidence E is genuinely needed.

### 33. "~all realistic apps at C+D" needs its two missing qualifiers — major
- **Raised by:** 05 (coverage-claim)
- **Finding:** Stress-testing: (a) wide-gamut photo editor (P3 export) — §7.2 concedes
  saturated >1.0 colors clip, so the editor can't display or proof what it exports; the
  ceiling is gamut, not float range, and §6 carries none of §7.2's caveat; (b) node
  compositor — needs E by §6's own list (scopes, curves, annotations), contradicting the
  coverage sentence; (c) HDR video player — covered at C only with unadmitted costs:
  app-side bt.2020→sRGB-ext conversion + full-frame float upload per frame (~33 MB convert,
  ~16 MB fp16 upload at 24-60 fps for 4K). Medical imaging and game-editor viewports do
  hold. Honest statement: "all apps whose HDR content is rectangular and inside the
  extended-sRGB gamut."
- **Suggested decision:** Rewrite the coverage sentence with both qualifiers; move the
  gamut caveat into the upstream issue's non-goals with the video-bandwidth note —
  maintainers will ask about bt.2020 content immediately.

---

## F. Testing & CI (§7.7, §7.8)

### 34. No fp16 test/capture mechanism exists anywhere: kittest readback is u8, the headless probe no-ops, capture fails silently — critical
- **Raised by:** 06 (kittest-fp16-readback, capture-silent-noop), 03 (headless-fp16-no-path,
  capture-silent-empty), 05 (kittest-cant-do-hdr), 01 (capture-breakage-underweighted)
- **Finding:** §7.7's core claim ("kittest/wgpu-offscreen can regression-test encoding")
  is false as written. (a) kittest's offscreen *render* target is `target_format`, but
  `texture_to_image` hardcodes 4 B/px → `image::RgbaImage`; on Rgba16Float the
  bytes-per-row is half the required size → wgpu copy validation error → panic. No fp16
  readback exists in egui_kittest or kittest 0.4.0. (b) kittest constructs no compatible
  surface, so `RenderState::create` synthesizes `vec![Rgba8Unorm]` — after PR 1 the CI
  probe always falls back to SDR; neither PR 2's encoding/dither behavior nor PR 3's float
  textures is exercisable in snapshot tests at all, and no tier owns the fix. (c) §4's
  capture claim is also wrong in direction: `CaptureState::read_screen_rgba` matches only
  Rgba8/Bgra8 and otherwise logs an error and returns — the screenshot is silently never
  delivered (not "a lie"). (d) Upstream reviewers will demand the readback fix in or beside
  PR 1 regardless (#8703's author needed +105 lines of capture.rs for exactly this).
- **Evidence:** egui_kittest texture_to_image.rs; wgpu.rs:60-74; egui-wgpu lib.rs:272-280;
  capture.rs:196-204; PR #8703 capture.rs diff.
- **Suggested decision:** Make fp16 readback + a headless format override explicit PR-1/2
  deliverables with an owner: (1) `texture_to_float_image` (or plain wgpu tests in
  egui-wgpu with a hand-built fp16 RenderState — no kittest diff machinery needed); (2)
  honor an explicit format/test override so the no-surface path can force fp16; (3) fp16
  branch in `read_screen_rgba` plus a loud error/Result instead of early return. Reuse and
  credit #8703's capture approach.

### 35. Encoding-contract tests must be numeric float-readback assertions; u8 goldens are structurally blind; decide_target tests need synthetic `format_capabilities` fixtures — major
- **Raised by:** 06 (contract-test-underspecified, u8-golden-blind,
  decide-target-tautology, tier-test-matrix, surface-path-ci-gap)
- **Finding:** (a) "Snapshot tests for the encoding contract" isn't a spec, and u8 goldens
  *cannot* catch the regressions that matter: 2.0 and 10.0 clamp to identical white, so a
  clamp-to-1.0 regression matches goldens perfectly. Real contract tests assert numbers off
  fp16 readback: known input → `gamma_from_linear` within f16 quantization; >1.0 survives
  upload→shader→blend (2.0 and near-f16-max); negative behavior per contract; dither
  disabled → bit-identical repeat renders; SDR [0,1] unchanged vs u8 path. (b) §7.4's
  decide_target invariant test is tautology-risk as a single canned case — and in wgpu 30
  HDR-capable formats live in `format_capabilities` (per-format color spaces), *not*
  `caps.formats`; fixtures built from `formats` can never exercise the probe. Needs
  property tests over synthetic caps (Metal-like, DXGI-like, float-without-EXTENDED_SRGB,
  empty). (c) The plan never states which tier-B behaviors are CI-validatable vs
  manual-only: the surface probe, configure threading, alpha×fp16, jitter-fix, and
  display_hdr_info are all manual-only; CI validates the pure decider + offscreen pipeline.
- **Suggested decision:** Add an "encoding contract test matrix" to §7.7 (input, expected,
  tolerance, runner) and a per-tier tests column to the §5 ladder; specify synthetic
  `format_capabilities` fixtures; add the explicit CI-vs-manual paragraph.

### 36. CI reality: every GPU test runs on exactly one OS/backend; "macOS tested" has no manual protocol — major
- **Raised by:** 06 (ci-single-os-backend, manual-protocol-missing, example-ci-liability)
- **Finding:** egui CI runs `cargo test --all-features` only on `macos-latest` (Metal/M1,
  no HDR display, no window); Linux/Windows/wasm are fmt/clippy/doc-only. So §7.3's
  "probe + SDR fallback path is exercised" is macOS-only truth, and kittest deliberately
  prefers CPU adapters (lavapipe) where available — developer machines and CI exercise
  different adapters, so fp16 tolerances must absorb that drift. §7.3's "macOS tested,
  others experimental" is anecdote without a written protocol (steps, deterministic
  patterns, expected HdrStatus/headroom readings per platform). The upstreamed example app
  is compile-only in CI — its value shows only on HDR hardware, so it rots invisibly; make
  it the tool of the manual protocol, snapshot-free.
- **Suggested decision:** Commit to MANUAL_TESTING.md in PR 1 (per-platform steps +
  expected readings + "CI cannot validate this"); add one §7.7 sentence mapping each test
  to its runner; ship the example snapshot-free with a deterministic pattern menu +
  HdrStatus readout.

### 37. §7.8 interaction list: add the missing items — major
- **Raised by:** 06 (s78-interactions-incomplete)
- **Finding:** The list is wgpu-surface-centric. Verified omissions: (a) the kittest
  readback path itself (entry 34); (b) glow backend (entry 2); (c) wasm must keep compiling
  with HDR types (kittest excludes BROWSER_WEBGPU, so web is compile-checked only); (d)
  multi-viewport × capture (`screen_capture_state` is one shared `Option` across viewports
  with per-window sizes — worth a test independent of HDR); (e) tier C/D data-model fallout
  (Eq/serde — entry 28); (f) dither-suppression interaction (entry 20).
- **Suggested decision:** Extend §7.8 with (a)-(f), each marked CI-testable or manual-only.

### 38. §7.8 items that are resolvable now, not open questions — minor
- **Raised by:** 03 (msaa-fp16-fine, jitter-fix-stale, transparency-no-conflict)
- **Finding:** Three "verify" items close on source evidence: (a) MSAA resolve on fp16 —
  granted on all realistic Metal hardware except `apple1x` (pre-Apple-silicon); egui's
  intermediate target just uses `target_format`; and it's CI-testable via kittest
  `with_render_options(msaa_samples: 4)`. (b) `present_with_transaction` — at 0.36.2 the
  jitter fix is a latency clamp (frame latency ≥2 during resize) + a Cargo feature; it
  never touches `color_space`, and reconfigure rebuilds from the stored decision. (c)
  Transparency — `validate_surface_configuration` checks color space and alpha mode in
  independent blocks; no cross-constraint anywhere (Metal reports `[Opaque, PostMultiplied]`);
  the only platform breakage is DX12's missing `ExtendedSrgb` (entry 10), independent of
  alpha.
- **Suggested decision:** Replace the three open items with the resolved statements +
  two macOS manual-QA checkboxes (apple1x caveat; jitter-fix+HDR drag-resize; compositor
  behavior of extended-range transparent windows).

---

## G. Sequencing & scope (§1 sizing, §5 ladder)

### 39. PR 1 as bundled in §5 is 2-3× its advertised size and crosses into eframe — major
- **Raised by:** 01 (pr1-scope-drift), 05 (pr1-budget)
- **Finding:** §1 pitches "~150 lines, lib.rs + winit.rs, no eframe changes", but §5's PR 1
  additionally bundles the full §7.4 API (breaking in-place signature replacement, new
  public fn, `decide_target` + invariant tests, `frame.info()` mirror — entry 22) which is
  cross-crate (epi) and review-hostile. Realistic surface for the bundle: 2-3× the budget,
  plus a public-breaking-API review on top. Cross-crate scope is exactly what slows egui
  reviews (#8703: single-crate, still no human reply after 12 days).
- **Suggested decision:** Split: PR 1a = egui-wgpu only (HdrPreference, probe,
  color-space threading, `HdrStatus` in RenderState) — the #8703-comparable unit; PR 1b =
  eframe reporting (if any after entry 22). Say "~250-350 lines incl. tests/docs" for 1a.

### 40. "Side items" is an unowned grab bag carrying hard dependencies — major
- **Raised by:** 05 (side-items-unowned), 01 (capture-breakage-underweighted)
- **Finding:** Of five side items: fp16 capture/kittest readback is a *hard prerequisite*
  for testing PR 3 (snapshots clip >1.0 through the Rgba8 path; entry 34) and arguably for
  PR 1's acceptance (opt-in feature silently breaks `ViewportCommand::Screenshot`);
  DisplayHdrInfo pass-through is a cross-crate design problem (entry 22), not an item; the
  example+docs are release-blocking for adoption claims. Unowned side items are how review
  threads die.
- **Suggested decision:** Re-sequence: capture/kittest fp16 → with PR 1/2 and before PR 3;
  DisplayHdrInfo → own PR with the mirror decision made; example+docs → inside PR 1 or 3.

---

## H. Walkthrough index

Numbered entries for the one-at-a-time pass, grouped by theme, criticals first inside
each group:

| # | Entry | Severity |
|---|---|---|
| 1 | PR #8703 occupies tier B (stale tracker premise) | critical |
| 2 | egui_glow exists; one-backend premise false | critical |
| 3 | `[patch]`-fork strategy riskiest; early draft PR instead | major |
| 4 | Anchors stale vs HEAD/upstream main | major |
| 5 | Breaking-change precedent wrong; deprecation policy unaddressed | major |
| 6 | Issue draft: lead with bug, anchor to #7484/#6863 | minor |
| 7 | Non-goals: glow/wasm rationale rewrites | minor |
| 8 | Missing `hdr-egui-port-plan.md` | minor |
| 9 | `Auto` → ExtendedSrgbLinear; §1 failure mode wrong | major |
| 10 | DX12: no ExtendedSrgb; Windows needs PR-2 scRGB | critical |
| 11 | `set_hdr()` recreate loses textures | major |
| 12 | Policy home: SurfaceConfig vs create-time-only | major |
| 13 | PR-1 blockers: probe policy, no-surface, Err semantics | major |
| 14 | §7.5 corrections (iOS, 4-step precedence, Gamut) | minor |
| 15 | Wayland reality | minor |
| 16 | "One shader-correct combination" false; argue continuity | major |
| 17 | §7.1 numeric errors (5.5%, precision wash) | major |
| 18 | Negative-value contract unimplemented in WGSL | major |
| 19 | Contract gaps: premultiply, filtering, picker claim | major |
| 20 | Dither harmful, default-ON, knob precedence | major |
| 21 | PQ ingest = 203 nits | minor |
| 22 | Reporting: no refresh path; frame.info() mirror wrong | critical |
| 23 | HdrStatus payload blank; §1/§7.4 contradiction | critical |
| 24 | HdrPreference naming/variants/derives | major |
| 25 | Format-fn contract: Err domain, name, named struct | major |
| 26 | preferred_color_space: make private/delete | major |
| 27 | OutputEncoding: derive, don't store | major |
| 28 | Tier C: Eq cascade + FloatImage shape + serde | major |
| 29 | RAM f32 vs half decision | minor |
| 30 | Tier D picker design collisions | major |
| 31 | Tier E blast radius mispriced | major |
| 32 | §6 overclaims (text, rotated line) | minor |
| 33 | Coverage claim needs gamut/rectangular qualifiers | major |
| 34 | No fp16 test/capture mechanism (kittest, probe, capture) | critical |
| 35 | Numeric contract tests; goldens blind; fixture design | major |
| 36 | CI single-OS; manual protocol missing | major |
| 37 | §7.8 missing interactions | major |
| 38 | §7.8 closable items (MSAA, jitter, transparency) | minor |
| 39 | PR 1 scope/budget drift; split 1a/1b | major |
| 40 | Side items unowned/missequenced | major |

**Verdict across reviewers (unedited, one per file):**
- 01: technical core is upstreamable, but it was drafted against a vacuum that no longer
  exists (#8703); correct glow before any issue text ships; endorse-or-evolve, not outrun.
- 02: §7.1's DECIDED rationale has three quantitative/factual errors upstream reviewers
  will find in minutes; ExtendedSrgb stays defensible via status-quo parity + web — the
  argument the plan inexplicably leaves out.
- 03: egui-side anchors verify solidly; the two credibility-breakers are platform-layer
  (DX12, glow); the substantive design fixes are Auto, set_hdr, and the headless test gap.
- 04: API skeleton is right; reporting rests on a nonexistent refresh point, payloads are
  blank, and the policy's home contradicts the create-time-only ruling.
- 05: invasiveness logic sound and B-first right, but two false premises and one internal
  contradiction would each derail PR 1; re-anchor to upstream main; demote the fork.
- 06: §7.7 is the weakest verified section — the named harness cannot read back fp16 and
  the shader violates the decided negative contract; recoverable cheaply with float-readback
  assertions + synthetic-caps property tests + an explicit CI-vs-manual split.

---

## Decision log

| # | Decision | Plan edit |
|---|---|---|
| 1 | **Accepted** (2026-10-25). Engage with PR #8703 instead of competing: review it, test on macOS EDR, position `HdrPreference`/`HdrStatus` as follow-up hardening PR. Verified PR facts directly (created 2026-10-12, `prefer_hdr` bool, fp16+`EXTENDED_SRGB_LINEAR` via `Auto`, linear fragment path, fp16 capture fix, Linux-tested/macOS+Windows-untested, BLOCKED, no maintainer reply). | Header tracker sentence re-verified/dated; new §7.9 strategy bullet (a)–(d). |
| 2 | **Accepted** (2026-10-25). Replace "egui_glow does not exist / one backend" with the capability argument (GLES reports `SRGB`-only, `gles/adapter.rs:1347` verified; EDR only via CAMetalLayer). Glow stays SDR by design. Downstream ripples deferred: tier E consumer list → entry 31, "glow removed" citation → entry 5. | Header sentence replaced; glow added to §5 non-goals. |
| 3 | **Rejected — user decision** (2026-10-25). We WILL use a patched fork: the app must verify the design end-to-end before any upstream PR is opened. Reviewer's churn concern (rebases vs upstream drift, no shipping until fork tracks a tag) recorded as accepted cost; mitigation = keep fork delta small and upstream-shaped. | §7.9 fork bullet replaced with the fork-first decision record. |
| 4 | **Accepted** (2026-10-25). Re-anchor workstream: Appendix anchors → HEAD before fork code; API surface → #8703's branch when composing; → upstream main only at PR-drafting. Verified at HEAD: `preferred_framebuffer_format` already `Result`-returning (lib.rs:421-423), `WgpuError` (:46), `with_surface_config` (:376). | New §7.9 "Re-anchor before implementing" bullet. |
| 5 | **Accepted** (2026-10-25). Fix §7.4 precedent citation (wgpu-30 bump + `Modifiers` #8336 / `clip_rect_margin` #8366 removals in 0.36.0; `RendererOptions` was 0.33 not 0.35→0.36; glow already corrected in entry 2) and engage the deprecation objection: old signature fails *silently* under HDR preference — deprecated-but-kept needs a doc-visible warning, replaced follows 0.36.0 precedent. Verified both citations at tag. | §7.4 rationale parenthetical replaced; deprecation sentence added. |
| 6 | **Accepted + extended — user decision** (2026-10-25). Issue-draft structure (hook order, #7484/#6863 anchors, objection pre-emption) kept as *prepared material*, but **all upstream engagement deferred**: no issue, no PR, no public comments — fix it on the fork first. Amends entry 1(a): #8703 review is local-only (fetch branch, test on macOS EDR); public comment held with everything else. Both anchors verified live (#7484 open ~14 mo, reporter-confirmed double gamma; #6863 open, `linear_from_gamma_rgb` body). | §7.9: (a) amended to local-only review; issue bullet converted to "All upstream engagement deferred" with prepared draft. |
| 7 | **Split decision** (2026-10-25). Web rationale rewrite accepted (non-goal reframed: deferred for egui-side scope; wgpu 30 web already reports `EXTENDED_SRGB|EXTENDED_DISPLAY_P3` — verified `webgpu.rs:4163`). Glow part was done in entry 2. **Gamut deferral REJECTED — user decision: wide gamut is in scope.** Mined #2712 live: emilk demand ask (2023), Wumpf's in-shader colorspace+OETF direction matching our §2, unspecified `Rgba` primaries ("bug that needs fixing"), CAMetalLayer/ColorSync mechanism, glow's too-bright P3 bug + EGL lead. §7.2 rewritten as staged workstream (v1 ext-sRGB → P3 policy variant via EXTENDED_DISPLAY_P3 on Metal/web → primaries-tagged colors). Entry 33's coverage rewrite will inherit in-scope gamut. | §5 non-goals web clause rewritten; §7.2 fully replaced; §7.1 issue-sentence marked v1-only. |
| 8 | **Accepted, option 1** (2026-10-25). Port plan located at `../hdr-test/hdr-egui-port-plan.md` (never committed here); relevant bits inlined into the plan header (app architecture, validated choices: Rgba16Float+ExtendedSrgb-not-Linear rationale, unclamped srgb_encode, dithering off, SDR fallback shape, no-runtime-toggle recreate rule); file reference removed. | Header paragraph replaced with inline summary. |
| 9 | **Accepted** (2026-10-25). §1 failure mode corrected: `Auto`+fp16 resolves to `ExtendedSrgbLinear` (scRGB) not "SDR-interpreted" — status quo on macOS is accidental EDR with linear-interpreted gamma output; #7484 black screen is the no-fp16 case. Added named PR-1 invariant: configure_surface must set `color_space` explicitly (get_default_config seeds `Auto`), §7.4 test asserts built SurfaceConfiguration coherence. Verified `resolve_auto_color_space` fallbacks `[ExtendedSrgbLinear, Srgb]` at wgpu-core-30.0.1 surface_config.rs:24-38. | §1 third bullet rewritten; new "Invariant" paragraph after "The change". |
| 10 | **Accepted** (2026-10-25). §7.3 Windows bullet resolved against source: DX12 never reports/mapps `ExtendedSrgb` (`unreachable!()`), fp16 → `EXTENDED_SRGB_LINEAR` only — PR 1's probe fails on every Windows machine → deterministic SDR fallback; label corrected to "no HDR on Windows until PR 2 scRGB variant"; #8703 de-risks that combination; per-platform fallback question parked for PR 2. Verified at wgpu-hal dx12/adapter.rs:1348 + dx12/mod.rs:1455-1466. | §7.3 Windows bullet replaced. |
| 11 | **Accepted + transitions amendment** (2026-10-25). Runtime switching decided create-time-only; `Painter::set_hdr()` dropped. Rationale documented in plan: renderer-only recreate loses font atlas + user native textures (delta-only updates leave fresh Renderer empty → silent blank glyphs); Context recreate impossible as eframe API (Context held by value); half-measures fail silently — worse than #7484's visible failure. Display-transitions line corrected: fp16+ExtendedSrgb config is display-agnostic, SDR display → `tone_map_headroom() == Some(1.0)` (verified surface.rs:732-757), headroom poll is the whole transition story — no runtime switch ever needed for SDR↔HDR drags. User verified the SDR-display behavior question before deciding. | §4 runtime-switching bullet rewritten with full rationale; §4 display-transitions bullet replaced. |
| 12 | **Accepted** (2026-10-25). `HdrPreference` moves off `SurfaceConfig` (runtime-mutable contract; verified at HEAD lib.rs:67 + set_surface_config winit.rs:97 + eframe epi.rs:965) onto `WgpuConfiguration`, create-time-only. Runtime flips would silently no-op or half-apply into #7484 black screen. #8703 places `prefer_hdr` on SurfaceConfig — hardening PR relocates it. Direction recorded for entry 24: Auto variant as endgame default once capture/kittest fp16 infra exists; `Off` default is rollout safety; our fork opts in unconditionally (user: "why not always HDR" — SDR fallback is free display-side but capture/kittest/memory costs forbid always-on upstream today). | §1 "The change" paragraph replaced. |
| 13 | **Accepted, shrunken** (2026-10-25). User walkthrough clarified: items 1-2 are record-only (code already behaves: first-surface probe, inheritance, hard-fail on late mismatch — no macOS impact, Metal unconditional); item 3 = one `SdrReason::NoSurface` variant (defined with entry-23 types); item 4 = status-not-Err (Err stays `NoSurfaceFormatsAvailable` only). Open footnote deleted from §7.4. Verified create-once guard winit.rs:258 + no-surface vec![Rgba8Unorm] lib.rs:267 at HEAD. | §7.4 residual-policy bullet replaced with three recorded decisions. |
| 14 | **Accepted** (2026-10-25). §7.5 + Appendix corrections: (1) `DisplayHeadroom` macOS-only population — root-caused to an in-code TODO (`wgpu-hal metal/surface.rs:180-82` names `UIScreen.currentEDRHeadroom`), acknowledged upstream-wgpu gap not platform limit; plus macOS fills only headroom+coarse bit, and `high_dynamic_range` means "HDR active now" not "capable". (2) `tone_map_headroom()` = 4 steps (Apple → SDR `Some(1.0)` guard → nits ratio → None); `Some(1.0)`=no-headroom vs `None`=unknown distinction recorded for HdrStatus design. (3) `DisplayGamut` `#[non_exhaustive]` + unordered — pass-through only. All verified at wgpu-types 30.0.1. | §7.5 macOS bullet + two new consequence bullets; Appendix DisplayHeadroom + tone_map_headroom lines fixed. |
| 15 | **Accepted** (2026-10-25). §7.3 Linux/Wayland bullet resolved against source: no Wayland CM protocol code in wgpu 30; only lever is VK_EXT_swapchain_colorspace (unconditional, instance.rs:280-282); PR-1 policy = probe-gated SDR on Linux (EXTENDED_SRGB driver-rare [INFERENCE]); PR-2 scRGB path demonstrably works on KWin (#8703) and carries the wrong-brightness experimental label. Verified extension + CM absence. | §7.3 Linux bullet replaced. |
| 16 | **Accepted** (2026-10-25). "One shader-correct combination" retracted — two exist (ExtendedSrgb+gamma passthrough; ExtendedSrgbLinear+existing linearize entry, #8703). §1 rationale reframed on continuity: bit-exact SDR-chrome parity (egui/src/lib.rs:147 gamma-blend house style, verified), one texture grammar, web (ExtendedSrgbLinear native-only, surface.rs:219-220, verified); honest permanent cost stated (gamma-space HDR compositing/filtering, 0.214-not-0.25). §7.1 point 2 rewritten (washout dismissal was wrong-entry-point property); §2 shader claim downgraded to "correct existing entry point, modulo dither placement + negative handling". App choice unchanged. | §1 why-paragraph, §7.1 point 2, §2 shader sentence replaced. |
| 17 | **Accepted** (2026-10-25). §7.1 rationale numeric corrections; decision (gamma storage) unchanged. Point 4: "~5.5% per multiply" does not reproduce — measured +1.5% mid-gray / +0.1% near white / +38% at 0.1 / +672% at 0.01 (reproduced in Python; dominant term −0.055 intercept); acceptance argument = bit-identical to today's SDR `in.color × tex` path. Point 5: linear is ~2.4× finer in luminance terms (0.234% vs 0.098%) + f16 subnormal floor at L≈4.7e-6; both ≪ JND so grammar still decides. Notably reviewer 02's own "exactly zero in the toe" claim also failed reproduction (+1192% at 0.001) — corrected shape recorded instead. | §7.1 points 4 and 5 replaced. |
| 18 | **Accepted, option (a)** (2026-10-25). Negative-extension contract: odd-symmetric everywhere. ecolor already complies (verified lib.rs:171-178 `-f(-x)`); WGSL routes negatives to ×/÷12.92 branch (verified egui.wgsl:52-57; GPU encode(−0.5)=−6.46 vs CPU −0.735) and evaluates pow(neg) in a discarded lane. Tier C ships the `sign(x)·f(|x|)` WGSL fix before negative-capable content + CPU↔GPU negative round-trip test. Rejected (b) changing ecolor public behavior; (c) negatives-undefined (would close §7.2 scRGB-negatives wide-gamut path). Sources of negatives explained to user: image-processing ringing (tier C upload path), un-premultiply, scRGB out-of-gamut convention. | New resolution paragraph after §7.1 contract line. |
| 19 | **Accepted — all three by continuity principle** (2026-10-25). Premultiply = encoded-space (`stored = a × enc(L)`, 0.912-not-1.353 at a=0.5/L=4) — exactly Color32's convention, forced by fixed-function blend over stored values + PreMultiplied window layer. Filtering/blending = gamma stored-space (inherited; linear-variant caveat noted). Point-6 picker claim reworded to accurate version. **Standing principle set by user for remaining entries: when a question can be decided by "what does egui currently do", choose that — no revamp of working mechanics.** | §7.1 "Contract clarifications" block added; point-6 Bonus reworded. |
| 20 | **Accepted** (2026-10-25). Dither guidance corrected: on float targets force off (noise applied to encoded values, egui.wgsl:180 — near-black becomes ×12.92 relative-luminance grain; "meaningless" was wrong, it's harmful). Knob + ON-default untouched for 8-bit targets (continuity); doc note updates to "assumes an 8-bit sRGB target". Verified defaults ON at HEAD (renderer.rs:230, epi.rs:500) + uniform plumbing (:147/:293/:970). | §2 dither sentence replaced. |
| 21 | **Accepted** (2026-10-25, handled just before #20). §7.5 PQ-ingest crumb corrected: 203 nits (ITU-R BT.2408) replaces stale 100-nit convention; Windows path uses `sdr_white_nits` directly. Not continuity-decidable (egui has no PQ API); industry-convention fact-fix, app-level guidance only. | §7.5 last bullet replaced. |
| 22 | **Accepted** (2026-10-25, critical). Reporting mechanism corrected — three defects: (1) "refreshed at reconfigure" has no code path (`RenderState::create` runs once, winit.rs:258; app-visible RenderState is a one-shot clone) → HdrStatus documented create-time-immutable, matching entry-12 policy; (2) `frame.info()` mirror cut from PR 1 — cross-crate (eframe IntegrationInfo), third staleness copy; `Frame::wgpu_render_state()` already exists (verified epi.rs:943) and is the read path; (3) headroom is per-viewport → DisplayHdrInfo pass-through = own later PR targeting `ViewportInfo` (verified egui/src/data/input/viewport_info.rs:28), a cross-crate design decision not a side item. Also folds `NoSurface` into the reason list (entry 13). | §7.4 Reporting paragraph fully rewritten with three-defect correction block. |
| 23 | **Accepted** (2026-10-25, critical). HdrStatus payload pinned: `Hdr { format, color_space }` (mirrors decide_target outputs — bug reports self-describing) / `Sdr { reason: SdrReason }`, closed SdrReason enum (`Disabled`, `NoExtendedSrgb`, `NoSurface`); derives Clone, Copy, Debug, PartialEq, Eq. §1's contradicting `hdr_active: bool` replaced with `hdr_status: HdrStatus`. | §7.4 payload block inserted; §1 field reference fixed. |
| 24 | **Accepted** (2026-10-25). HdrPreference shape: `{ Off, Auto, ExtendedSrgb }` unit variants (Copy+Eq+Hash cheap); `ExtendedSrgb` names the target, `Auto` = probe-else-SDR with explicit "not wgpu's Auto" doc disclaimer; defaults per user direction (Off = upstream rollout, Auto = endgame once capture/kittest fp16 infra exists, fork opts in unconditionally); fail-hard = check-hdr_status recipe, no Required variant; no #[non_exhaustive] (egui-wgpu house style — verified zero enum attributes); §7.4 evolution bullet softened to "variant sets grow, signatures survive". | §1 sketch updated; §7.4 HdrPreference shape paragraph; evolution bullet reworded. |
| 25 | **Accepted** (2026-10-25). Format fn reshaped: `decide_surface_target(caps: &SurfaceCapabilities, hdr) -> Result<SurfaceTarget, WgpuError>`, `SurfaceTarget { format, color_space, status }` named struct; "keeps its name" retracted (anti-drift) — input type must change anyway (HDR formats live in format_capabilities with per-format color spaces, invisible to &[TextureFormat]); decider unifies with the public fn (projections gone); RenderState stores the three outputs; Err = NoSurfaceFormatsAvailable only; migration note for #8703 sibling (deprecate 1 release) vs in-place cut. Prefer token → ExtendedSrgb. | §7.4 format-selection bullets 1/3/4 replaced. |
| 26 | **Accepted** (2026-10-25). `preferred_color_space` projection deleted: color-space support is per-format in wgpu 30, so the projection must secretly re-pick a format and its totality masks errors; no consumer needs an unpaired color space (internals + hand-rolled users get the coherent SurfaceTarget, diagnostics read HdrStatus::Hdr). configure_surface reads the stored field — same mechanics as target_format today (continuity). | §7.4 projection bullet replaced with removal note. |
| 27 | **Accepted** (2026-10-25). OutputEncoding: derived at Renderer::new via `OutputEncoding::of(format, color_space)`, never stored (threading a derivable value = the drift decide_surface_target killed). Continuity: derivation already exists (is_srgb() sniff), just color-space-blind. Table: u8→GammaEncoded; fp16+ExtendedSrgb→GammaEncoded; fp16+ExtendedSrgbLinear→Linear. Consequence: PR 1 needs no encoding change; color-space input hits RendererOptions only in PR 2. Invariant test covers the table. | §2 selection block replaced. |
| 28 | **Accepted** (2026-10-25). Tier C repriced: "purely additive" → "additive + one breaking enum variant" — Eq drops off ImageData/ImageDelta/TexturesDelta (f32; Rgba PartialEq-only precedent, verified rgba.rs) + closed serde enum extends (changelog note). Shape pinned: `ImageData::Float(Arc<FloatImage>)`, `FloatImage { size, pixels: Vec<[f32;4]> }` (encoded-space premultiply), ColorImage-parity helpers + From impls for ImageDelta::full/partial, serde roundtrip tests (NaN/neg/>1.0). Verified derives at HEAD image.rs:14-16 + full/partial :530/:539. | §5 tier C row; §7.1 float-image bullet. |
| 29 | **Accepted, amended to f16-now** (2026-10-25). User challenged the delay ("if f16 is the end goal, why not now?") — f32-v1 rationale ("no half dep") was false: half 2.7.1 already in Cargo.lock:2206 via wgpu/naga. Decision: `Vec<[f16; 4]>` in RAM from tier C — 1:1 with Rgba16Float upload (zero-copy cast_slice, no conversion pass), half the memory (4K: 66 vs 133 MB; ~400 vs 800 MB per 50MP photo), zero precision loss (RAM quantization = upload quantization), pre-builds tier E's half dep surface (serde+bytemuck features). Helpers take f32 at edges (authoring ergonomics). epaint gains a direct half dep (changelog note); glow-only users newly compile it (small, no_std). | §7.1 float-image bullet amended; §5 tier C row notes direct half dep. |
| 29b | **Accepted — user-directed reorder** (2026-10-25). "PR 0 = tagging/groundwork; PR 1 = proper wide gamut end-to-end (Display P3); sRGB→P3 conversion only for apps that opt into the wider gamut." Verified feasibility: `EXTENDED_DISPLAY_P3` is extended-range (HDR brightness included) with same platform coverage as ExtendedSrgb (Metal + web; DX12 gets neither until scRGB). New ladder: **PR 0** = `Primaries` enum + `FloatImage.primaries` at birth + contract names primaries + WGSL negative fix (Srgb-only pairing, loud rejection); **PR 1** = tier-B surface with `{Off, Auto, ExtendedSrgb, ExtendedDisplayP3}` — P3 headline, opt-in chrome-shift documented, ExtendedSrgb = zero-shift compat default, callback example shows out-of-sRGB saturation; PR 2 = scRGB+encoding; C/D/E unchanged. Supersedes parts of entries 16 (web-coverage claim — P3 also covers web) and 24 (variant set), folds §7.2 stage-2 into PR 1. This pre-empts the #2712 "what primaries?" review flag on all NEW types. | §5 sequencing rewritten (PR 0-6); §7.2 re-staged; §1 sketch + entry-16 web sentence + §7.4 paragraph updated; §7.1 FloatImage gains primaries field. |
| 29c | **Accepted — user-directed split of PR 1** (2026-10-25). "PR 1 split in two: wide gamut only (no HDR) first; second half does the HDR." New ladder: **PR 1** = `DisplayP3` policy (P3-referred [0,1], gamma-encoded) on plain `Rgba8Unorm` — **no fp16**, so the capture/kittest fp16 cluster (entry 34) stays out of its critical path; tier-B machinery (preference enum renamed from HdrPreference → `SurfaceColorSpacePreference` TBD, decide_surface_target, configure threading, HdrStatus) debuts minus extended parts. **PR 2** = HDR: `ExtendedSrgb` + `ExtendedDisplayP3` on Rgba16Float (values >1.0, EDR), Auto finalizes, fp16 capture readback = its critical path. **PR 3** = scRGB/Windows + encoding. C/D/E renumbered 4/5/7. | §5 sequencing re-split; §1 sketch updated (rename + DisplayP3 variant + PR markers). |
| 29d | **Accepted — user correction** (2026-10-25). "Not just P3, or any one specific color space — the idea is primaries different from sRGB." Reframing applied: the *tag + pairing table* is the mechanism (content declares primaries; table decides display pairing; unpaired = loud reject); DisplayP3 = first instance to pair (platform exposure, not ideology); BT.2020 has no native gamma surface (wgpu BT.2020 = PQ/HLG transfer only) so it displays via conversion to a paired primaries; new primaries = new enum variant (cheap growth); color space = primaries × transfer, storage transfer fixed to gamma, PQ/HLG decodes app-side (§7.5). | §7.1 FloatImage primaries paragraph, §5 PR-1 item, §7.2 framing paragraph reframed primaries-generic. |
| 30 | **Accepted, decoupled chroma/exposure model** (2026-10-25, after HSV deep-dive). `color_edit_button_hdr(ui, rgba: &mut Rgba, alpha: Alpha)` — parameter parity. State = (chroma [0,1], exposure stops), painted chroma × 2^exposure (test-app grammar). Chroma via standard HSV field+strip (hue/sat only, V folds into exposure — 0-1 limitation dissolves since chroma is normalized); brightness via stops slider. Existing-value decomposition: exposure = log2(max_channel), chroma = color/2^exposure. Straight edit state (to_rgba_unmultiplied pattern). Distinct fn from color_edit_button_rgba (cross-referenced docs). OkLCh = follow-up when P3-native editing arrives. HSV excluded from brightness (no multiplicative axis) and negatives (emits only [0,v]) — model routes around both. Also: P3-native HSV viable later (user's correct challenge). | §7.1 tier D bullet fully replaced. |
| 31 | **Accepted** (2026-10-25). Tier E blast radius corrected: phantom software renderer deleted; `egui_glow` painter added (or explicit scoping-out per entry 2) + `Context::tessellate` consumers + Eq cascade (Vertex→Mesh→Shape, both cfg variants incl. unity — packing trick must work in both field orders; verified mesh.rs:8-60). §7.6 gains three pre-E type decisions: Eq drop, serde repr choice, half/bytemuck surface. | §5 tier E row; §7.6 type-decisions paragraph. |
| 32 | **Accepted** (2026-10-25). §6 D-limitation claims corrected: text = "no *general* mechanism" (galley-keyed float-texture cache viable per-widget, ~16 KB fp16 label; hdr_text helper = first prototype for the demo app's nits readout — validates deferral or evidences E); rotated HDR rects DO work at D (Image::rotate, image.rs:243-247/867-880) — real gap is antialiased rotated geometry/joins/caps/polylines. Precision protects the tier-E case. | §6 items 1 and 2 amended. |
| 34 | **Accepted — kittest fp16 support is a hard requirement** (2026-10-25, critical; user: "kittest must support fp16 so we can do proper testing when we introduce the HDR extension"). PR-2 named deliverables: (1) texture_to_float_image readback + clamp-to-white u8 path (~60-100 lines, 256-byte alignment, credit #8703); (2) headless format override (RenderState pub fields + from_render_state escape hatch exist; kittest builder surfaces it; no-surface stub honors override); (3) fp16 branch in read_screen_rgba + loud error/Result. PR 1 (DisplayP3/u8) unaffected. Verified texture_to_image.rs:81 u8 hardcode + lib.rs:267 stub at HEAD. Difficulty assessed for user: mechanical ~1-2 days, not research. | §7.7 rewritten with deliverables block. |
| 35 | **Accepted, convention-mapped** (2026-10-25; user directed: derive test plan from existing tests). Survey: ecolor 14 inline (blend/roundtrip precedents incl. test_color32_blend_vs_gamma_blend), epaint 96 inline, egui 102, kittest snapshots in egui_demo_lib/tests (color_picker.rs, image_blending.rs), egui-wgpu ZERO runtime tests (only wasm compile-asserts) — #8703 sets its first-test precedent. Matrix mapped: contract properties → ecolor inline; decide_surface_target property tests over synthetic format_capabilities fixtures → egui-wgpu unit; numeric encoding assertions → egui-wgpu w/ fp16 RenderState (runner/tolerances); visual → kittest snapshots (SDR-appearance only; goldens blind to >1.0); low-value tests named; CI-vs-manual split stated (GPU CI = macos-latest only, lavapipe drift). | §7.7 test-matrix block appended. |
| 33 | **Accepted, primaries-generic restatement** (2026-10-25; superseded reviewer's P3-specific qualifiers after user's correction). Coverage envelope: "all apps whose content is rectangular, within extended brightness, within the gamut of a *paired* primaries" (first instance Display P3); unpaired primaries (Bt2020-gamma) app-convert to a paired primaries (documented path); vector-shaped HDR → E; node compositors → E's list; video upload cost halved by f16 (entry 29). Envelope goes in the issue draft. | §5 "Reading the ladder" coverage bullet replaced. |
| 37 | **Accepted** (2026-10-25). §7.8 additions: (a) harness readback (→34, CI-testable); (b) egui_glow compile/SDR gate (→2); (c) wasm compile gate (kittest excludes BROWSER_WEBGPU, web job check-only — compile-only claims); (d) multi-viewport × capture (shared Option<CaptureState>, per-window sizes — test independent of HDR); (e) Eq/serde cascades (→28); (f) dither precedence (→20). | §7.8 missing-items block appended. |
| 38 | **Accepted** (2026-10-25). §7.8 items resolved on evidence: MSAA fp16 fine except apple1x (CI-testable via kittest msaa_samples:4); present_with_transaction = latency clamp, never touches color_space (manual checkbox: jitter+HDR+resize); transparency = independent validation blocks, no cross-constraint (manual checkbox: compositor + premultiplied >1.0); WgpuSetup::Existing = probe reads surface caps regardless of instance provenance; user instance lacking VK_EXT_swapchain_colorspace → correct SDR fallback [INFERENCE from verified mechanics]. | §7.8 first four items replaced with resolved statements + checkboxes. |
| 39 | **Accepted — closed by prior decisions + one honest number** (2026-10-25). frame.info() creep gone (entry 22 cut it); 1a/1b split happened as PR 0/1/2/3 (entries 29b/c). Remaining: stale "~150 lines" → §1 core ~150 lines; sequenced PR 1 realistically ~300-450 lines incl. tests/docs (policy + decide_surface_target + HdrStatus + pairing table + tests + MANUAL_TESTING + example); no shader/epaint/eframe changes; re-measure vs upstream main pre-drafting (entry 4). | §1 sizing sentence replaced. |
| 40 | **Accepted — closed by reassignment** (2026-10-25). Side-items grab bag dissolved: DisplayHdrInfo → own later PR w/ ViewportInfo home (entry 22); capture fp16 → PR 2 critical path (entry 34); docs + test app + MANUAL_TESTING.md → PR 1 (entry 36). | §5 item 6 rewritten with owners. |
| 29e | **Housekeeping** (2026-10-25, user-provided). Sibling checkout of upstream kittest at `/Users/alex/Developer/kittest` (rerun-io/kittest, **0.5.0** — matches workspace-resolved `kittest = "0.5.0"`, Cargo.toml:109; reviewers' "0.4.0" refs were tag-era). Labor split recorded: `crates/egui_kittest` (in-repo) owns renderer/readback (`texture_to_image.rs`); sibling owns Harness/builder/snapshot machinery — builder-side fp16 changes happen in the sibling (fork: path/patch dep). | Header "Sibling checkout" block; §7.7 deliverable 2 points at both locations. |
| R2-1/2/3 | **Accepted — full consistency pass** (2026-10-25). Round-2 entries 1-3 applied as one sweep: §7.2 sketch rewritten to the 29c ladder (PR 1 = DISPLAY_P3 SDR on Bgra8/Rgba8Unorm; PR 2 = extended pair; PR 3 = scRGB); §7.3 Windows/Linux bullets re-numbered (Windows: PR 1+2 both unavailable on DX12, HDR = PR 3); §7.4 paragraph renamed to SurfaceColorSpacePreference with the 5-variant set + split-PR markers + Auto open-question noted (round-2 entry 21); decide_target → decide_surface_target everywhere (payload line, §1 invariant, signature param renamed to `pref`); PR-2-evolution bullet → PR-3; §4 headroom/capture bullets aligned to entries 22/34; §7.5 frame.info survivor fixed; §7.7 smoke-vs-manual reconciled; §5 tier table B/C cells updated; §7.9 issue anchors remapped (PR 1→#2712, PR 2→#7484, PR 3→#6863); all 26 entry-citations verified current. | §2/§4/§5/§7.2/§7.3/§7.4/§7.5/§7.7/§7.9 swept. |
| R2-4 | **Accepted** (2026-10-25, residual after the consistency pass). §7.2 verified-facts block gains the load-bearing row: distinct **non-extended** `SurfaceColorSpace::DisplayP3` (sRGB transfer, P3 primaries, SDR clamp, is_hdr()==false) — Metal reports for every format incl. u8, web reports for every canvas (verified webgpu.rs:4154-4160), DX12 none; "this is what PR 1 pairs — no fp16 required". §5 web hedge "verified at implementation" dropped (already verified). | §7.2 verified-facts row added; §5 PR-1 hedge fixed. |
| R2-5 | **Accepted + P3-de-specialization directive** (2026-10-25, critical; user: "P3 is NOT some special case — one of many color spaces, cannot be treated as special"). (1) Pairing-mismatch row defined: surface pairs only its own primaries tag; mismatched uploads reject loudly at `update_texture` (§5 PR 1 + §7.2 sketch). (2) `SdrReason` generalized once: `NoExtendedSrgb` → `NoPairedColorSpace { requested: SurfaceColorSpace }` — never grows as pairings land. (3) OutputEncoding table gains u8+DisplayP3 and fp16+ExtendedDisplayP3 → GammaEncoded rows. (4) **Design rule added to §7.2 (binding):** P3 is one instance of a primaries-generic mechanism; nothing may name/branch on P3; adding a primaries = enum variant + pairing row + conversion matrix, no design changes. Decision paragraph + matrix phrasing de-specialized. | §5 PR-1 pairing semantics; §7.4 payload generalized; §2 table rows; §7.2 decision paragraph rewritten + design-rule block. |
| R2-6 | **Accepted — PR 0 dissolved, ladder restructured** (2026-10-25, critical; user directive: "each piece comes in as needed, nothing untestable or unused"). PR 0's pieces distributed to consumers: FloatImage/Primaries/ImageData::Float + upload pairing → **PR 2** (tier C, the consumer); WGSL sign fix + ecolor contract tests → **PR 2**; GPU negative round-trip → **PR 3** (with fp16 readback). **Bt2020 dropped from the v1 Primaries enum** — per the §7.2 design rule a primaries lands as variant+pairing row+conversion matrix together; BT.2020 arrives when its conversion path is demanded. New ladder: PR 1 surface (no epaint changes) → PR 2 tagged float content → PR 3 HDR extended + fp16 testing → PR 4 scRGB → PR 5 picker → side items → E deferred. Restructure rule recorded as binding. All PR-number cross-references renumbered (§1 sketch, §2, §4, §7.2, §7.3, §7.4, §7.7, §7.9, §5 tier C/D rows); stale 29c-era duplicate in the §7.2 sketch removed. | §5 sequencing rewritten + restructure rule; §7.2 sketch renumbered; §7.1/§2/§4/§7.3/§7.4/§7.7/§7.9 reference updates. |
| R2-7 | **Accepted — option 1 sharpened; do NOT flip** (2026-10-25, major; user challenged the flip). Reasons: (1) flipping to ExtendedSrgb-as-PR-1 re-imports the fp16 cluster into PR 1, undoing the 29c manageability split; (2) brightness demand is #8703's lane — if it merges, our wide-gamut pairing PR is the correct next contribution; competing head-on contradicts entry 1's evolve-don't-compete stance; (3) fork order is independent of upstream PR order. Fix = narrative: PR 1's issue framing is "follow-up to #8703's brightness work", #2712 anchor kept honest. Recorded the #8703-vs-this-plan diff table in §7.9 for the issue draft. Anchors verified consistent post-R2-6 (PR 1→#2712, PR 3→#7484, PR 4→#6863). | §7.9 (e) comparison block added. |
| R2-8 | **Accepted — option 2, total upstream silence stands** (2026-10-25, major; user: "We will not interact with that work"). #8703 collision cost explicitly accepted: if it merges first, our PR 1 becomes an evolution PR against freshly-merged API (field relocation off SurfaceConfig, encoding supersession for their adopters, capture.rs conflicts) — entry 25's "deprecate one release" note under-prices this and the cost is now knowingly carried. No watch/comment, no exception to entry 6. | Decision log only; entry 6 unchanged. |
| R2-9 | **Accepted** (2026-10-25, major). Entry-38 MSAA closure corrected to conditional: the offscreen harness hardcodes `sample_count: 1`/`resolve_target: None` (wgpu.rs:202/217) — MSAA plumbing added to entry-34's harness deliverables; until it lands the CI test cannot run. Platform linear-vs-gamma map walked with user (macOS/iOS/web = encoded-gamma family; Windows/KWin-Linux = linear-scRGB family; intermediate-linear render pass = documented cross-platform escape hatch). | §7.8 MSAA bullet gains harness prerequisite; §7.7 deliverables gains item 4. |
| R2-10 | **Accepted — corrects 29e** (2026-10-25, critical; reviewers 02+06 independently verified). The kittest labor split was inverted: rerun-io/kittest 0.5.0 is an accesskit-query-layer only (Harness/builder/snapshot/wgpu code never existed in that repo — verified via full git history); ALL testing machinery lives in-repo in `crates/egui_kittest`. Sibling fork unnecessary; header block + §7.7 deliverable-2 pointer corrected; sibling checkout demoted to reading reference. | Header "kittest note" corrected; §7.7 deliverable 2 relocated in-repo. |
| R2-11 | **Accepted** (2026-10-25, major). Entry-34 shrinks: (a) deliverable 2 mostly done — `builder().renderer(from_render_state(...))` already works (all pub); real work = egui_wgpu no-surface stub override + documented test recipe; (b) deliverable 1 relocated: float readback lands in egui-wgpu (next to capture.rs, shared #8703 decode — no dep edge egui_kittest→egui-wgpu and no float egress through RgbaImage-typed harness API); egui_kittest keeps clamp-to-u8 golden path only. | §7.7 deliverables 1-2 rewritten. |
| R2-12 | **Accepted** (2026-10-25, major). Entry-17's replacement mechanism claim ("−0.055 intercept dominates") refuted by decomposition: at 0.01 the driver is the encoded product crossing the decode knee; at ≤0.0031308 the composition is exactly 12.92·ab with no intercept active; zeroing the intercept in the power region makes the error 26× worse (+1.49%→+39.2%) — no single term dominates; the measured values reproduce (verified twice). §7.1 point 4 mechanism sentence rewritten as branch-crossing. | §7.1 point 4 mechanism sentence replaced. |
| R2-13 | **Accepted** (2026-10-25, major). Entry-30 decomposition hardened: canonical-band clause dropped (no-op under real-valued log2 — chroma max is always exactly 1.0); edges specified — black → exposure 0 identity, negatives via max(|c|) carried by chroma (odd-symmetric contract), alpha=0 safe because the edit state is straight (the to_rgba_unmultiplied black-at-zero hazard belongs to premultiplied conversions). Decompose→recompose verified bit-exact across extended range. | §7.1 tier-D decomposition sentence replaced. |
| R2-14 | **Accepted** (2026-10-25, major). Premultiply rule verified but cost now stated: encoded-space premultiply attenuates HDR translucency by a^2.4 (50% alpha on L=4 ≈ −2.4 stops vs photometric); enc(a·L) rejected (overshoots over non-black dst); fix = alpha-domain work in future linear-surface policy. Forcing rationale corrected ("forced by the pipeline" overclaimed — blend is convention-agnostic; real forcing = Color32 continuity + compositor PreMultiplied contract). | §7.1 premultiply bullet expanded. |
| R2-15 | **Accepted** (2026-10-25, major). PQ ingest chain specified in §7.5: PQ⁻¹ in BT.2020 linear light (never sRGB-inverse — 2.12× error) → 203-nit scale → linear-light primaries matrix (emits negatives — entry-18 contract covers) → paired-primaries gamma encode (no double-encode). HLG out of scope until a transfer-carrying tag exists. Hazards were never written down before this. | §7.5 PQ-ingest-chain block appended. |
| R2-16 | **Accepted** (2026-10-25, minor). f16 numeric labels fixed: max normal rel error = 2⁻¹¹ (±2⁻¹⁰ is full-ULP); entry-35 tolerance gains subnormal carve-out (flat relative tolerance flakes at linear 1e-6, rel err 1.1e-3); §7.1 point-5 subnormal labels corrected — 4.7e-6 is *gamma's* subnormal onset, linear f16's floor is ~4.6e-9 (0.098% step): gamma goes subnormal 12.9× earlier (strengthens the entry's conclusion). | §7.1 point-5 floor sentence corrected; entry-35 tolerance line fixed. |
| R2-17…22 | **Accepted as one batch** (2026-10-25; user: "accept any other similar small findings"). R2-17: "bit-exact/bit-identical" → "formula-identical" in §1/(§7.1 pt 2)/(pt 4), with the nuance — bit-exact on u8 targets, ≤½-code drift on fp16 (no store-time quantization), extended values have no baseline. R2-18: §7.3 gains the DX12 `Rgb10a2Unorm`/`BT2100_PQ` HDR10 nuance (out of v1 transfer contract; "no *extended-sRGB-family* HDR" is the accurate phrasing). R2-19: timing updated — 0.37 shipped 2026-10-15, ladder lands 0.38+; #8714 privatized `ColorImage`/renamed size→resolution → FloatImage sketch marked shape-stale, constructor-based API re-verified at PR 2 start. R2-20: per-texture content conversion added to the demand-gated list (chrome-shift ΔE numbers already recorded). R2-21: zero-shift scope noted (Off/ExtendedSrgb only; DisplayP3 pairing is opt-in shift); Auto-vs-P3 question already flagged open. R2-22: dust — wgpu lock 30.0.0 noted (30.0.1 diffs identical), ExtendedDisplayP3 Metal OS-version TODO noted, invariant fixtures gain the `(Bgra8UnormSrgb, DisplayP3)` double-grammar pair, #7484 = partial-recreate case (not "no-fp16"). | 13 edits across header, §1, §2, §5, §7.1, §7.3, §7.4, §7.9, Appendix. |
