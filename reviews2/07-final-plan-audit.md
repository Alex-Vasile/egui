# Final-plan audit — hdr-egui-upstream.md

Audit mode: full read as a final document (all 1164 lines) + cross-check against the
decision log (`reviews/00-consolidated.md:718-779`, entries 1–40 incl. 29b/c/d/e,
R2-1…R2-22) and against the working tree / registry sources. All line numbers below are
`hdr-egui-upstream.md` unless prefixed otherwise.

---

## 1. Reading experience (top-to-bottom)

The material is complete and internally rich, but the document still reads as working
notes with two review rounds painted over. A reader who never saw the rounds hits the
following, in document order:

### 1.1 The title and framing contradict the artifact
- **l.1–3**: "# egui HDR support — working notes … Accumulates findings … **not an
  implementation plan**." The opening sentence explicitly disclaims the thing the
  document now is. Everything downstream is sequenced PRs, pinned API shapes, and
  binding design rules. This is the single most jarring line for a fresh reader.
- **l.4–18**: the app-workaround backstory (2×2 quad, orange marker, 250 ms idle
  repaint, `../hdr-test/rust`) is context, not plan; it sits above the fold and mixes
  "choices we validated" narrative with normative statements.
- **l.19–23**: tracker status is narrated as a *history of checks*: "checked
  2026-09-11; **re-verified 2026-10-25**: the 2026-09-11 check found no open HDR issue
  or PR, but as of 2026-10-12 PR #8703 …". A final plan states the current fact once
  (PR #8703 occupies tier B; dated), not the audit trail of two lookups.

### 1.2 ~44 dated correction/decision annotations interrupt the text
`grep` counts 44 lines carrying "(corrected/added/decided/resolved/amended/recorded
2026-10-25)" markers. Each one is a narrated diff against a prior draft the reader
never saw. Representative, in order: l.32–34 (kittest: "corrected 2026-10-25 — the
earlier labor split was inverted"), l.59–61 ("shape decided 2026-10-25; `DisplayP3`
added and HDR variants moved to PR 3 by the split-PR reorder same day" — inside a code
comment), l.70–77 ("direction recorded 2026-10-25"), l.90–93 ("corrected 2026-10-25" on
the #7484 partial-recreate story), l.96–99 ("corrected 2026-10-25; 'the one
shader-correct combination' was false"), l.105–107 ("the PR-1 reorder promoted P3"),
l.120–134 (sizing: "the original '~150 lines' pitched §1's core … Still cheap; now
honestly priced"), l.242–243 ("supersedes 'needs surface recreate at minimum'"),
l.328–329 ("qualifiers added 2026-10-25"), l.337 ("Upstream sequencing — RESTRUCTURED
2026-10-25 … PR 0 dissolved into its consumers" as a *section heading*), l.368
("**Restructure rule (user directive, 2026-10-25 — binding)**"), §6's two "corrected /
amended 2026-10-25" notes (l.397–412), §7.1's five (l.446, l.454, l.459, l.465–466,
l.497–499, l.509–516, l.529, l.533–535), §7.2's header "supersedes the 2026-09-12
non-goal" (l.571) plus "user directive … treat as binding" (l.581) and "re-staged
2026-10-25 … split ladder per 29b/29c" (l.637), §7.4's "renamed from the round-1
`HdrPreference` sketch by the 29c split" (l.726–727) and the entire "**Reporting
mechanism corrected 2026-10-25 (three defects in the original sentence)**" block
(l.737–752), §7.7's header "(corrected 2026-10-25)" plus "**The original text assumed
kittest could already test fp16 encoding** — false at 0.36.2 and HEAD, three ways"
(l.920), §7.8's "**Resolved 2026-10-25** (these were 'verify' items …)" (l.995), §7.9's
"must be restated" (l.1091). None of these survive into a final document; each must
become either a plain statement of the current truth or nothing.

### 1.3 Decision-log entry IDs are cited mid-sentence ~30 times
The plan references log rows a fresh reader cannot resolve: "decision log R2-10"
(l.42), "entry 22" (l.133, l.247, l.252, l.870), "entry 4" (l.134, l.1099), "entry 30
shape" (l.362), "entry #29's f16-in-RAM decision" (l.332 — note the inconsistent `#`
style), "entry 10" (l.346, l.360, l.1013), "entry 34" (l.357, l.1002, l.1023), "entry
27" (l.653), "entry 16's residual cost" (l.465), "entry 18" (l.452 area, l.956),
"entry 2" (l.313, l.1024), "entry 26 removes preferred_color_space" (l.799), "entry
13" (l.803, l.1016), "entry 25's simplification" (l.818), "entry 35" (l.838, l.1000),
"entry 28" (l.1029), "entry 20" (l.1029), "round-2 entry 21" (l.734), "entry #12's
create-time-only policy" (l.743), and worst, raw log IDs: "split ladder per 29b/29c"
(l.637), "PR numbers per the 29c split" (l.683), "renamed from the round-1
`HdrPreference` sketch by the 29c split" (l.726–727). None of these identifiers exist
in the final document's world; the decision log stays external, so every such citation
is dead weight that must be deleted or converted into the fact it annotates.

### 1.4 Decision statements are duplicated across sections
- **Policy defaults** (Off = rollout default; Auto = endgame once fp16 capture/kittest
  infra exists; fork opts in unconditionally): stated at l.70–77 (§1) *and* l.730–733
  (§7.4). Same words, two homes.
- **The gamma-compositing permanent cost** ("50%-alpha white over black → linear 0.214,
  not 0.25"): stated at l.119–122 (§1) *and* §7.1 rationale point 2 (l.490–493).
- **The DisplayHdrInfo "own later PR" decision**: stated four times — §4 l.251–253, §7.4
  l.790–795, §7.5 l.869–872, §5 l.363–364 — *and* those four statements do not agree
  (see §2.4 below).
- **Create-time-only**: §1 l.62–65, §4 l.215–219, §7.4 l.743–744. Two of the three are
  legitimate (policy + rationale); the third restates.
- **decide_surface_target stores three outputs / configure_surface reads the stored
  field**: stated twice inside §7.4 itself (l.799–803 and l.822–827).

### 1.5 Ordering problems
- **§1 depends on §5 and §7.4 before they exist.** l.105: "The §7.4 invariant test
  asserts…"; l.129–130: "PR 1 as sequenced (§5) adds `decide_surface_target` +
  `HdrStatus` + the primaries pairing table…"; l.94: "payload pinned in §7.4". The
  reader meets `HdrStatus`, `decide_surface_target`, and "the primaries pairing table"
  with no definition for ~300–600 lines.
- **§2 depends on the §7.1 contract** (the encoding table is only interpretable with
  the extended-gamma contract, which appears at l.443, 300 lines later).
- **The ladder — the plan's actual backbone — first appears at l.259 (§5)**, after two
  sections that already use its vocabulary ("PR-4 scRGB work" l.152, "PR-3 deliverable"
  l.256). A final plan needs the ladder up front.
- **§7 is titled "Open questions / not yet discussed" (l.436) with intro "Threads
  surfaced but not resolved" (l.438)** — yet it contains the plan's *decided core*:
  the color contract, the gamut scope, the platform matrix, the reporting API, the
  calibration data, the testing story. Every subsection header is a DECIDED/RESOLVED
  banner. The frame and the content are opposites; a reader cannot tell what is still
  open (answer: a short list — see §3.5 below).
- **The Appendix anchor base (0.36.2) contradicts the re-anchor decision** (l.1086–1092):
  the plan's own §7.9 says anchors must be re-verified against HEAD before any fork
  code, and half the inline cites already are HEAD numbers while the Appendix and §1/§3
  are not (see §2.13).

### 1.6 Note-taking artifacts a plan should not have
- Strikethrough-and-removal records: "~~`preferred_color_space`~~ — removed 2026-10-25"
  (l.818); the whole bullet is a tombstone.
- A meta-block about the section's own history: "Missing from this list (added
  2026-10-25; cross-referencing decided entries): (a)…(f)" (l.1022–1029) — §7.8
  contains a patch note about §7.8.
- Governance language aimed at reviewers, not implementers: "user directive", "binding",
  "treat as binding" (l.368, l.581), "(recorded 2026-10-25, for the future issue
  draft)" (l.1038), "(accepted, option 1 sharpened; do NOT flip)" phrasing implicit in
  l.1050 area.
- Four bracketed `[INFERENCE …]` tags (l.243, l.702, l.1014, l.1020) — honest, but
  scattered; a final plan collects unverified items in one place (see §4 below).
- Research-notes voice in §7.2's #2712 lead list (commenter names + dates as evidence
  trail, l.611–635): fine as sources, but written as mined-notes, not as design
  rationale.

---

## 2. Contradictions & staleness (section-by-section)

Numbered for rewrite use. "Stale" = surviving pre-decision text; "contradiction" =
two current statements that cannot both stand.

### Header (l.1–44)
1. **l.1–3 — identity contradiction**: "not an implementation plan" vs. the document's
   actual content (sequenced PRs, pinned payloads, binding rules).
2. **l.19–23 — check-history narration** (see 1.1). Not wrong, just pre-final.
3. **l.32–42 kittest note**: content is correct and matches R2-10 (all fp16 work
   in-repo; sibling is reading reference only). The "(corrected …)" framing is the only
   problem.

### §1 Presentation (l.46–134)
4. **l.87–89 vs §7.4 l.834–837 — invariant-test mismatch (contradiction)**: §1 says
   "The §7.4 invariant test asserts the *built* `SurfaceConfiguration` is coherent, not
   just `decide_surface_target`'s return." §7.4's invariant-test bullet (l.834–837)
   asserts only that the decider's returned triple is coherent; there is no
   built-`SurfaceConfiguration` assertion anywhere in §7.4. Entry 9 promised it; the
   §7.4 test list never received it. Either §7.4 gains the test or §1 drops the claim.
5. **l.128–131 — sizing inventory keyed to the wrong ladder (stale)**: "PR 1 as
   sequenced (§5) adds `decide_surface_target` + `HdrStatus` + **the primaries pairing
   table** + tests + `MANUAL_TESTING.md` + the example". Under the final ladder (R2-6),
   PR 1 has *no* primaries pairing table — §5's PR-1 item (l.346–347) says "No epaint
   changes, no new color types", and the tag/`update_texture` pairing lands in PR 2
   (l.349–352). The phrase is an entry-39 survivor of the 29b/29c ladder. (The
   decider's internal preference→target table is not "the primaries pairing table".)
6. **l.82–84 — probe mechanics predate the decider (stale)**: §1 still describes
   `RenderState::create` "probes `caps.color_spaces(Rgba16Float).contains(…)`" — the
   pre-entry-25 sketch. The decided mechanism (§7.4 l.795–803) is
   `decide_surface_target(caps: &SurfaceCapabilities, pref)`, with HDR formats coming
   from `format_capabilities` (§7.7 l.966–968). §1 should delegate to the decider in
   one sentence, not restate older mechanics. (The `caps.color_spaces(format)` helper
   does exist in wgpu 30 — verified at `wgpu-types-30.0.1/src/surface.rs:544` — so the
   sentence is not *false*, it is superseded.)
7. **l.105–108, l.120–134 — sizing narration** (see 1.2); the ~150/~300–450 numbers
   themselves are the entry-39 decision and are fine.
8. **l.133–134 — "the mirror was cut, entry 22" / "Re-measure … (entry 4)"**: log
   citations; facts correct.

### §2 Output encoding (l.136–176)
9. **l.154–158 — PR-1 rationale sentence keyed to the pre-split PR 1 (stale)**:
   "Consequence: **PR 1 needs no encoding change at all** (fp16 + `ExtendedSrgb` →
   gamma passthrough is exactly today's `is_srgb()`-false path)". Under the final
   ladder, PR 1 is u8 + `DisplayP3` (which the table one line earlier correctly maps to
   `GammaEncoded`, also no change). The *conclusion* survives; the *justification* is
   the old 29c-era PR 1 (fp16+ExtendedSrgb — now PR 3). Rewrite the parenthetical.
10. l.150–154 encoding table: correct and complete per R2-5 (all five rows present,
    including u8+DisplayP3 and fp16+ExtendedDisplayP3). No issue.
11. l.159–163 dither paragraph: correct per entry 20 (force-off on float targets; knob
    untouched for u8).

### §3 HDR content (l.178–211)
12. **l.195 — "Self-contained, additive, doesn't disturb the SDR path" (contradiction)**:
    superseded by entry 28 / R2 tier-C repricing, which §5 correctly carries (l.302–305:
    "additive + one breaking enum variant"; Eq cascade; serde changelog). §3's blanket
    "additive" is the pre-review claim. Also internally inconsistent with §5's tier-C
    row (l.270–272) in the same document.

### §4 Lifecycle (l.213–256)
13. **l.246–253 vs §7.5 l.869–872 vs §7.4 l.787–795 — headroom exposure mechanism
    (contradiction, and the only genuinely undecided item in the plan)**:
    - §4 l.246–248: "expose `display_hdr_info().tone_map_headroom()` **through
      `RenderState`**" — a distilled f32 on the global RenderState.
    - §7.5 l.869–872: "pass through the **whole `DisplayHdrInfo`** via `RenderState` …
      **— not a distilled headroom `f32`**" — the exact opposite granularity, same
      channel.
    - §7.4 l.790–795 + §5 l.363: the pass-through "goes in its own later PR, targeting
      the per-viewport channel (`ViewportInfo`)" — i.e., *neither* RenderState shape is
      the decided home.
    Entry 22 decided: mirror cut, read path = `frame.wgpu_render_state()`, per-viewport
    pass-through = later PR via ViewportInfo. What (if anything) ships *before* that
    later PR — `tone_map_headroom()` on RenderState, nothing, or the whole struct — is
    not pinned anywhere. The rewrite cannot proceed on this point without a decision.
14. **l.254–256 capture bullet**: content correct (silent no-screenshot failure; PR-3
    deliverable). Only the "(entry 34)" citation is noise.

### §5 Ladder & sequencing (l.259–376)
15. **l.270 — tier table row B "~Size: PR 1+2, ~300–450 lines" (stale PR numbering,
    contradiction with §1)**: "PR 1+2" is the 29c-era numbering (then PR 2 = HDR). After
    R2-6, tier B (surface creation, wide gamut + HDR) = **PR 1 + PR 3**; PR 2 is tier C.
    Additionally "~300–450 lines" is §1's *PR-1-only* figure (l.131), so the cell
    conflates one PR's size with two PRs' scope. R2-1's sweep ("§5 tier table B/C cells
    updated") evidently missed this cell.
16. **l.325 — "C+D + PR-1's pairing table already cover…" (stale)**: same defect as #5 —
    the primaries pairing table is PR 2's mechanism. Should read "PR 1's paired surface
    + C+D (PR 2/PR 5)".
17. **l.337 — section heading narrates restructure history** (see 1.2). The heading
    should be "Upstream sequencing"; the restructure rule (l.366–369) should become a
    plain design constraint ("Every PR ships consumed, testable functionality…") without
    "(user directive … binding)".
18. l.340–376 PR items: otherwise correct against the decided ladder (PR 1
    `DisplayP3`/u8/no-fp16; PR 2 tagged float content incl. WGSL sign fix + ecolor
    contract tests; PR 3 extended pair + fp16 readback + negative GPU round-trip; PR 4
    scRGB + encoding; PR 5 picker; side items; E deferred). `Primaries {Srgb,
    DisplayP3}` with Bt2020 dropped is correct per R2-6. Non-goals (l.370–376) match
    entries 2/7.
19. **l.645 cross-ref (in §7.2 but listed here because it mis-points into §5)**: see #23.

### §6 D vs E (l.380–434)
20. **l.432 — "and there's no clean D-tier story for text at all." (contradiction
    within the section)**: §6's own item 2 (l.400–412) was amended (entry 32) to say
    per-widget float-texture caching *is* viable for short labels and text has "no
    *general* mechanism". The closing paragraph still asserts the retracted absolute.
    One of the two must go — per entry 32, the closing paragraph is the survivor that
    should have been edited.

### §7 front matter (l.436–439)
21. **l.436–439 — "Open questions / not yet discussed" + "Threads surfaced but not
    resolved" (contradiction with all seven subsections)**: every subsection is
    DECIDED/RESOLVED/IN SCOPE. The genuinely open items are: whether `Auto` ever
    selects plain `DisplayP3` (l.733–735, deliberately open per round-2 entry 21); the
    PR-4 per-platform DX12 fallback question (l.698–700); web/wasm wiring (deferred,
    l.375–376); PQ/HLG transfer-carrying tag (l.901–902); OkLCh picker upgrade
    (l.569); tier E format controversy (§7.6). These fit in one short list.

### §7.1 Contract (l.441–569)
22. **l.446–452 — "tier-C test matrix gains a CPU↔GPU round-trip test with negative
    inputs" (stale PR assignment)**: R2-6 moved the *GPU* negative round-trip to PR 3
    ("with fp16 readback"); §5 l.353 gives PR 2 only ecolor contract property tests
    (CPU), and §5 l.357–359 gives PR 3 the "negative GPU round-trip" — consistent there
    and in §7.7 (l.976–977). §7.1's sentence predates R2-6 and contradicts both.
23. **l.565–568 — issue sentence survives the superseded non-goal (stale framing)**:
    "gamut conversion is app territory (v1 — wide-gamut output is an explicit
    follow-on, §7.2)". Entry 7 *rejected* the gamut deferral; §7.2 (l.571) explicitly
    "supersedes the 2026-09-12 non-goal". The sentence was patched ("marked v1-only")
    rather than rewritten, so it still reads as a deferral in the body of a section
    that says wide gamut is in scope. Rewrite to the §7.2-aligned statement (v1 storage
    contract is extended-sRGB/Bt.709; wide-gamut output ships via the staged pairing
    ladder; app-side conversion only for unpaired tags).
24. l.533–549 FloatImage sketch: shape correct per entries 28/29/29d/R2-6 (f16 pixels,
    `Primaries {Srgb, DisplayP3}`, encoded-space premultiply, `half` direct dep) — but
    **stale against upstream main** per #8714/R2-19; the shape-stale caveat lives only
    in §7.9 (l.1096–1100), not at the sketch. A §7.1-only reader takes
    `size: [usize; 2]` + `ImageData::Float(Arc<FloatImage>)` as pinned. The caveat must
    move to (or be repeated at) the sketch.
25. l.454–472 premultiply/filtering clarifications: correct per entries 19/R2-14; only
    the "(corrected — …)" and "(added 2026-10-25)" narration needs folding (see 1.2).
26. l.484–529 rationale points 1–6: numbers match entries 17/R2-12/R2-16/R2-17
    (branch-crossing; 2⁻¹¹ vs 2⁻¹⁰; 12.9× subnormal onset; formula-identical with the
    u8-bit-exact/fp16-½-code nuance). Content fine; narration to strip.

### §7.2 Gamut scope (l.571–659)
27. **l.644–646 — pairing rule attributed to PR 1 (tension/misattribution)**: "mismatched
    tags reject loudly at `update_texture` (see §5 PR 1)" — but §5 PR 1 ships no
    `update_texture` float path and no tags (that is PR 2, l.349–352). The *rule* is
    decided (R2-5) and the *enforcement* arrives with PR 2's tagged uploads; the plan
    should say so once. As written, §7.2 PR-1 item contradicts §5 PR-1 item ("No epaint
    changes, no new color types").
28. l.595–609 verified wgpu facts: match R2-4 (non-extended `DisplayP3`, Metal u8,
    web `webgpu.rs:4154-4160`, DX12 none). l.611 "consistent with our entry-2 'glow
    stays SDR' scoping" — log citation.
29. l.637 "re-staged 2026-10-25 … split ladder per 29b/29c" (see 1.3); the sketch's PR
    numbering itself (1–4 + later) is correct per R2-6.

### §7.3 Platform matrix (l.661–711)
30. **l.683 — "(2026-10-25; PR numbers per the 29c split)" (stale label)**: the 29c
    split numbered HDR as PR 2; the bullet then correctly uses final numbering (PR 2 =
    tier C, HDR pair = PR 3). The parenthetical label contradicts the bullet's own
    numbers. Delete the label; content per entries 10/R2-1/R2-18 is correct (DX12
    `EXTENDED_SRGB_LINEAR` only, `Rgb10a2Unorm`+`BT2100_PQ` HDR10 nuance present at
    l.699–702).
31. l.690–697 Linux/Wayland bullet: correct per entry 15/R2-1 (KWin scRGB under PR 4;
    P3 probe-gated SDR under PR 1). l.704–707 web: one line, correct.
32. l.666–678 manual protocol + windowed smoke: correct per entry 36's owners (via entry
    40) and §7.7's reconciliation; `MANUAL_TESTING.md` in PR 1 ✓ (l.131, l.673).

### §7.4 Reporting/fallback API (l.713–842)
33. **l.715–717 vs l.719–724 — stale `SdrReason` list (direct internal contradiction,
    four lines apart)**: intro paragraph says "reasons at least `Disabled`,
    **`NoExtendedSrgb`**, and `NoSurface`"; the pinned-payload paragraph immediately
    below states the decided closed set `Disabled`,
    `NoPairedColorSpace { requested: SurfaceColorSpace }`, `NoSurface` (R2-5). The
    first list is the pre-R2-5 survivor. Any rewrite that misses one of these two
    bakes the contradiction into the final doc.
34. **l.737–752 — the "three defects" block is correction history** (see 1.2). Its
    *conclusions* (create-time-immutable status; read via `frame.wgpu_render_state()`;
    per-viewport pass-through = later PR via ViewportInfo) each become one declarative
    sentence in the final reporting section.
35. l.775–778 / l.818 / l.822–827 — narration + duplication + tombstone (see 1.4, 1.6).
36. l.795–812 `decide_surface_target` contract: matches entries 25/26/R2-1 (named
    `SurfaceTarget` struct, `pref` param, `NoSurfaceFormatsAvailable`-only Err,
    migration note). The migration/"deprecate one release" sentence (l.803–806) remains
    deliberately under-priced per R2-8 (collision cost accepted, recorded log-only) —
    acceptable, but the final doc should not * oversell* the migration path; one clause
    noting the accepted #8703-collision cost would prevent misreading.
37. l.834–842 invariant test + PR-4 evolution: correct (incl. the
    `(Bgra8UnormSrgb, DisplayP3)` double-grammar fixture per R2-22) — except that §1
    claims a second, stronger assertion that does not exist here (see #4).

### §7.5 Calibration (l.844–902)
38. **l.869–872 — the "whole DisplayHdrInfo / not a distilled f32" bullet contradicts
    §4 l.246–248** (see #13 for the full triangle). Whichever exposure ships pre-
    ViewportInfo, exactly one statement must survive.
39. Rest of §7.5: correct per entries 14/21/R2-15/R2-16 (macOS-only headroom + TODO
    root-cause, 4-step precedence, 203-nit anchor, PQ ingest chain, HLG out). l.866
    "(entry 22 cut the `frame.info()` mirror…)" — log citation.

### §7.6 (l.904–916)
40. Correct per entry 31 (Eq cascade both cfg variants, serde repr, half/bytemuck). No
    staleness found. "(added 2026-10-25)" only.

### §7.7 Testing (l.918–991)
41. **l.928 — "not 'lies' as §4 claimed" (stale cross-reference)**: §4 l.254–256 now
    says "silently produces nothing (logs + returns)"; it no longer claims "lies". The
    correction's target was already fixed; the narration is dangling.
42. l.920–921 "The original text assumed… — false at 0.36.2 and HEAD, three ways":
    correction history (see 1.2); the three mechanical facts themselves are correct and
    verified (`texture_to_image.rs` 4 B/px `RgbaImage`; `lib.rs:267` stub ✓ HEAD;
    silent capture return).
43. Deliverables 1–4 and the test matrix: consistent with entries 34/35/R2-9/R2-11 and
    with the final ladder (fp16 cluster = PR 3; readback in egui-wgpu; clamp-to-white
    u8 in egui_kittest; MSAA plumbing wgpu.rs:202/217 ✓ HEAD-verified). l.943 "(no
    builder changes needed — corrected 2026-10-25)" narration.
44. **l.949–951 test-count survey**: "ecolor 14 inline tests, epaint 96, egui 102" —
    re-counted at audit: ecolor 14 ✓, epaint 96 ✓, egui **99** (drifted since entry 35;
    upstream-facing number, harmless but should be refreshed or dropped).

### §7.8 Interactions (l.993–1029)
45. **l.993–995 — header "to verify (not design)" vs body "Resolved 2026-10-25"**
    (contradiction): the items are resolved statements + manual checkboxes; the header
    still advertises open verification work.
46. **l.1022–1029 — "Missing from this list (added 2026-10-25…) (a)–(f)"**: patch note
    about the section's own completeness (see 1.6). Its content is six correct
    cross-references; fold them into the resolved list and delete the meta-block.
47. Items themselves: correct per entries 37/38/R2-9 (MSAA apple1x caveat, latency
    clamp, independent validation blocks, instance-extension inference tagged).

### §7.9 Process (l.1031–1101)
48. **l.1086–1091 — "§7.4's loud-break rationale **must be restated** (remaining break =
    caps parameter + richer return, not Result)" (stale directive)**: the restatement
    already happened (entry 5; §7.4 l.812–821 now cites the wgpu-30 bump + #8336 +
    #8366 precedent). The sentence reads as an outstanding TODO.
49. **l.1082, l.1087 — "213 commits since 0.36.2" / "213/16 commits diverged" (stale
    counts)**: re-counted at audit: `git rev-list --count 0.36.2..HEAD` = **216**
    (behind = 16 ✓). Also, since 0.36.2 is *not* an ancestor (the plan itself says so),
    "since" is the wrong preposition — "diverged 216/16" is the accurate phrasing.
50. l.1033–1060 engagement posture: consistent (total silence per R2-8 + entry 6; local
    #8703 review; fork-first per entry 3; PR 1 framed as follow-up to #8703 per R2-7;
    hooks PR 1→#2712, PR 3→#7484, PR 4→#6863 ✓).
51. l.1094–1100 timing: correct per R2-19 (0.37 shipped 2026-10-15, ladder 0.38+;
    #8714 shape-staleness). The FloatImage staleness note belongs at the §7.1 sketch
    too (see #24).

### Appendix (l.1103–1164)
52. **Anchor base is 0.36.2 while the tree is at HEAD — mixed anchor systems across the
    document.** Verified drift (0.36.2 number → HEAD number):
    | Anchor | Appendix/inline says | Actual at HEAD |
    |---|---|---|
    | `SurfaceConfig` | lib.rs:76-109 (l.1107) | lib.rs:**69** |
    | `RenderState` struct | :111-140 | :**105** |
    | `RenderState::create` | ~:289 (l.48, l.1111) | :**197** |
    | no-surface stub `vec![Rgba8Unorm]` | (§7.4/§7.7 say :267) | :**267** ✓ |
    | `SurfaceErrorAction` | :307-328 | :**306** |
    | `WgpuConfiguration` | :335-346 | :**326** |
    | `with_surface_config` | (§7.9 says :376) | :**376** ✓ |
    | `preferred_framebuffer_format` | :411-427 (l.51, l.1115) | :**421-436**, `Result`-returning (§7.9 correct) |
    | `configure_surface` | winit.rs:100-154 (l.53, l.1121) | winit.rs:**110** |
    | create-once guard | winit.rs:258 | :**258** ✓ |
    | vertex `array_stride: 5*4` | renderer.rs:378-382 | :**408** |
    | entry-point `is_srgb()` selection | renderer.rs:407-414 (l.140, l.1127) | :**435** |
    | texture creation `Rgba8Unorm` | renderer.rs:701-705 / ":702" (l.146, l.1129) | :**721-723** |
    | `register_native_texture` docs | :772-824 / ":772-821" | :**801/:851** |
    | `RendererOptions::dithering` default ON | renderer.rs:230 | :**230** ✓ |
    | WGSL negatives ÷/×12.92 | egui.wgsl:52-57 | :**46-47, 54-55** |
    | dither application | egui.wgsl:180 (l.162) | :**183/:198** |
    | ecolor clamp fns | :112-124/:126-131 | :**115/:130** |
    | ecolor unclamped fns | :157-179 | :**160/:172** ✓ (range ok) |
    | `ImageData` / `ColorImage` | epaint image.rs:16-18/:47-51 | :**16** / :**57** |
    | `Image::rotate` | image.rs:243-247 | :**243** ✓ |
    | kittest offscreen MSAA hardcode | wgpu.rs:202/217 | :**202/:217** ✓ |
    | `ViewportInfo` | viewport_info.rs:28 | :**28** ✓ |
    | `Frame::wgpu_render_state` | epi.rs:943 | :**943** ✓ |
    Pattern: everything cited during round 2 (entries 13/14/22/34/38, R2-1/R2-9) is
    HEAD-accurate; everything inherited from the 0.36.2 verification pass (Appendix +
    §1/§3 inline) is drifted 7–30 lines. The final document needs ONE base (HEAD, with
    the commit named) and one re-verification sweep (entry 4 already mandates this).

---

## 3. Structural plan for the final rewrite

### 3.1 Target outline (concrete; each section lists what folds into it)

```
# egui HDR enablement plan                          ← new title; drop "working notes"
   one-paragraph summary: goal, fork-first posture, ladder table (PR 1–5 + side + E),
   current upstream state (PR #8703 occupies brightness; we build the architecture) —
   distills l.19–23 + §5 l.337–368 into one table + three sentences.

## 1. Background and evidence base
   The app workaround (header l.4–18, compressed), #7484 (partial-recreate failure mode,
   from l.87–93), #8703 facts, kittest layout fact (header l.32–42, declarative).
   No check-history, no "corrected".

## 2. Surface policy (tier B → PR 1 + PR 3)
   Fold: §1 (l.46–134) + §7.4 (l.713–842). Canonical single statements:
   - policy enum `SurfaceColorSpacePreference {Off, Auto, DisplayP3, ExtendedSrgb,
     ExtendedDisplayP3}` (final shape; variants annotated with their PR),
     on `WgpuConfiguration`, create-time-only (one rationale, from l.62–89);
   - `decide_surface_target(caps: &SurfaceCapabilities, pref) -> Result<SurfaceTarget,
     WgpuError>`, `SurfaceTarget {format, color_space, status}`, RenderState stores the
     three outputs, configure_surface reads the stored field, invariant test (one list:
     incoherent triples incl. double-grammar pair + the built-SurfaceConfiguration
     assertion, resolving finding #4 either by adding the test or dropping the claim);
   - `HdrStatus` payload with the closed `SdrReason` (post-R2-5 set only — delete the
     l.715–717 list, resolving #33);
   - status is create-time-immutable; read via `frame.wgpu_render_state()` (one
     paragraph; the three-defect block becomes these three facts);
   - defaults (Off rollout / Auto endgame / fork unconditional — stated once);
   - extended-gamma-vs-scRGB rationale + permanent compositing cost (fold §1
     l.96–124 + §7.1 rationale pt 2; cost stated once);
   - sizing (~150 core / ~300–450 PR 1, corrected inventory without "pairing table");
   - migration note re #8703 (one clause; collision cost accepted per R2-8).

## 3. Output encoding and dithering (§2, fixed per #9/#14)
   OutputEncoding derivation + table (keep as-is), dither rules, PR-1 consequence
   restated against the final ladder (u8+DisplayP3 → GammaEncoded ⇒ no change in PR 1;
   encoding selection becomes load-bearing in PR 4).

## 4. Content model: the float color contract (§7.1 + §3, merged)
   Contract line, negative extension (odd-symmetric + WGSL fix owned by PR 2; GPU
   round-trip owned by PR 3 — per #22), premultiply/filtering rules, FloatImage shape
   (with the #8714 shape-stale caveat inline, per #24), tier-D picker spec, what §3's
   u8-ceiling motivation becomes as intro. "Self-contained, additive" (l.195) replaced
   by the priced statement from §5 tier C (resolves #12).

## 5. Wide gamut: the primaries-generic mechanism (§7.2)
   Decision, design rule (de-"user-directive"-ed), verified platform facts, the
   tag+pairing mechanism with enforcement point stated once ("loud reject at
   update_texture, enforced from PR 2 when tagged uploads exist" — resolves #27),
   demand-gated future list.

## 6. What tier D cannot do (§6)
   Keep; fix the closing text contradiction (#20); strip "(corrected/amended)" notes.

## 7. Lifecycle (§4)
   Runtime switching non-goal + full rationale (keep; it is the best rationale text in
   the doc), display transitions, capture note. Headroom exposure = one sentence
   pointing to §2's reporting model + the later per-viewport PR — after the #13
   decision is made.

## 8. Platform matrix and testing (§7.3 + §7.7, merged)
   Platform table (macOS tested; Windows/Linux experimental with the DX12/HDR10 and
   KWin nuances), MANUAL_TESTING.md + windowed smoke, then the test matrix
   (deliverables 1–4, conventions, tolerances, CI-vs-manual split). Drop the stale
   survey counts or refresh them (#44/#4 in §4 below).

## 9. Known interactions (§7.8)
   Resolved interactions as a checklist + the two manual checkboxes; fold (a)–(f) into
   the list; delete the "missing from this list" meta-block.

## 10. Upstream strategy and process (§7.9)
   Total-silence decision, fork-first + [patch] workflow, #8703 posture + comparison
   (keep (e) — it is issue-draft material), prepared issue draft (hooks + objections),
   re-anchor protocol, timing (0.38+), #8714 drift note.

## 11. Tier E outlook (§5 tier E row + E bullets + §7.6)
   Blast radius, format controversy, pre-E type decisions, who-needs-E list.

## Appendix A — source anchors
   Re-verified against the working tree at the named HEAD commit (one base; the table
   in §2.52 above is the starting diff). Registry cites keep their explicit
   `wgpu-{types,hal,core}-30.0.1` paths.

## Appendix B — design history (optional, ≤10 lines)
   One paragraph: "This plan went through two review rounds on 2026-10-25 (~60
   decisions); the full decision log lives in reviews/00-consolidated.md §Decision
   log." Nothing per-decision; no entry IDs in the body.
```

### 3.2 Folding rules for the layered passages
- **§7.4's contract+clarifications+corrections stack** → §2 above: delete "(renamed
  from…)", "(replaces the 'keeps its name' bullet)", the ~~preferred_color_space~~
  tombstone, the duplicate decider bullet; one bullet per fact.
- **§7.4's three-defect block** → three declarative sentences (create-time-immutable;
  read path; per-viewport later PR). The *defects* disappear; a "design history"
  pointer can carry them if wanted.
- **§7.2's decision+framing+rule stack** → §5 above: decision paragraph, then the
  binding rule as a normal rule ("Nothing in the mechanism may name or branch on a
  specific primaries…"), then facts, then sketch. Delete "(supersedes the 2026-09-12
  non-goal)" and "re-staged … per 29b/29c".
- **Every "(corrected/added/… 2026-10-25)" and every "entry N" citation** → delete;
  where the correction carried a *constraint* (e.g. "formula-identical, bit-exact only
  on u8"), the corrected text already embodies it — verify each of the 44 against this
  report's §2 before deleting.
- **History**: decision log stays external (reviews/00-consolidated.md); at most
  Appendix B's pointer. No per-decision history in the plan body.
- **Appendix anchors**: single base = working-tree HEAD (name the commit + date),
  re-swept per entry 4; keep registry cites explicitly versioned. Do not keep two
  anchor systems.

### 3.3 Sentence-level style for the rewrite
Present tense, no audit trail: "PR #8703 (open since 2026-10-12) occupies tier B with
`prefer_hdr: bool` on the runtime-mutable `SurfaceConfig`; this plan's PR 1 is
positioned as its follow-up" — not "the 2026-09-11 check found nothing, but as of…".

### 3.4 Missing pieces for a never-saw-the-rounds reader
- The ladder table up front (currently only at §5).
- A one-screen "Architecture at a glance": surface policy → decider → status →
  encoding → content contract → picker (five boxes, one line each) — all material
  exists, nothing assembles it.
- An explicit open-questions list (see 3.5).
- The term "pairing" used in exactly one sense (content-tag ↔ surface pairing) with the
  enforcement point named (resolves the #5/#16/#27 cluster).

### 3.5 Residual genuinely-open items (the new "Open questions" section)
1. Does `Auto` ever select plain `DisplayP3` (turning on the chrome shift for the
   endgame default)? — deliberately open, l.733–735.
2. Headroom exposure before the per-viewport pass-through lands: `tone_map_headroom()`
   on `RenderState`, whole `DisplayHdrInfo`, or nothing? — must be decided (finding
   #13); it is the only undecided *decision* in the document.
3. PR-4: per-platform DX12 fallback inside `decide_surface_target`
   (`ExtendedSrgb`→`ExtendedSrgbLinear`)? — l.698–700.
4. Web/wasm wiring (deferred for scope) — l.375–376.
5. Transfer-carrying tags (PQ/HLG), OkLCh picker geometry, tier E vertex format —
   future work, already framed as such.

---

## 4. Claims needing final verification

Load-bearing claims that are unverified, stale-numbered, or inferred. The cleanup pass
should check or hedge each:

1. **l.243 [INFERENCE]** — "resize-driven reconfigure carries the stored color space":
   derived from config semantics, never executed. The plan already routes it to the
   macOS manual pass; keep as an explicit manual-check item.
2. **l.702 [INFERENCE]** — Linux desktop Vulkan drivers "essentially never" expose P3
   swapchain spaces: ecosystem claim, no driver survey. Hedge ("driver-dependent; not
   observed in the wild") or cite a survey.
3. **l.1014 [INFERENCE]** — compositor behavior of premultiplied >1.0 content "fine on
   macOS": never tested. Manual checkbox (plan already says so).
4. **l.1019–1021 [INFERENCE]** — user-supplied instance without
   `VK_EXT_swapchain_colorspace` → correct SDR fallback: derived from
   `instance.rs:280-282` + probe-from-caps, not run.
5. **§1 l.105 "The §7.4 invariant test asserts the *built* `SurfaceConfiguration`…"** —
   no such assertion exists in §7.4 (finding #4). Decide: add the test to the §7.4
   list, or reword §1.
6. **"neutrals 0.0 ΔE, accents ≤6.2 ΔE"** (l.345, l.641) — the quantified chrome-shift
   figures have no derivation recorded anywhere in the plan or log (which palette?
   which ΔE formula?). Load-bearing for PR 1's opt-in story; verify or present as
   representative-estimate.
7. **"~2.12× brightness error" for sRGB-inverse PQ decode** (l.895) — arithmetic
   stated in R2-15 without shown work; recompute once (BT.2100 constants vs sRGB
   EOTF at reference white) before it anchors the PQ ingest text.
8. **git divergence "213/16"** (l.1082, l.1087) — actual 216/16 at audit time; the
   number will keep drifting, so the final doc should either name the measured commit
   or drop the count.
9. **Test-count survey** (l.949–951) — egui inline tests now 99, not 102 (ecolor 14 ✓,
   epaint 96 ✓). Refresh or replace with "per-crate inline-test conventions" without
   counts.
10. **"0.37 shipped 2026-10-15 (#8725)"** (l.1094) — upstream release fact taken from
    the review round; re-verify against the upstream changelog before the rewrite
    freezes it.
11. **#8703 facts** (created 2026-10-12, +105/-capture, Linux-tested) — verified on
    2026-10-25 via the live PR; the plan already mandates a dated re-verification
    before drafting (l.1046–1047). Keep that mandate; do not freeze the numbers into
    prose that outlives the PR's state.
12. **"the macOS runner has a window server"** (l.677–678) — CI-infrastructure claim
    underpinning the windowed smoke test; confirm against the actual runner image /
    a smoke run before relying on it in PR 1.
13. **"wgpu never writes HDR metadata"** (l.708–710) — relayed from a wgpu doc comment
    (`wgpu-types-30.0.1/src/surface.rs:569`); true of the doc, and the only
    `SetHDRMetaData`/`vkSetHdrMetadataEXT` occurrence in wgpu 30.0.1 sources is that
    comment (verified). Fine to keep, but cite it as wgpu-documented, not as an
    independent audit.
14. **Metal "reports `EXTENDED_SRGB`/`EXTENDED_DISPLAY_P3` unconditionally"** claims
    (l.1012, l.604–607) — carry the upstream OS-version TODO caveat (already noted at
    l.606, R2-22); make sure the final doc keeps the caveat wherever the
    "unconditional" phrasing appears.
15. **FloatImage shape vs post-#8714 main** — constructor-based API re-verification is
    already scheduled "when PR 2 starts" (l.1098–1100); the final doc should carry the
    caveat *at the sketch* (finding #24) so the shape is never read as pinned.

---

## Verdict

**Not ready to rewrite as-is — but nothing here requires new research, only decisions
and discipline.** The content is decision-complete against the log (the ladder,
payloads, contract, platform facts, and testing story all match the decided record);
what stands between this file and a final plan is (a) two documents interleaved — the
pre-review narrative and the post-review decisions — with 44 dated annotations, ~30
log-ID citations, and a §7 that is titled "Open questions" while holding the decided
core, and (b) a short list of surviving contradictions (§2 findings #4, #5/#16/#27,
#12, #20, #22, #33, #15) that a linear rewrite could easily preserve by copying.

**Single biggest blocker: finding #13 — the headroom/`DisplayHdrInfo` exposure
mechanism is stated three incompatible ways (§4 vs §7.5 vs §7.4) and is the only
genuinely undecided *decision* in the document.** Everything else is editorial or
mechanical (strip annotations, resolve the stale PR-number/anchor survivors via the
§2.52 table, fold §7 into the body per §3). Settle #13 first; the §3.1 outline is then
executable without further design decisions.

Side note for the log owner: the decision log cites "entry 36" (in entry 40) but has
no row 36 — numbering gap only; the plan does not depend on it.
