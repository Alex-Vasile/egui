# 08 — Coherence audit: hdr-egui-upstream.md (post round 3)

Question: does the document read as one plan designed from the start, or as an initial
plan with review-round decisions layered on? Method: full read of all 743 lines;
every suspect below checked against both passages; load-bearing claims verified in the
working tree (`hdr_enablement` branch) and registry sources.

**Source facts verified** (cited where used below): `SurfaceCapabilities.format_capabilities`
exists, and `formats` excludes explicit color spaces (wgpu-types 30.0.1 `surface.rs:506-521`);
`Auto` resolves to `ExtendedSrgbLinear` for fp16 (`surface.rs:243-268`); `SurfaceColorSpace`
variants are CamelCase, the `SurfaceColorSpaces` bitflags SCREAMING (`surface.rs:242-414`,
`468-484`); `DisplayP3` is supported on Vulkan where the driver exposes it (`surface.rs:320-321`);
`tone_map_headroom` precedence incl. `high_dynamic_range == Some(false) → Some(1.0)`
(`surface.rs:732-746`). egui-wgpu: `configure_surface` exists (winit.rs:112) and builds
`SurfaceConfiguration` from `get_default_config` (winit.rs:134-152), so `color_space` is
implicitly `Auto` — the token `color_space` appears nowhere in `crates/egui-wgpu/src`;
entry-point sniff `is_srgb()` → `fs_main_linear_framebuffer` + the stale #6863 warning,
else `fs_main_gamma_framebuffer` (renderer.rs:435-439); dithering default true
(renderer.rs:230) with doc "Dithering assumes an sRGB output" (renderer.rs:195);
`bytes_per_row: Some(4 * width)` (renderer.rs:671); texture creation hardcoded
`Rgba8Unorm` (renderer.rs:721-723); capture `_ =>` logs and returns (capture.rs:203-211);
no-surface stub `vec![Rgba8Unorm]` (lib.rs:267). egui gamma house style (lib.rs:147);
egui.wgsl gamma fns use `select()` and evaluate `pow(negative, …)` in the discarded lane
(egui.wgsl:53-59); `ViewportInfo` at data/input/viewport_info.rs. `reviews3/00-consolidated.md`
exists ("Six fresh personas, cold read", 30 entries; plan was 541 lines at round-3
dispatch; this file is now 743).

---

## 1. Reading-experience defects

Line numbers refer to `hdr-egui-upstream.md` (743 lines).

1. **Tier letters A/B are never defined; C/D/E are load-bearing.** The ladder uses
   "tier C" (L36), "Tier D" (L39), "tier E" (L41); §6 is titled around tiers D/E (L434);
   §11 is "Tier E outlook" (L683); §10 promises "per-tier tests" (L662). Nowhere does
   the document say what tiers A and B are, or map tiers to PRs. A first-time reader
   must reverse-engineer that tier D = PR 5 and tier E = the deferred row.
2. **Four names for the apps, roles never disambiguated.** "the workaround app"
   (`../hdr-test/rust`, L50-51), "the demo app" (L446), "the upstreamed test app"
   (L575), "the app"/verification vehicle (§10, ~L669), "the example" (L259),
   `examples/hdr_smoke.rs` (L584), ladder "Side items: docs, test app" (L40). Are the
   demo app and the upstreamed test app the same program? Is the §10 verification
   vehicle the §8 protocol instrument? The reader cannot tell, and D3 below depends on
   the answer.
3. **`hdr_text` is promised with no home.** L445-447: "The `hdr_text` helper is the
   first thing to prototype for the demo app's nits readout — it either validates
   deferring E or becomes the evidence E is needed." It appears in no ladder row, no
   side item, no deliverable list (L34-41, L619-633). A promise the plan never schedules.
4. **The 2a/2b split was never propagated into §4, §8, or §10.** §4 is still titled
   "the float color contract (PR 2)" (L299) and says "PR 2 adds the float path" (L303)
   and "PR 2 ships the `sign(x)·f(|x|)` fix" (L316-317); §8 says "PR 1/PR 2 are
   unaffected" (L635). The ladder (L35-36) ships 2a and 2b with different content, so
   the reader must guess which "PR 2" statement belongs to which half. §10's anchor
   list (L660-661) covers PR 1/3/4 only — 2a, 2b, and 5 have no stated anchors.
5. **Two divergent future-work lists.** §5 "Demand-gated future work" (L429-432):
   `Rgba`-level primaries tagging (#2712 proper), pairwise conversion matrices,
   per-texture content conversion, color-managed textures. Open questions item 2
   (L699-701): transfer-carrying tags, OkLCh picker geometry, tier E vertex format,
   per-texture content conversion, runtime preference changes. Three §5 items are
   missing from Open questions; "OkLCh picker geometry" (L699) appears nowhere else in
   the document and is never explained.
6. **Malformed cross-reference.** L549: "defers as a whole to the transfer-carrying-tags
   work (§ Open questions)" — the Open questions section has no number; "§ Open
   questions" is not a reference a reader can resolve.
7. **§7 heading advertises a topic the section drops.** "Lifecycle: switching,
   transitions, headroom" (L458) — "transitions" occurs nowhere else in the document
   (single hit: the heading). Leftover from an earlier draft's scope.
8. **Decision-log archaeology in the body.** "the uncaptured-error/panic path of the
   one-target model is gone by construction" (L181-182) — the one-target model is never
   described; "`Disabled` dropped: it existed only to shove `Off` into the old enum"
   (L232-233) — neither `Disabled` nor "the old enum" exists in this document; "Two
   independent bools (`want_hdr`, `want_wide_gamut`) were rejected" (L118-119) — the
   bools never existed here. These are citations to prior drafts the reader never saw,
   and Appendix B does not contain the referenced history either.
9. **The decider's result is written in three shapes.** The probe snippet returns
   `Granted(cap.format, cs)` / `FellBack { NoPairedColorSpace }` (L138-143); the
   signature returns `Result<(SurfaceTarget, SurfaceStatus), WgpuError>` (L159-163);
   the struct block defines `SurfaceTarget`/`SurfaceStatus` with
   `NoPairedColorSpace { requested: SurfacePreference }` (L207-229) — a payload the
   snippet's variant lacks. Same section, three grammars; the reader must reconcile
   pseudocode with spec unaided.
10. **PR-tagging is inconsistent at summary level.** The glance tags headroom with its
    PR ("+ `hdr_headroom`, PR 3", L18) but states `OutputEncoding::of(format,
    color_space)` untagged (L21-22) although §3 says PR 1 ships none of it and the
    color-space input arrives only in PR 4 (L270-273, L286-288). §2's heading scopes it
    "(PR 1 + PR 3)" (L84) while the section carries PR 4 material (L236-249, L254,
    L263-265). The glance mixes per-PR and end-state statements with no rule.
11. **Two naming conventions for the same wgpu values.** SCREAMING bitflag names used as
    if they were the enum: `EXTENDED_SRGB_LINEAR` (L67), `EXTENDED_SRGB`,
    `EXTENDED_DISPLAY_P3` (L410), `EXTENDED_SRGB_LINEAR` (L415) — versus the CamelCase
    variants the plan's own code uses (`ExtendedSrgb` L56, L242; table L283-284).
    Source: variants vs bitflags are distinct types (wgpu-types surface.rs:242-414 vs
    468-484).
12. **"Pairing" means two different tables.** The decider walks a "priority-ordered
    pairing table" of (format, color space) (L130-131); §4/§5 build a "pairing table"
    over (content tag, granted surface space) (L367-369, L393-394). L146-147 uses both
    in one sentence: "Concrete primaries exist only in these table rows and the
    content-tag pairings". Nothing flags the homonym.
13. **"The probe" antecedent is 500 lines away and shifts sense.** Defined as a nickname
    for the decider's table walk (L130); §9 uses "the probe reads the compatible
    surface's caps" (L648-650) — related but a different act (caps read vs walk), with
    only the bolded nickname as the bridge.
14. **"Our fork opts in unconditionally" (L155) never names the variant.** The reader
    can only infer the fork's chosen preference from the smoke example's
    `run_native` with `WideGamut` (L587).
15. **Two similarly-named negative tests, distinction never drawn.** Ladder 2a:
    "negative CPU↔GPU round-trip" (L35); ladder 3: "negative GPU round-trip" (L37).
    §4 defines only one test (L317-318). Same test twice under two names, or two
    tests? Undecidable from the text.
16. **Appendix A's wgpu bullet groups four crates under one header.** "✓ wgpu-types
    30.0.1: `Auto` resolution (core surface_config.rs:24-40) … DX12 mapping (hal
    dx12/mod.rs:1455-1466); GLES … (hal gles/adapter.rs:1347); Vulkan … (hal
    vulkan/instance.rs:280-282); web P3 (webgpu.rs)" (L724-730). The paths are real
    (verified: wgpu-core-30.0.1/src/device/surface_config.rs;
    wgpu-hal-30.0.1/src/dx12/mod.rs) but they are not in wgpu-types; as written the
    anchors do not resolve where the header says they do.
17. **A palette-specific number is borrowed for textures.** L381: "the §5 re-encode
    (≤0.21 ΔE) is the escape hatch for either" — the ≤0.21 figure is defined in §5 for
    the u8 *style palette* re-encode (L422-426); §4 extends it to texture re-encodes
    ("for either") without support.
18. **Near-duplicate statements with drifted wording.** #8703 migration cost stated
    twice: §2 L263-265 ("field relocation off `SurfaceConfig`, encoding supersession
    for their adopters, `capture.rs` conflicts") vs §10 L664-668 ("field relocation,
    encoding supersession for its adopters, `capture.rs` conflicts"). The
    viewport-creation/monitor-change decision stated twice in §2 (L166-167 vs L203-204).
    The "enum variant + pairing row + conversion matrix, in one PR" rule appears three
    times (L43-46, L351-352, L404-405).
19. **Decision-status voice leaks into the body.** "**Exposure (decided; ships in PR
    3)**" (L487) — decided when, by whom, recorded where? Also "Three distinct
    mechanisms, decided separately" (L460) and "MSAA on fp16 — resolved:" (L639).
    Reads as minutes, not plan.
20. **PR 4's platform scoping disagrees across sections.** Ladder consumer cell:
    "Windows HDR + wide gamut" (L38); §8 gives Linux the same pairing ("KWin works with
    scRGB today", L568-569) and §10 cites "Linux by #8703's author" (L663).
21. **Two unreconciled "headless" mechanisms.** §2: headless ⇒ `FellBack { NoSurface }`
    status, never configurable (L182-183). §8 deliverable 2: "Headless format override:
    the no-surface stub consults an explicit format (doubles as the power-user escape
    hatch)" (L626-629). Same word, two layers (decider path vs `RenderState` stub,
    lib.rs:267), and the document never says how they relate or which one kittest
    exercises.

## 2. Contradictions

Each item cites both passages; source facts where relevant.

1. **Pipeline-cache cardinality: "one entry per format" vs "two entries per
   preference".** L201: "the pipeline-cache lookup resolves each granted format to the
   correct pipeline **(one entry per format through PR 3)**". L254-256: "the renderer's
   pipeline cache holds **two entries per preference, keyed by format** — the renderer's
   only change." A cache keyed by format cannot also be keyed per preference; through
   PR 3 there are exactly two formats (L252-253), i.e. two entries total. "Per
   preference" is the pre-status-split design speaking; the same paragraph that says
   "keyed by format" contradicts its own count.
2. **PR 4's Windows wide-gamut promise vs §4's upload-pairing table.** L536-538: "The
   same `(Rgba16Float, ExtendedSrgbLinear)` pairing therefore delivers Windows HDR
   brightness **and** wide gamut: P3/BT.2020-referred content converts via the 3×3
   matrix in linear light — emitting negatives, which the §4 odd-symmetric contract
   covers". But L376: "| `DisplayP3` (explicit) | sRGB / extended-sRGB / **scRGB** |
   **loud reject** |" and L377: "| BT.2020 (future) | anything non-BT.2020 | loud
   reject until conversion matrices land |". The conversion §7 describes is exactly the
   "pairwise primaries↔primaries conversion matrices" listed under "**Demand-gated
   future work**" (L429-431), and §7 itself says "v1's content model has no BT.2020
   pairing — so the ingest chain defers as a whole" (L546-548) four paragraphs later.
   Under the table as written, no v1 content tag can deliver wide gamut on the PR 4
   surface, so the ladder's consumer cell "Windows HDR + wide gamut" (L38) and L540-541
   ("Windows wide gamut arrives only through this fp16-linear pairing") promise a
   content path the plan explicitly does not ship. This is the corrected Windows scRGB
   story layered in without re-deriving §4's table.
3. **Per-texture content conversion: available escape hatch vs demand-gated future
   work.** L427-428: "Per-texture content conversion is the opting-in app's escape
   hatch for existing sRGB-referred images." L429-432 lists "per-texture content
   conversion" under "**Demand-gated future work**"; L700 repeats it in Open questions'
   future-work list; L381 calls the §5 re-encode "the escape hatch for either". The
   intended reconciliation (an app-side recipe exists in v1; the egui-side feature is
   future work) is never stated — the same noun is both shipped and deferred.
4. **"Storage transfer fixed to gamma" vs the PR 4 linear pairing.** L403-404: "egui
   tags primaries and **fixes storage transfer to gamma**; PQ/HLG decode app-side
   (§7)." But §2 ships linear storage: "linear values on an `ExtendedSrgbLinear`
   surface" (L242-243), encoding table "fp16 + `ExtendedSrgbLinear` → `Linear`"
   (L284), and §7's PR 4 pairing is exactly that surface (L536). §5's absolute claim
   has no carve-out for the pairing family the same plan ships in PR 4.
5. **The f16-decode code has two different named donors.** §7 L501-503: "The fp16
   branch is a PR 3 deliverable (§8), **sharing the f16 decode code with the test
   readback**." §8 L622-623: "Float readback in **egui-wgpu**, next to `capture.rs`
   (**sharing #8703's f16-decode/padding code**)." §10 makes #8703's merge optional:
   "If it stalls, PR 1 stands alone. We do not interact with that work" (~L667-669;
   #8703 "blocked since 2026-10-12", L664). If #8703 stalls — an explicitly accepted
   scenario — there is no #8703 code to share; only §7's version (share with the
   in-repo test readback) survives every branch. The plan states both as the
   mechanism.
6. **§1's format-change law vs the plan's never-recreate law.** L58-59 (§1, listed as a
   *validated* constraint): "no runtime HDR toggle (**a format change requires
   recreating `egui::Context` + `egui_winit::State` + `egui_wgpu::Renderer`
   together**)." L106-107 (§2): "What the per-viewport machinery deliberately **never
   does is recreate the renderer or context**"; L171-173: "A monitor change that alters
   the capability re-configures that viewport's surface and swaps the pipeline; **no
   renderer or context is ever recreated** … by construction"; §7 L464-466 repeats it.
   Nothing marks §1's rule as superseded by the re-decide path; worse, L111-112 frames
   recreation as optional ("Full recreate remains an app-level **technique** (the
   workaround app does exactly this)") where §1 framed it as *required*. A maintainer
   reading top-to-bottom meets two opposite laws about format changes with no bridge
   sentence.
7. **§9 still assumes one stored decision.** L642-644: "`present_with_transaction` …
   reconfigure rebuilds from **the stored decision**." The round-3 model stores a
   decision per viewport: "stores the resulting `(SurfaceTarget, SurfaceStatus)` in
   **that viewport's state**" (L166-169), "per-viewport, decided at viewport creation
   and on monitor change" (L203-204), monitor re-decide per viewport (L462-466).
   Singular "the stored decision" is pre-per-viewport wording (cf. defect 8's
   "one-target model", L182).
8. **Appendix B's history is incomplete and mischaracterizes round 3.** L738-739: "a
   third user-walked round on 2026-10-26 (~90 decisions: **six specialist reviewers per
   round**, every finding decided and applied)". L740-742: "The **complete** decision
   logs … live in `reviews/00-consolidated.md` and `reviews2/00-consolidated.md`."
   Verified: `reviews3/00-consolidated.md` exists ("Six fresh personas, cold read",
   30 consolidated entries) and is uncited; "six specialist reviewers per round" does
   not describe the user-walked round it sits in the same sentence with. The one
   paragraph added for round 3 was grafted onto round-2 text without reconciling it.
9. **Headroom's PR gate is stated once and then dropped.** L18: "(+ `hdr_headroom`,
   **PR 3**)"; ladder PR 3 (L37) and §7 ("ships in PR 3", L487, L492) agree. But §2
   presents headroom as shipping with PR 1's plumbing twice, unqualified: "surfaces
   both through per-viewport `ViewportInfo` **alongside `hdr_headroom`** (§7)" (L170)
   and "surfaced to apps through `ViewportInfo` (**next to `hdr_headroom`**, §7)"
   (L203-204). A reader of §2 alone concludes headroom is a PR 1 deliverable.
10. **The binding no-primaries rule forbids what the plan's own tables do.** L396-399:
    "**Design rule (binding): no specific color space is privileged — nothing in the
    architecture may name or branch on a specific primaries.**" The decider walks a
    table whose one entry is `DISPLAY_P3` (L134-137, L147-148); the upload table
    branches explicitly on `DisplayP3` and BT.2020 (L371-377); the flagship pairing is
    named `Bgra8Unorm + DisplayP3` (L190). L400-401 ("concrete primaries live only in
    pairing-table rows") states the intended carve-out, but the bolded binding sentence
    as written prohibits the tables it then relies on.
11. **§2's platform census omits Vulkan; §5 includes it.** L147-148: "Today the walk
    has one entry on Metal and web (`DISPLAY_P3`) and none on DX12/GLES." L416-417:
    "Vulkan is driver-dependent via `VK_EXT_swapchain_colorspace`." Source:
    wgpu-types 30.0.1 surface.rs:320-321 — `DisplayP3` "Supported on: Vulkan (where the
    driver exposes it), Metal, and the browser WebGPU backend." A Vulkan driver
    reporting P3 today invalidates "one entry on Metal and web" as a full census; the
    two sections state incompatible inventories of the same walk.

## 3. Fix list

Ordered by severity; "Lnn" = current text location. Old → new where practical.

1. **(C2, D20) Re-scope PR 4's wide-gamut claim to match §4's table.**
   - L536-538, old: "The same `(Rgba16Float, ExtendedSrgbLinear)` pairing therefore
     delivers Windows HDR brightness **and** wide gamut: P3/BT.2020-referred content
     converts via the 3×3 matrix in linear light — emitting negatives, which the §4
     odd-symmetric contract covers —"
     new: "The same `(Rgba16Float, ExtendedSrgbLinear)` pairing therefore delivers
     Windows HDR brightness and makes the surface wide-gamut-*capable* (signed values,
     gamut-unbounded). Displaying P3/BT.2020-tagged content on it would convert via the
     3×3 matrix in linear light — emitting negatives, which the §4 odd-symmetric
     contract covers — but that conversion leg ships with the §5 matrices work; until
     then §4's loud-reject table governs."
   - L38 consumer cell, old: "Windows HDR + wide gamut" → new: "Windows HDR
     (wide-gamut-capable surface; content conversion lands with §5's matrices)".
   - L540-541, old: "Windows wide gamut arrives only through this fp16-linear pairing"
     → new: "Windows wide gamut can only ever arrive through this fp16-linear pairing;
     in v1 the surface is capable but no content tag drives it (§4 table)."
2. **(C1) Fix the pipeline-cache count.** L255, old: "so the renderer's pipeline cache
   holds two entries per preference, keyed by format — the renderer's only change."
   new: "so the renderer's pipeline cache holds one entry per format — two through PR 3
   — keyed by format only; the renderer's only change."
3. **(C3, D17) Separate the escape hatch from the future work.** L427-428, old:
   "Per-texture content conversion is the opting-in app's escape hatch for existing
   sRGB-referred images." new: "App-side per-texture re-encoding is the opting-in
   app's escape hatch for existing sRGB-referred images — a recipe, not an egui
   feature; egui-side per-texture conversion is demand-gated future work (below)."
   L431: rename the list item "egui-side per-texture content conversion". L381, old:
   "the §5 re-encode (≤0.21 ΔE) is the escape hatch for either" → new: "app-side
   re-encoding is the escape hatch for either (palette-quality ≤0.21 ΔE per §5's
   palette measurement)".
4. **(C4) Carve PR 4 out of §5's gamma claim.** L403-404, old: "egui tags primaries and
   fixes storage transfer to gamma; PQ/HLG decode app-side (§7)." new: "egui tags
   primaries; storage transfer is gamma on every extended-gamma pairing and linear on
   the PR 4 scRGB pairing (§2); PQ/HLG decode app-side (§7)."
5. **(C5) Give the f16-decode one donor.** L622, old: "(sharing #8703's
   f16-decode/padding code)" → new: "(sharing the f16-decode/padding code with the §8
   test readback; the same code #8703 carries — adopt theirs if it merges first, per
   §10)". L501-503 then stays as the canonical statement.
6. **(C6) Bridge §1 to the re-decide model.** After L59 append: "Those constraints are
   the workaround app's, which owns the whole render loop and predates the per-viewport
   decider; the plan's reconfigure path (§2) changes format with no recreation, which
   is what retires the recreate-together rule."
7. **(C7) Per-viewport the stored decision.** L643-644, old: "reconfigure rebuilds from
   the stored decision." → new: "reconfigure rebuilds from the viewport's stored
   `(SurfaceTarget, SurfaceStatus)` (§2)."
8. **(C8) Complete Appendix B.** L740-742, old: "live in `reviews/00-consolidated.md`
   and `reviews2/00-consolidated.md`" → new: "live in `reviews/00-consolidated.md`,
   `reviews2/00-consolidated.md`, and `reviews3/00-consolidated.md`". L738-739, old:
   "(~90 decisions: six specialist reviewers per round, every finding decided and
   applied)" → new: "(~90 decisions: rounds 1–2 six specialist reviewers each; round 3
   six fresh personas on a cold read, decisions taken in place)".
9. **(C9) Gate headroom at both §2 mentions.** L170: "alongside `hdr_headroom` (§7)" →
   "alongside `hdr_headroom`, which follows in PR 3 (§7)". L204: "(next to
   `hdr_headroom`, §7)" → "(next to `hdr_headroom`, PR 3, §7)".
10. **(C10) Qualify the binding rule.** L396-399, old: "nothing in the architecture may
    name or branch on a specific primaries." → new: "outside pairing-table data,
    nothing in the architecture may name or branch on a specific primaries."
11. **(C11) Complete the census.** L147-148, old: "Today the walk has one entry on Metal
    and web (`DISPLAY_P3`) and none on DX12/GLES;" → new: "Today the walk has one entry
    on Metal and web (`DISPLAY_P3`), driver-dependent entries on Vulkan, and none on
    DX12/GLES;"
12. **(D4) Sweep PR 2 → 2a/2b.** L299 heading → "## 4. Content model: the float color
    contract (PR 2a + 2b)"; L303 → "PR 2a/2b add the float path."; L316-317 → "PR 2a
    ships the `sign(x)·f(|x|)` fix …"; L635 → "PR 1/PR 2a/2b are unaffected." §10
    L660-661: state anchors for 2a/2b/5 or mark them "no upstream anchor yet".
13. **(D1) Define the tier letters once** (ladder, after L41), or replace tier letters
    with PR numbers throughout (L36, L39, L41, §6 title, §11 title, L662). Tier A/B
    definitions must come from the review logs; they exist nowhere in this document.
14. **(D2, D3) Name the apps once and give `hdr_text` a home.** Add to §1 or §8: the
    workaround app (`../hdr-test/rust`, replaced); the demo app (the fork-side
    verification vehicle of §10, hosts the `hdr_text` prototype of §6); the upstreamed
    test app (the §8 protocol instrument shipped under "Side items", L40). Then either
    add `hdr_text` to the L40 side-items cell or state explicitly that it is
    demo-app-local and never upstreamed.
15. **(D5) Merge the future-work lists.** Open questions item 2 (L699-701) → add
    `Rgba`-level primaries tagging (#2712 proper), pairwise conversion matrices,
    color-managed textures; keep §5 L429-432 as the detail and point Open questions at
    it. Define or delete "OkLCh picker geometry" (L699).
16. **(D6)** L549: "(§ Open questions)" → "(see Open questions)".
17. **(D7)** L458 heading → "## 7. Lifecycle: switching, headroom".
18. **(D8) Retire the archaeology.** L181-182, old: "the uncaptured-error/panic path of
    the one-target model is gone by construction" → new: "the uncaptured-error/panic
    path of a single globally-stored target is gone by construction". L232-233, old:
    "`Disabled` dropped: it existed only to shove `Off` into the old enum — Off is
    simply `Granted`." → new: "Off is simply `Granted`." L118-119, old: "Two
    independent bools (`want_hdr`, `want_wide_gamut`) were rejected:" → new: "Two
    independent bools (`want_hdr`, `want_wide_gamut`) would give the same four
    combinations but leave the fallback join implicit:".
19. **(D9) One result grammar.** Either annotate the L133-143 snippet "// shape
    abbreviated; see Status + target below", or rewrite it to return
    `Ok((SurfaceTarget, SurfaceStatus))` with the `requested` payload so it matches
    L159-163 and L207-229.
20. **(D10)** L21-22 → "- **Encoding** — `OutputEncoding::of(format, color_space)`,
    derived at `Renderer::new`, never stored (derivation lands in PR 4; PR 1 keeps
    today's `is_srgb()` sniff)." L84 heading → "## 2. Surface policy (PR 1 + PR 3;
    PR 4 encoding/migration groundwork)".
21. **(D11) Standardize on variant names.** L67 → "`ExtendedSrgbLinear`"; L410 →
    "(`ExtendedSrgb`, `ExtendedDisplayP3`)"; L415 → "`ExtendedSrgbLinear` (scRGB)".
    (The SCREAMING names are `SurfaceColorSpaces` bitflags, a different type.)
22. **(D12) Name the tables apart.** L130-131 → "priority-ordered surface-pairing
    table"; L368 and L394 → "content-tag pairing table"; L146-147 → "Concrete primaries
    exist only in these surface-pairing rows and the content-tag pairings".
23. **(D13)** L648-650 → "the §2 probe (the decider's table walk) reads the compatible
    surface's caps regardless of instance provenance".
24. **(D14)** L155 → "Our fork opts in unconditionally to `WideGamut` (an app-level
    choice; the §8 smoke example asserts this path)."
25. **(D15)** L37 ladder cell → "negative fp16 GPU round-trip via the PR 3 readback"
    and one clause in §8 noting it reuses the 2a vectors on the fp16 surface — or merge
    the row into 2a if it is the same test.
26. **(D16)** L724 bullet header → "✓ wgpu 30.0.1 (types / core / hal / web):" and tag
    each path with its crate ("core device/surface_config.rs:24-40", "hal
    dx12/mod.rs:1455-1466", …).
27. **(D18) Deduplicate.** L263-265 → "Migration: if #8703 merges first, PR 1 becomes
    an evolution PR; the accepted cost is itemized in §10." L203-205 → "**Status +
    target** — the per-viewport decision of the previous paragraph, surfaced to apps
    through `ViewportInfo` …". Drop the L351-352 repetition of the binding rule
    ("(v1 — new primaries follow the binding rule above)").
28. **(D19)** L487 → "**Exposure (ships in PR 3).**"; L460 → "Three distinct mechanisms,
    each with its own policy:"; L639 → "- **MSAA on fp16** — Metal grants resolve on
    all realistic hardware except `apple1x`; …".
29. **(D21) Connect the two headless stories.** L626-629, append: "This override hooks
    the no-surface stub in `RenderState` creation (lib.rs `vec![Rgba8Unorm]`) used by
    headless harnesses; the decider is not on this path (a surfaceless viewport gets
    `FellBack { NoSurface }`, §2)."

## 4. Verdict

Layered, not single-plan. The round-3 vocabulary (capability enum, status/target split,
per-viewport decisioning, headroom-in-PR-3, 2a/2b, corrected scRGB story) is present,
but its consequences were not propagated: §4/§8 still say "PR 2" (L299, L635); §9 still
says "the stored decision" (L643); §2 still carries pre-split arithmetic ("two entries
per preference", L255) and ungated headroom mentions (L170, L204); the corrected Windows
story (L536) now contradicts §4's loud-reject table (L376) — the plan promises a
wide-gamut content path its own pairing table forbids; Appendix B omits the round-3 log
that exists on disk. Top 5 fixes: (1) C2/fix 1 — re-scope PR 4's wide-gamut claim;
(2) C1/fix 2 — pipeline-cache count; (3) C3+C5/fixes 3+5 — escape-hatch and f16-donor
reconciliation; (4) D4/fix 12 — the 2a/2b sweep; (5) C8+D5/fixes 8+15 — Appendix B and
the merged future-work list.
