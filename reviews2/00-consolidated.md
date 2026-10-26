# Consolidated review 2 — re-audit of hdr-egui-upstream.md (post-decision state)

Second-round audit: six independent reviewers (`reviews2/01`–`06`) re-investigated the
amended plan and the justifications in `reviews/00-consolidated.md`. 65 raw findings →
**22 distinct entries** below. Unlike round 1, most findings here are about *our own
round-1/round-2 decisions* — stale text surviving 40+ in-place edits, an inverted labor
split, unverified assumptions the first round baked in, and numeric claims that didn't
survive independent reproduction.

**Reviewers:** 01 ConsistencyAudit · 02 FactCheck · 03 WideGamutSystems · 04 ColorMath ·
05 UpstreamStrategy2 · 06 KittestApiAudit.

**Headlines (3 critical):** (1) large blocks of pre-split/pre-rename text survive in
§7.2/§7.4 and the PR-number citations; (2) the pairing table's *default row*
(sRGB-tagged content on a P3 surface) is undefined; (3) PR 0 as specified is
upstream-unmergeable and internally untestable. Plus one inversion: the kittest sibling
fork (29e) turned out to be unnecessary — the labor split was backwards.

---

## A. Stale state & consistency (post-edit survivors)

### 1. Pre-split text survives in §7.2 and §7.4; six stale PR-number citations — critical
- **Raised by:** 01 (stale-stage survivors), 05 (§7.2/§7.4 critical), 03 (§7.2 block)
- **Finding:** Entry 29c split PR 1 into DisplayP3-SDR (PR 1) and extended-range (PR 2),
  but §7.2's "Design sketch — re-staged" block still stages PR 1 as
  `ExtendedDisplayP3`-on-fp16 with `ExtendedSrgb` as compat default (29b-era), and §7.4's
  HdrPreference paragraph still decides `HdrPreference::{Off, Auto, ExtendedSrgb}` under
  the pre-rename name with an `Auto` definition ("best HDR target") that contradicts
  §1's post-split sketch. Six more spots cite stale PR numbers (§2, §7.1, §7.3, §7.4,
  §7.9's issue anchors: "PR 1 → #7484" now maps to PR 3's recreate doc; "PR 2 → #6863"
  now PR 2-encoding/#6863 mapping needs recheck). Implementing from §7.2/§7.4 would drag
  the fp16 capture cluster back into PR 1.
- **Suggested resolution:** One consistency pass over §7.2/§7.4/§5: rewrite the §7.2
  sketch to the 29c ladder (PR 1 = DISPLAY_P3 SDR; PR 2 = extended pair), rename
  HdrPreference→SurfaceColorSpacePreference in §7.4, re-map issue anchors to the new PR
  numbering, and grep every "PR 1"/"PR 2" mention.

### 2. §4 and §7.5 still route DisplayHdrInfo through the cut `frame.info()` mirror; §4's capture claim contradicts §7.7 — major
- **Raised by:** 01
- **Finding:** Entry 22 cut the `frame.info()` mirror from the *reporting* section (§7.4),
  but §4's headroom bullet and §7.5's consequence bullet still say "expose through
  `RenderState`/`frame.info()`". §4 also still says float-target capture "will lie"
  while §7.7 correctly documents the silent-failure mode. §7.3's new windowed CI smoke
  test contradicts §7.7's "every windowed-surface behavior is manual-matrix only" (the
  smoke test verifies *integration*, §7.7's sentence reads as *all* windowed behavior).
- **Suggested resolution:** Sweep §4/§7.5 to the entry-22 decision (RenderState-only,
  per-viewport later); reword §4's capture line to "silently produces nothing (entry
  34)"; reconcile §7.3 smoke-test vs §7.7 manual-only sentence ("integration CI-testable,
  payoff manual").

### 3. Entry-number drift: several "entry N" citations point at superseded wording — major
- **Raised by:** 01
- **Finding:** e.g. citations to entry 24's variant list predate 29b/c's
  `ExtendedDisplayP3`/rename; entry 16's continuity argument predates the P3 promotion;
  "entry 2" glow scoping is cited for tier E but the glow question now has a second
  dimension (egui_glow compile-vs-display under the pairing table). Each is harmless
  alone; together they make the plan's cross-reference web unreliable for an implementer
  who reads only the cited entry.
- **Suggested resolution:** During the entry-1 consistency pass, replace bare "entry N"
  citations that resolve to superseded text with the current decision restated inline.

---

## B. PR-1 architecture (the new, round-2-specific surface)

### 4. PR 1 must name `SurfaceColorSpaces::DISPLAY_P3` — the plan never does — critical
- **Raised by:** 01 (critical), 02 (confirmed), 03, 05 (mechanism sound)
- **Finding:** The fp16-free DisplayP3-on-u8 design is real — wgpu 30 has a distinct
  non-extended `SurfaceColorSpace::DisplayP3` (sRGB transfer, P3 primaries, SDR clamp,
  `is_hdr()==false`), reported unconditionally for u8 formats on Metal and **already
  reported for u8 canvases on web** (webgpu.rs:4154-4160 — the plan's "verify at
  implementation" deferral is stale). But the plan's §7.2 verification block and §5 PR-1
  item justify the pairing via `EXTENDED_DISPLAY_P3`, which is fp16-gated — an
  implementer pairs the tag to the wrong space and PR 1 silently delivers SDR. Also:
  Metal's u8 surface format is **`Bgra8Unorm`**, not `Rgba8Unorm` as the plan's PR-1
  text says.
- **Suggested resolution:** PR 1's pairing text names `DISPLAY_P3` explicitly
  (non-extended, u8-valid, is_hdr()==false); fix surface format to `Bgra8Unorm` on
  Metal; drop the web "verify at implementation" hedge (already reported); note
  `DisplayP3`'s absence on DX12 (deterministic SDR fallback there — add the §7.3 row).

### 5. The pairing table's *default row* is undefined: sRGB-tagged content on a P3 surface — critical
- **Raised by:** 03 (critical), 01 (payload gaps)
- **Finding:** Every `ColorImage` texture egui ever uploads stays implicitly
  Srgb-tagged forever. The moment any app opts into the P3 policy, its existing images
  hit the (Srgb-content × P3-surface) cell of the pairing table — reject, silently
  shift, or convert? The plan specifies neither this cell (the most-trafficked one) nor
  the multi-viewport content-clip case. Additionally the pinned `HdrStatus` payload
  cannot express PR 1's outcomes: `Hdr { format, color_space }` can represent them, but
  `SdrReason` lacks a P3-unsupported reason distinct from `NoExtendedSrgb`, and
  `OutputEncoding`'s table lacks the fp16+`EXTENDED_DISPLAY_P3` → `GammaEncoded` row
  (PR 2).
- **Suggested resolution:** Specify the mismatch row in PR 1: v1 = "surface pairs only
  its own primaries tag; mismatched-tag uploads reject loudly (same semantics as PR 0)"
  with conversion explicitly deferred; extend `SdrReason` (`NoDisplayP3` or fold into a
  `UnsupportedSurfaceSpace(SurfaceColorSpace)`); add the PR-2 encoding row.

### 6. PR 0 as specified is upstream-unmergeable and internally untestable — critical
- **Raised by:** 05 (critical), 03 (reject mis-layered)
- **Finding:** PR 0 = dead public API (`FloatImage` with an upload-rejecting primaries
  tag, no consumer), an `Eq`/serde break with zero functionality, a WGSL sign fix for
  negative values that cannot exist yet, and its GPU negative round-trip test requires
  PR 2's fp16 `RenderState`. No groundwork-without-consumer precedent in recent egui
  merges (#8644, #8686, #8714 all shipped consumed functionality); CONTRIBUTING demands
  issue-first for big things. Separately: reject-at-upload is mis-layered — the
  TextureManager is surface-blind; only egui-wgpu's `update_texture` knows the paired
  space — and `Bt2020` is a permanently unpairable variant under the plan's own
  primaries×transfer framing (a category error to enumerate).
- **Suggested resolution:** Either (a) fold PR 0 into PR 1 (groundwork lands *with* its
  consumer — the DisplayP3 pairing is the consumer), or (b) re-scope PR 0 to the
  mergeable subset: WGSL sign fix + ecolor contract tests (real value, no new public
  API). Drop `Bt2020` from the v1 enum (add when a BT.2020 pairing exists); state the
  rejection point as `update_texture` (egui-wgpu), not the texture manager.

### 7. Demand map is inverted by the reorder — major
- **Raised by:** 05
- **Finding:** Live upstream demand is for **HDR brightness** (#7484 bug, LightCraft
  production use via #8703) — which the reorder moved to PR 2. Wide gamut has *zero*
  live demand (#2712 dormant since 2025-10-17, and its actionable residue is the glow
  bug the plan scopes out). Meanwhile §7.9 still anchors "PR 1 → #7484" although the
  recreate-semantics fix moved to PR 3. The first upstream PR now leads with the
  *lower*-demand feature.
- **Suggested resolution:** Either re-anchor PR 1's issue to the wide-gamut case on its
  own merits (P3 displays are common; #2712 proves the problem exists) *and* fix the
  anchor mapping, or reconsider once more whether PR 1 should carry
  `ExtendedSrgb` (brightness, the demanded thing, zero chrome shift) alongside
  `DisplayP3` — both are u8/fp16-cheap to ship together given PR 2 exists for the rest.
- **Severity note:** strategy-level, not technical.

### 8. #8703 collision is under-priced, and entry-6's total silence forfeits the cheap mitigation — major
- **Raised by:** 05
- **Finding:** #8703 unchanged (OPEN, BLOCKED, bot-only comments since 2026-10-19). If it
  merges during fork work, the collision with our evolved API is semantic, not
  mechanical: field relocation off `SurfaceConfig`, silent encoding break for #8703
  adopters (their scRGB-linear choice vs our pairing), capture.rs conflicts. Entry 6's
  "no public upstream contact" rule forfeits the one cheap mitigation (a single
  informational comment).
- **Suggested resolution:** Keep the no-PR rule but carve out one exception: a single
  watch/comment on #8703 flagging that a follow-up pairing/color-space policy is planned
  — or accept the collision cost explicitly in the decision log.

### 9. kittest MSAA closure (entry 38) is unexecutable as written — major
- **Raised by:** 06
- **Finding:** Entry 38 closed the MSAA item as "CI-testable: kittest
  `with_render_options(msaa_samples: 4)`", but the offscreen harness hardcodes
  `sample_count: 1` and `resolve_target: None` (`wgpu.rs:202/217`) while pipelines take
  `msaa_samples` — msaa=4 is a validation error on any format. The closure is currently
  false.
- **Suggested resolution:** Either add harness support (plumb sample_count +
  resolve_target through `WgpuTestRenderer::render`) or downgrade the MSAA item back to
  "hardware-verified, CI-blocked by harness gap" and add the harness gap to entry 34's
  deliverable list.

---

## C. Entry-34/35 landing corrections (kittest)

### 10. Entry 29e's labor split is INVERTED — the sibling fork is unnecessary — critical
- **Raised by:** 06 (critical), 02 (major), 05 (§7.9 prices two forks)
- **Finding:** Verified against the sibling's full git history: rerun-io/kittest 0.5.0 is
  an *accesskit query layer only* — `Harness`, `HarnessBuilder`, snapshot machinery, and
  any wgpu/renderer/readback code **never existed there**; all of it lives in-repo in
  `crates/egui_kittest` (lib.rs:124 Harness, builder.rs:10 HarnessBuilder,
  snapshot.rs:13, wgpu.rs renderer). The header's "Sibling checkout" block and §7.7
  deliverable 2 ("builder API lives in the sibling checkout") are inverted; §7.9's
  fork-maintenance price books a second fork that shouldn't exist.
- **Suggested resolution:** Delete the header "Sibling checkout" block; rewrite §7.7
  deliverable 2 to in-repo only (`crates/egui_kittest/src/builder.rs`); correct 29e in
  the decision log (keep the checkout pointer as reference-only, or note it can be
  removed). The fork stays one repo.

### 11. Entry 34's escape hatch already works; the deliverable-1 home conflicts with entry 35 — major
- **Raised by:** 06
- **Finding:** `Harness::builder().renderer(WgpuTestRenderer::from_render_state(fp16_state))`
  works *today* — both are pub, `RenderState` is all-pub. So deliverable (2)'s "surface
  it on the builder" is already done; the only real override work is egui-wgpu's
  no-surface stub (lib.rs:267). Separately, deliverable (1) homes `texture_to_float_image`
  in egui_kittest, but the numeric assertions (entry 35) live in egui-wgpu — which has no
  dep edge to egui_kittest and no float egress through the `RgbaImage`-typed harness API.
- **Suggested resolution:** Deliverable (2) shrinks to "use the existing escape hatch +
  fix the stub"; deliverable (1) lands the float readback in **egui-wgpu** (next to
  capture.rs, sharing #8703's decode code) with numeric tests there — egui_kittest keeps
  only the clamp-to-u8 golden path.

---

## D. Color-math refutations (round 2, numeric)

### 12. The multiply-error *mechanism* claim is refuted: it's knee-crossing, not the intercept — major
- **Raised by:** 04
- **Finding:** Entry 17's corrected text says "dominant term is the −0.055 intercept".
  Refuted by decomposition: at linear 0.01 the driver is the *encoded product (0.00997)
  crossing the decode knee (0.04045)*; at ≤0.0031308 the composition is exactly
  `12.92·ab` with no intercept active; and in the power region zeroing the encode
  intercept alone makes the error 26× *worse* (+1.49% → +39.2%). No single term
  dominates. The *measured values* (+1.49/+0.13/+37.7/+671.7%, exactly 12.92× at
  0.001) all reproduce.
- **Suggested resolution:** Rewrite the mechanism sentence: "the error is driven by the
  encode/decode knees: products whose decoded value crosses the 0.04045 boundary take
  the wrong branch's scale; in-toe products are exactly 12.92× off" — and cite the
  measured table.

### 13. Entry 30's decomposition: canonical-band clause is a no-op; NaN/destructive edges — major
- **Raised by:** 04
- **Finding:** Decompose→recompose is bit-exact across the extended range, but the
  "chroma max clamped to [0.5, 1]" clause is a no-op as written (under real-valued log2,
  chroma max is always exactly 1.0). And the edges are unspecified: black (max ≤ 0 →
  log2 undefined/−inf), all-negative or mixed-sign colors (which max?), and additive
  alpha=0 input — `to_rgba_unmultiplied` assumes black at a=0 (`rgba.rs:200-205`), so
  editing an additive-extended color silently destroys it.
- **Suggested resolution:** Spec the edges: max ≤ 0 → exposure 0, chroma = color
  (round-trip identity); negative channels → decomposition uses max(|c|) and chroma
  carries the sign (contract already odd-symmetric); a=0 → defined passthrough. Drop the
  band-clamp clause or state the integer-stops variant that makes it real.

### 14. Entry 19's premultiply rule: verified, but the HDR cost is unpriced and "forced by the pipeline" overclaims — major
- **Raised by:** 04
- **Finding:** The rule reproduces (a=0.5, L=4 → 0.912), but its HDR cost was never
  stated: 50%-alpha L=4 *displays* as linear 0.812 where a photometric composite would
  give 2.0 — generally encoded-space premultiply attenuates by `a^2.4` (50% alpha = −2.4
  stops). The alternative `enc(a·L)` isn't a fix (overshoots over non-black
  destinations). Also "forced by the pipeline" is wrong — the fixed-function blend is
  convention-agnostic; the real forcing is continuity + the compositor's `PreMultiplied`
  semantics.
- **Suggested resolution:** State the cost honestly ("HDR transparencies darken:
  50% alpha ≈ −2.4 stops; the fix is alpha-domain work — an app concern, documented"),
  and replace "forced by the pipeline" with the continuity+compositor rationale.

### 15. PQ decode chain is never written; double-encode and negative hazards — major
- **Raised by:** 04
- **Finding:** §7.5/§7.2 say "PQ decodes app-side into the storage contract" but the
  chain is never spelled out, and it has verified hazards: using sRGB-inverse instead of
  PQ⁻¹ = 2.12× brightness error; the BT.2020→P3 matrix must run in *linear* light
 (matrix-after-gamma-encode = garbage); demuxed gamma-P3 output re-encoded into gamma
  storage double-encodes; BT.2020→P3 conversion emits negative components the plan never
  mentions in this context; the `Primaries` tag cannot express HLG.
- **Suggested resolution:** One §7.5 paragraph: the exact chain (PQ⁻¹ in BT.2020 linear
  → 203-nit anchor scale → matrix to paired primaries → gamma encode), the negative
  output note (entry 18 covers the math), and "HLG = out of scope until a tag carries
  transfer".

### 16. f16 tolerance and subnormal labels are wrong in entry 35/§7.1 — minor
- **Raised by:** 04
- **Finding:** f16 max normal relative error is 2⁻¹¹, not ±2⁻¹⁰ (that's the full-ULP
  bound) — and the tolerance is *violated by subnormals* (linear 1e-6 → rel err 1.1e-3),
  so entry 35's assertion tolerance will flake on dark values. "Subnormal floor L ≈
  4.7e-6" mislabels the boundary: that's *gamma's* subnormal onset; linear storage's true
  floor is ~4.6e-9 with 0.098% step (not 0.5%); gamma's subnormal region is 12.9×
  *darker* than linear's.
- **Suggested resolution:** Fix the numbers (2⁻¹¹ normal, explicit subnormal carve-out
  or relative-in-log-space tolerance; corrected floor values).

### 17. Entry 16's "bit-identical" is formula-true, bit-false on fp16 surfaces, and vacuous for extended values — minor
- **Raised by:** 04
- **Finding:** On an fp16 surface, ~3.3% of u8 blend combos shift 1 LSB through f16
  storage + readback vs the u8 path (concrete example in the review) — formula-identical,
  not bit-identical. And for extended values "bit-identical to today's SDR path" is
  vacuous: extended colors never existed in the u8 path.
- **Suggested resolution:** Reword: "formula-identical to the SDR path; on fp16 targets
  ≤1 LSB rounding differences vs u8 targets; extended values have no SDR baseline."

---

## E. Platform & upstream facts (round 2)

### 18. "No HDR on Windows until scRGB" is over-absolute: DX12 reports an HDR10 path — major
- **Raised by:** 02
- **Finding:** DX12 reports `Rgb10a2Unorm: SRGB | BT2100_PQ` — a real HDR10 (PQ, BT.2020
  primaries) surface path the plan's resolved matrix omits. It doesn't fit the
  gamma-storage contract (PQ transfer), but "Windows = zero HDR until PR 3" is stronger
  than the platform truth; the accurate statement is "no *extended-sRGB-family* HDR on
  DX12; HDR10/PQ exists but is out of the v1 transfer contract".
- **Suggested resolution:** Add the DX12 `Rgb10a2Unorm/BT2100_PQ` row to §7.3's table
  marked out-of-scope (transfer contract), and soften the "no HDR until PR 3" phrasing
  to "no extended-sRGB-family HDR".

### 19. 0.37 shipped 2026-10-15; upstream privatized `ColorImage` (#8714) — timing and house-style are stale — major
- **Raised by:** 05
- **Finding:** egui 0.37.0 released 2026-10-15 (#8725) — the §7.9 "target the 0.37
  window?" question is already answered by time; the ladder realistically lands 0.38+.
  Fresher drift: emilk's #8714 (merged 2026-10-19) privatized `ColorImage`'s fields and
  renamed `size`→`resolution` — the plan's `FloatImage { size, primaries, pixels }`
  public-field sketch is now anti-house-style *and* the "re-anchor at draft time"
  ordering (entry 4) is falsified for the data-model layer: the FloatImage design is
  already stale against main.
- **Suggested resolution:** Answer the timing question (0.38 target), and re-anchor the
  FloatImage sketch now (constructor-based API à la post-#8714 `ColorImage`, field
  naming per `resolution`) — or accept the churn knowingly at draft time. Also pull the
  #8714 lesson into PR-0's design: check `ColorImage`'s current API shape before
  mirroring it.

### 20. Chrome-shift is quantified and small; the follow-up conversion is NOT mandatory for chrome — minor
- **Raised by:** 03
- **Finding:** CIEDE2000 analysis: neutrals shift exactly 0.00 ΔE on a P3-referred
  surface (shared D65 white); saturated accents 1.4–6.2 ΔE (chroma +8–36%); re-encoding
  the u8 style palette into P3-referred is near-lossless (≤0.19 ΔE); the converted
  palette's gamma-blend residual is ≤~3.2 ΔE. Verdict: v1 "app-side conversion" is
  workable, and egui-side chrome conversion is a genuine follow-up, not a hidden
  requirement. Unowned remainder: per-*texture* content conversion (apps with sRGB
  images on a P3 surface).
- **Suggested resolution:** Record the numbers in §7.2 (they de-dramatize the opt-in
  caveat) and add the per-texture conversion to the "later, demand-gated" list.

### 21. Entry 16's zero-shift continuity survives only on Off/ExtendedSrgb; `Auto` collision — minor
- **Raised by:** 03
- **Finding:** Post-reorder, PR 1 has *no* zero-shift wide-gamut mode — the continuity
  argument now scopes to Off/ExtendedSrgb (fine, but say it). New collision: §1's `Auto`
  comment ("best available: DisplayP3 → extended pair") would turn the chrome shift on
  globally when Auto lands, contradicting §7.4's extended-only Auto definition.
- **Suggested resolution:** One sentence in §7.4: Auto's preference order and whether
  plain DisplayP3 is ever Auto-selected (recommendation: Auto = extended pair only, P3
  stays explicit — or Auto docs carry the shift caveat).

### 22. Misc factual dust — minor
- **Raised by:** 02, 03, 05
- **Finding:** (a) workspace locks wgpu **30.0.0**; round 1/plan cite 30.0.1 (diffed
  identical — record the lock version); (b) `EXTENDED_DISPLAY_P3` on Metal carries an
  upstream OS-version TODO (note it); (c) the §7.4 invariant test misses the
  `(Bgra8UnormSrgb, DisplayP3)` double-grammar pair wgpu would accept — add to fixtures;
  (d) entry 9/§7.7's "#7484 black screen is the no-fp16 case" garbles the issue (it's
  the *partial-recreate* case — format switch + renderer recreate with stale caches);
  reword to "the #7484 failure is the partial-recreate case; fp16-unsupported surfaces
  fail differently".
- **Suggested resolution:** Batch these into the entry-1 consistency pass.

---

## Verdicts (unedited, one per reviewer)

- **01 ConsistencyAudit:** the plan's decisions are individually sound but the 40-edit
  in-place process left pre-split/pre-rename text live in §7.2/§7.4 plus citation drift;
  PR 1's actual color space is never named. One focused consistency pass fixes it.
- **02 FactCheck:** 9 of 9 load-bearing round-1 corrections re-verified with fresh
  citations — the plan's fact base holds; PR 1's no-fp16 premise *holds* (DISPLAY_P3 on
  u8, Metal + web); main casualties are the 29e labor split and over-absolute Windows
  phrasing.
- **03 WideGamutSystems:** PR 1 does not collapse — DisplayP3-on-u8 is valid and the
  demo claim survives; the real gaps are the undefined default pairing row, unowned
  content conversion, and mis-staged plan text.
- **04 ColorMath:** all of entry 17's corrected numbers reproduce; three of the plan's
  mechanism/rationale sentences don't survive decomposition (intercept, band clamp,
  pipeline-forcing); entry 30 needs edge semantics; premultiply's HDR cost needs stating.
- **05 UpstreamStrategy2:** #8703 unchanged; PR 0 must ride with PR 1; demand map is
  inverted by the reorder (HDR is the demanded thing and now sits in PR 2); 0.37 already
  shipped and #8714 privatised ColorImage — re-anchor now, not at draft time.
- **06 KittestApiAudit:** the sibling fork is unnecessary (labor split inverted — all
  machinery is in-repo); the escape hatch already works; the MSAA closure is
  unexecutable as written; deliverable homes need the egui-wgpu/egui_kittest boundary
  drawn correctly.
