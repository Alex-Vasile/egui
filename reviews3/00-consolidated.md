# Consolidated round-3 review — hdr-egui-upstream.md (final form)

Third independent review round, dispatched 2026-10-26 against the rewritten final plan
(541 lines). Six fresh personas, cold read: first implementer (01), upstream maintainer
gate (02), color-science/standards (03), HDR app consumer (04), production
failure-modes (05), fresh-eyes consistency + prior-audit closure (06). Raw reviews:
`reviews3/01…06-*.md`.

**Result: 30 consolidated entries (5 critical, 21 major, 4 minor/question).** Three of
the five criticals were found independently by two or more reviewers. The final-form
rewrite introduced two regressions of its own (R3-1, R3-4). All quantitative claims
were independently recomputed this round; four failed reproduction.

## A. Blockers (critical)

### R3-1. `Auto` is defined, declared open, and renamed — three states of one decision (raised by 06-critical, 01-major, 02-major)
- §2 defines `Auto` as "…else `DisplayP3` where supported" while Open question 2
  declares exactly that choice "deliberately open"; §2 also calls headroom "the only
  undecided decision" while Q2/Q3 sit open. Separately: `Auto` collides with wgpu's
  same-named variant with *opposite* semantics in the same re-exported namespace
  (02: rename candidate `BestAvailable`); and PR 1's `Auto` leg is a placeholder until
  PR 3 — the extended leg cannot be reached (01).
- **Resolution:** pick one home for the Auto/DisplayP3 decision; rename to avoid the
  wgpu collision; state PR-1 Auto behavior as "SDR-best until PR 3" explicitly.
- **Severity:** critical.

### R3-2. `HdrStatus` cannot represent PR 1's own success case (06-major, 01-critical — independent)
- u8 + DisplayP3 is `is_hdr() == false`, but the enum is `Hdr{format, color_space}` |
  `Sdr{reason}` with no fitting reason — a granted wide-gamut SDR surface is
  unrepresentable. Also `SurfaceTarget` and `HdrStatus::Hdr` triplicate the same
  format/color_space pair (01); and the status is global while failures are
  per-viewport (05).
- **Resolution:** add a `WideGamut`/`Gamut{..}`-class variant or restructure as
  `{format, color_space, hdr: bool}`; dedupe against `SurfaceTarget`; state the
  per-viewport story.
- **Severity:** critical.

### R3-3. Headroom is unreachable from eframe, and Q1 pre-commits while claiming to be open (04-critical, 01-major, 06-major)
- `DisplayHdrInfo` is only obtainable via a `wgpu::Surface`; `RenderState` exposes
  adapter/device/queue but never the surface (egui-wgpu lib.rs:104-126; surface private
  in winit.rs:12-14). Open question 1 still offering "nothing" leaves an eframe app
  unable to read headroom at all — the input the whole §7 story (marker, readout,
  gain-map compositing) depends on. Meanwhile the ladder table already commits the
  "per-viewport pass-through" side item that Q1 claims is undecided (06).
- **Resolution:** decide Q1 now: the pass-through PR must expose the surface-derived
  info (per-viewport `ViewportInfo` or a `render_state` accessor); until then the
  workaround app's out-of-band surface handle is the only path — say so.
- **Severity:** critical.

### R3-4. Off-path format identity is unpinned — and can silently be fp16 (05-critical, 01-major)
- Nothing forces decider-`Off` to reproduce `preferred_framebuffer_format`'s exact
  semantics; that function's `formats.first()` fallback can itself yield
  `Rgba16Float` on unusual drivers — which would falsify the plan's "PR 1/PR 2 u8
  readback unaffected" claim and change SDR apps' surface format. No equality test
  exists. The SDR fallback format rule is unspecified and the fate of the public
  `preferred_framebuffer_format` is unaddressed (01).
- **Resolution:** pin the Off ordering (today's exact preference list), add an
  equality test decider-vs-legacy, decide `preferred_framebuffer_format`'s fate
  (delegate to decider or deprecate).
- **Severity:** critical.

### R3-5. Multi-viewport / cross-display failure is a process crash presented as "documented behavior" (02-critical, 05-major, 04-major)
- Inherit-or-fail on a viewport whose display can't configure the stored target hits
  wgpu's default uncaptured-error sink = **panic** (wgpu api/device.rs:898); the
  escape (`on_uncaptured_error`) leaves the surface unconfigured → per-frame
  warn-skip-loop **hang**. No third outcome; the plan never says which. Opting into
  P3/Auto then dragging to an SDR monitor or opening a second viewport on mixed
  monitors crashes mainstream setups; no uncaptured-error handler exists anywhere in
  egui-wgpu/eframe (verified by grep). Probe-once also contradicts §7's
  display-agnostic claim when windows move across displays of different capability
  (05).
- **Resolution:** a real degradation policy: per-viewport SDR fallback on configure
  failure (decider re-run per viewport, status per viewport), or explicit panic with
  docs; pick one and write it down.
- **Severity:** critical.

## B. Arithmetic that failed reproduction (all independently recomputed this round)

### R3-6. PQ "≈2.12×" is wrong — actual ≈14.6× linear / ≈3.4× display nits (03-major, 06-major — independent)
- Both reviewers recompute from BT.2100: no normalization yields 2.12. 06 got ≈3.4×
  (too dark) at the 203-nit anchor, ≈4.5× at 100 nits; 03 got 14.6× linear (3.87
  stops) / 3.4× in nits. The qualitative point survives; the number is wrong.
- **Resolution:** recompute with shown work; replace the constant.
- **Severity:** major.

### R3-7. "−2.4 stops" is vs the opaque contribution, not the stated photometric composite (03-major, 06-major — independent)
- Correct figure vs photometric composite (a·L) is ≈ −1.30…−1.4 stops (`a^1.4`, plus
  toe effects); −2.4 is vs unattenuated L. Related: §2's "50%-alpha white over black
  composites to linear 0.214, not 0.25" — 0.214 is the **opaque** target; vs the
  composite the target is 0.5 (03).
- **Resolution:** restate both figures against a single declared basis.
- **Severity:** major.

### R3-8. fp16 subnormal floors are wrong, and the toe invalidates the "below JND" claim (03-major, 06-major, 04-minor — independent)
- Correct absolutes: 5.96×10⁻⁸ (linear) / 7.70×10⁻⁷ (gamma-equivalent); ratio ≈12.92 ✓
  but the plan's absolutes (4.6×10⁻⁹ / 4.7×10⁻⁶) match no basis and read inverted.
  Separately: the "2.4× slope" precision argument ignores the toe, where gamma
  quantization reaches ~1.0–1.3% — at or above the ~1% Weber JND, invalidating "both
  far below JND" as stated (03).
- **Resolution:** recompute both; restate precision honestly (toe is the interesting
  region, and it favors linear).
- **Severity:** major.

### R3-9. ΔE figures "Quantified" with no derivation; endpoints slightly optimistic (06-major, 03-minor)
- Which palette, which ΔE (CIEDE2000 stated), which white point, which re-encode
  pipeline — nowhere. 03's recomputation gives 6.8 (not 6.2) worst-accent and 0.21
  (not ≤0.19) re-encode. Load-bearing for PR 1's opt-in story.
- **Resolution:** record the derivation (palette + conditions) once; adopt 03's
  endpoints or justify the difference.
- **Severity:** major.

### R3-10. "Fixable only by a future linear-surface pairing" is impossible under the plan's own contract (03-major)
- The premultiply attenuation is baked into *storage* (`stored = a × enc(L)`); a
  linear surface can display stored values linearly but cannot recover the unattenuated
  product — the escape hatch requires a content-contract change (photometric
  premultiply), not just a surface pairing.
- **Resolution:** reword as content-contract evolution; note storage is the binding
  constraint.
- **Severity:** major.

### R3-11. Premultiply's forcing argument cites compositor semantics the plan itself defers as unverified (03-major)
- §4 justifies encoded-space premultiply by the compositor's `PreMultiplied`-over-
  stored-values contract; §9 defers exactly that behavior to an unverified manual
  checkbox. Citing an unverified fact as a forcing rationale is circular.
- **Resolution:** demote the compositor citation to a named manual experiment; rest
  the forcing argument on Color32 continuity alone.
- **Severity:** major.

## C. API and ladder

### R3-12. Double-grammar invariant example reads as forbidding PR 1's flagship pairing; "grammar" is never defined (06-major)
- `(Bgra8UnormSrgb, DisplayP3)` as a named-bad target while §5/ladder/§3 ship u8+P3 as
  the flagship — the valid pairing is `Bgra8Unorm` (raw view) + DisplayP3; the bad one
  is the sRGB-view format + P3 space (double transfer). "Grammar" appears 4× undefined.
- **Resolution:** define "grammar" once (sRGB-decode-view vs raw-view formats); rewrite
  the example with the valid flagship pairing stated beside it.
- **Severity:** major.

### R3-13. §3 contradicts the ladder on whether PR 1 ships `OutputEncoding` (01-major, 06-minor)
- Ladder: encoding selection in PR 4; §3 text implies PR 1 replaces the sniff.
  01 verified it must not (PR 1 has no color-space input to `RendererOptions`). §3's
  "invariant tests cover the whole table" also points at the wrong suite (06).
- **Resolution:** §3 = design that PR 4 activates; state PR-1 ships nothing of it.
- **Severity:** major.

### R3-14. Configure-coherence invariant has no enforcement point (01-major, 05-minor)
- The configure path never runs in CI. The plan asserts the built `SurfaceConfiguration`
  is coherent but specifies no mechanism.
- **Resolution:** extract a pure `build_configuration(...) -> SurfaceConfiguration`
  used by `configure_surface`, debug_assert coherence there, unit-test the builder.
- **Severity:** major.

### R3-15. Pairing table never enumerated; reject mechanism unspecified; legacy-content fallout unowned (04-major, 02-major, 06-minor)
- The load-bearing table (tag × surface → native/convert/reject) exists only
  implicitly. "Reject loudly" has no defined mechanism (error path? log-and-skip?
  must not panic in the paint path — 02). DisplayP3 policy would fail *every* legacy
  u8 `ColorImage` upload with no escape (04). "Pairing/pair" used in four senses
  (06).
- **Resolution:** enumerate the table; define the reject as a logged skip + `Result`
  on the API surface; give untagged `ColorImage` an implicit-Srgb native pairing on P3
  surfaces (convert opt-in) — i.e., the reject only fires for *tagged* mismatches.
- **Severity:** major.

### R3-16. Tone control for HDR content is unowned — the demo's hero path bypasses PR 2 (04-major)
- `Color32` tint caps at 1.0 (attenuation only); exposure via CPU = re-encode + 66 MB
  re-upload per tick; the only live path is a `CallbackTrait` shader — which is
  precisely the workaround the plan retires. Gain-map compositing needs this too.
- **Resolution:** either scope a minimal float-tint path (e.g., per-texture gain
  uniform via `TextureOptions`) into the ladder, or explicitly scope it out with the
  callback as the documented intermediate.
- **Severity:** major.

### R3-17. Headroom reactivity has no owner (04-major)
- eframe idles in `ControlFlow::Wait` (run.rs:297-300); OS brightness changes produce
  no winit event; the "250 ms idle repaint" recipe has no owner, no
  `request_repaint_after` placement, and an unstated power cost; 4 Hz is too coarse
  for the 1–2 s EDR ramp (marker smears).
- **Resolution:** specify the polling recipe (who calls `request_repaint_after`, at
  what cadence, gated on EDR displays only).
- **Severity:** major.

### R3-18. "Nothing may name a primaries" is speculative generality for a two-row table (02-major)
- The binding rule (from round 2) generalizes for a mechanism with exactly one wide
  gamut. Maintainer-gate flags it as the kind of abstraction upstream resists.
- **Resolution:** keep the rule as a *naming/branching guideline*, not an invariant;
  or justify with the concrete second primaries (BT.2020) landing path.
- **Severity:** major.

### R3-19. Sequencing and API gaps at tier boundaries (02-major)
- PR 2 should split: (2a) WGSL negative fix + ecolor contract tests; (2b) float
  content — 2a is independently shippable and de-risks the NaN path. PR 4's explicit
  encoding knob contradicts the plan's own no-knobs argument (§2) — derive, don't
  expose. PR 5 names `color_edit_button_hdr` but no value type (what does the picker
  edit — `Rgba`? a new HDR type?). Upload mechanics unpinned: `bytes_per_row` is
  hardcoded `4 * width` (renderer.rs:671) and must become f16-aware; `write_texture`
  always stages (row-by-row when `8·width` isn't 256-aligned) — "zero-copy" is
  CPU-side only (04).
- **Resolution:** adopt 2a/2b split; derive PR 4 encoding; define the PR 5 value type
  (linear `Rgba` per the earlier round's decision, restated); add an f16 upload
  mechanics note to §4.
- **Severity:** major.

### R3-20. BT.2020 absent from v1 `Primaries` while the PQ chain targets it (04-major)
- All iPhone/PQ content is BT.2020-contained; the §7 PQ chain emits BT.2020-referred
  linear that must then be converted to P3/sRGB primaries by the app. That conversion
  matrix is v1-required for the chain to terminate but lives in "demand-gated future".
- **Resolution:** either add `Bt2020` to v1 `Primaries` + the two conversion matrices,
  or move the matrix into the PQ-chain recipe explicitly as app-side code.
- **Severity:** major (question of where, not whether).

## D. Reliability and failure modes

### R3-21. Device-lost / OOM / oversized content: zero coverage (05-major)
- No mention of device-lost callbacks, OOM, or upload size limits anywhere in the
  plan; oversized `FloatImage` panics in the paint path via wgpu's create/queue
  errors; `Lost → RecreateSurface` logic recreates against a dead device.
- **Resolution:** add a failure-path subsection: size clamps (documented max, error
  not panic), device-lost policy (delegate to wgpu default), staging expectations.
- **Severity:** major.

### R3-22. Metal `ExtendedDisplayP3` carries a wgpu-hal TODO: "needs protection in surface_capabilities like the BT.2100 cases" (05-major)
- wgpu-hal metal/surface.rs:299; configure failure there is a panic or a silent sRGB
  reinterpretation via `setColorspace(None)`. The plan notes the OS-version caveat
  once (Appendix) but the exposure claims read unconditional (§5, §8).
- **Resolution:** attach the caveat everywhere ExtendedDisplayP3 capability is
  claimed; treat Metal P3-HDR pairing as probe-gated, not guaranteed.
- **Severity:** major.

### R3-23. Windowed smoke test omits the macOS ceremony and has no host crate (01-major)
- Needs main-thread entry (`harness = false` — kittest asserts against eframe spawning)
  and a home crate (egui-wgpu has zero dev-deps for a winit app).
- **Resolution:** name the host (egui_wgpu example bin or the test app), the entry
  ceremony, and the CI job wiring.
- **Severity:** major.

### R3-24. `OutputEncoding::of` totality gap; dither force-off plumbing (05-minor)
- `Rgba16Float` + `Srgb` is legal in wgpu but unlisted → `of` is not total over legal
  inputs. Dither force-off needs a format input at the renderer site
  (renderer.rs:968-970).
- **Resolution:** make `of` total (list every legal pairing, panic-free fallback) and
  note the dither plumbing change.
- **Severity:** minor.

## E. Upstream strategy and posture

### R3-25. Total silence contradicts CONTRIBUTING and invites closure; #8703 framing reads territorial (02-major)
- CONTRIBUTING: "for bigger things, file an issue first" — the plan's "no issue, no
  PR, no public comments" sets up a cold-contact mega-PR and closure under the
  "undue burden" clause. "We do not interact with that work" plus pre-planned
  "encoding supersession for its adopters" (verified: #8703 open/blocked, zero
  maintainer comments) reads as competing, not follow-up.
- **Resolution:** revise §10: issue-first for the architecture (after the app proves
  it, per the fork-first gate), reference #8703 as the foundation being extended,
  drop "supersession" language for "evolution".
- **Severity:** major.

### R3-26. The `Off`→`Auto` endgame default flip is a silent visual breaking change while Q2 is open (02-major)
- If `Auto` may select DisplayP3, the endgame default shifts every app's chrome by up
  to ΔE ≈ 6.8 with no opt-out mention.
- **Resolution:** state the rollout policy explicitly (minor-version flip? opt-in
  window? tie to Q2's resolution).
- **Severity:** major.

## F. Consistency, dust, and hedging

### R3-27. Appendix and cross-reference sweep (06-major)
- Appendix A references dangling `§7.2/§7.4/§7.5` (sections no longer exist) and an
  undefined "entry-4 protocol"; a wrong `§11` pointer; Appendix B says "2025-10-25"
  (should be 2026) and cites only `reviews/00`, not `reviews2/00`.
- **Resolution:** mechanical fix pass over both appendices.
- **Severity:** major (easy, but every reader hits it).

### R3-28. Leftover drafting artifacts (06-minor)
- "chrome shift" used before definition; headless `Sdr{NoSurface}` silently amended by
  PR 3; "+ #7484 semantics" cryptic; #8703 contingency stated twice verbatim (§2, §10);
  fp16-readback ownership drifting across three statements.
- **Resolution:** dust pass.
- **Severity:** minor.

### R3-29. Anchor drift and sizing (01-minor)
- Several ✓ anchors drift 10–30 lines (e.g., blend block :443-455, not :415-428);
  300–450-line PR 1 sizing optimistic once decider+status+tests+docs counted.
- **Resolution:** re-run the anchor sweep with line numbers refreshed; re-size after
  the API shapes settle.
- **Severity:** minor.

### R3-30. Hedge or verify the four remaining unhedged claims; standards citations (06-minor, 03-minor, 05-minor)
- Resize-driven reconfigure (manual item — keep), Linux Vulkan P3 "essentially never"
  (hedge), compositor >1.0 premultiply (named experiment — see R3-11), Existing-
  instance fallback (verify or hedge). Add: cite IEC 61966-2-1 extended range / CSS
  Color 4 for odd-symmetry; state macOS "1.0 = SDR white" is brightness-relative, not
  203 nits; negative scanout/sub-black per pairing untested; no-mipmap minification
  shimmer worth a limitations note (04-minor).
- **Resolution:** one hedging/citations pass.
- **Severity:** minor.

---

## Verdict (synthesized)

The architecture survives — every reviewer endorsed the extended-gamma contract, the
primaries-generic mechanism, and the fork-first ladder as sound. The plan does not yet
survive *implementation start*: the five blockers (R3-1…R3-5) are all API or policy
decisions the document claims are made but aren't, and the arithmetic section needs a
recompute pass before any number in it anchors an upstream issue. Theme of the round:
**the rewrite made the plan read decided where it is still open.**

| # | Severity | One-line fix |
|---|---|---|
| R3-1 | critical | Resolve Auto/DisplayP3 in one home; rename `Auto`→`BestAvailable` |
| R3-2 | critical | `HdrStatus` needs a wide-gamut-SDR state; dedupe vs `SurfaceTarget` |
| R3-3 | critical | Decide headroom exposure; surface handle must reach apps |
| R3-4 | critical | Pin Off-path format identity + equality test; decide `preferred_framebuffer_format` fate |
| R3-5 | critical | Per-viewport degradation policy instead of panic-or-hang |
| R3-6 | major | PQ constant → recomputed with shown work |
| R3-7 | major | Restate stops/0.214 figures on one declared basis |
| R3-8 | major | Recompute fp16 floors + toe/JND honestly |
| R3-9 | major | Record ΔE derivation; adopt 6.8/0.21 |
| R3-10 | major | Linear-pairing escape hatch → content-contract change |
| R3-11 | major | Demote compositor citation to named experiment |
| R3-12 | major | Define "grammar"; fix the flagship-pairing example |
| R3-13 | major | §3 = PR-4-activated design; PR 1 ships nothing of it |
| R3-14 | major | Pure `build_configuration` + debug_assert enforcement |
| R3-15 | major | Enumerate pairing table; Result-based reject; legacy u8 native pairing |
| R3-16 | major | Own a tone-control path or scope it out explicitly |
| R3-17 | major | Specify the headroom polling recipe |
| R3-18 | major | Soften the primaries-generic rule to a guideline |
| R3-19 | major | Split PR 2; derive PR 4 encoding; define PR 5 value type; f16 upload note |
| R3-20 | major | BT.2020: v1 variant or explicit app-side matrix in the PQ recipe |
| R3-21 | major | Failure-path subsection (limits, device-lost, staging) |
| R3-22 | major | Attach Metal ExtendedDisplayP3 caveat at every capability claim |
| R3-23 | major | Windowed smoke: host crate + main-thread ceremony |
| R3-24 | minor | Total `OutputEncoding::of`; dither plumbing note |
| R3-25 | major | Issue-first posture; #8703 as foundation, not rival |
| R3-26 | major | Default-flip rollout policy tied to Q2 |
| R3-27 | major | Appendix sweep (refs, dates, log citations) |
| R3-28 | minor | Drafting-artifact dust pass |
| R3-29 | minor | Anchor line-number refresh; re-size PR 1 |
| R3-30 | minor | Hedge/verify four claims; standards citations; limitations note |
