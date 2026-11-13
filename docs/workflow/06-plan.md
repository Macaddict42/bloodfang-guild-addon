# Plan (addon and uploader)

Each task is one PR with **goal, read, deliver, acceptance**. Every task follows `05-verification.md` §4 and spec-first (AD5). ∥ = can run in parallel. **[Owner]** = only the owner can do it. Sizes: S ≤ half a day, M ≈ 1 day, L ≈ 2–3 days. Site tasks are `P<n>` in the site repo's `docs/workflow/06-plan.md`.

## Phase 0: Rules
| ID | Task | Size |
|---|---|---|
| A0.1 | **[Owner]** Read `contract/SPEC.md` and this folder; accept or answer `07-open-questions.md`; merge; close PR #8 with a link here. | – |
| A0.2 | **Agent files.** Read: `05-verification.md` §4–5, `01-decisions.md` AD6–AD7. Deliver: `AGENTS.md` (repo map, commands, rules), `CLAUDE.md` (`@AGENTS.md`), PR template. Acceptance: present; branding check green; AGENTS.md under 120 lines. | S |

## Phase 1: CI on every PR
| ID | Task | Size |
|---|---|---|
| A1.1 | **`ci.yml`.** Goal: every PR gets a fast verdict; native builds only when needed. Read: `05-verification.md` §3, `.github/workflows/*`. Deliver: path-filtered `branding`, `addon` (run.py + package-check), `core` (test), `ui` (node --test), `app` (test), `native` (the existing Windows/macOS build turned into a reusable `workflow_call` with `release: false|true`), `ci-ok`; read-only PR permissions; remove `pull_request:` from `uploader-app.yml`; tag workflows call the same jobs. Acceptance: a README-only PR runs only `branding` + `ci-ok` (< 1 min); a `Codec.lua` change runs `addon`; an uploader change runs `native`. **[Owner]** adds a `main` ruleset requiring `ci-ok`. | M |
| A1.2 ∥ | **Lint.** Deliver: `.luacheckrc` + luacheck in `addon`; `cargo fmt --check` and `clippy -D warnings` in `core` and `app`; fixes in this PR if small, else a first PR of fixes and then the blocking switch (no blanket `allow`s). `dependabot.yml`. Acceptance: all three linters blocking and green. | S–M |

## Phase 2: The contract is tested
| ID | Task | Size |
|---|---|---|
| A2 | **Golden fixtures.** Goal: this repo publishes exactly what it produces. Read: `contract/SPEC.md` (all), `tests/run.py`, `uploader/core/src/{envelope,ranks,log,http}.rs`. Deliver: `contract/golden/exports/<type>-v<n>.txt` + `.json` for every type the addon produces (`link`, `gbank`, `recipes`, `ranks`) and the site produces for the addon (`link-ok`, `signups`), generated from fixed inputs; `contract/golden/uploader/<endpoint>.request.json` / `.response.json` from a `cargo test` with a fixed clock; `reasons.json`; `MANIFEST.json`; `run.py --write-golden`; `run.py` and the cargo test fail on drift; the add-only rule with the `golden-break` label; remove the `/tmp/addon-out.json` write; repoint every `export-format.md` reference in this repo to `contract/SPEC.md`. Acceptance: golden files committed; changing one byte of `Codec.lua` output fails `addon` with the file named; editing an existing golden file fails without the label. Unblocks site P5.1. | M |
| A2.1 | **Contract gaps.** Read: `contract/SPEC.md` §6. Deliver: for each gap that is this repo's side, a fix with a test or a line in `07-open-questions.md`: version hard-coding (one constant per side; the uploader reports unknown versions instead of skipping silently), rank-name limits aligned with the site (owner picks the number, Q3), the UI reason map regenerated from `reasons.json`, Lua decoder size limits, DEL/surrogate canonical-JSON cases, stale comments. Acceptance: every §6 item is fixed, moved to a site task, or listed as an open question. | M |

## Phase 3: In-game testing with wowless
| ID | Task | Size |
|---|---|---|
| A3 ∥ | **API check (cheap, no emulator).** Read: wowless `data/products/wowf/{apis,events,globals}.yaml` at a pinned commit. Deliver: `tests/api-check.py`: parse `addon/*.lua` (excluding `Libs/`) for global calls, `C_*` namespaces and `RegisterEvent` names, and fail on any not in the wowless data (with an allow-list file for the addon's own globals); the pinned wowless commit in `tests/wowless/VERSION`. Acceptance: green on `main`; a misspelled API in a scratch commit fails with file:line. | S |
| A4 | **wowless harness (AD8).** Read: `01-decisions.md` AD8, `05-verification.md` §2.4, wowless README and `bin/run.sh`. Deliver: `tests/wowless/Dockerfile` (wowless at the pinned commit), cached in CI; `tests/wowless/run.sh` (build the addon zip with test config, run `wowless run -p wowf --addondir …`); scenario files per §2.4 with their own assertions; error attribution (addon file in the stack = fail; wowless-only stack = annotation); the `wowless` CI job. **Timebox: 3 days.** Acceptance: all scenarios run in CI in < 5 min with cache; a deliberate `nil` call in `Bank.lua` fails the bank scenario. If the timebox ends first: the job lands non-blocking, a report of what failed is added to `07-open-questions.md`, and the remaining scenarios move to `run.py` stubs. | L |

## Phase 4: Uploader HTTP, UI and native
| ID | Task | Size |
|---|---|---|
| A5 ∥ | **HTTP contract tests** (after A2). Read: `05-verification.md` §2.5–2.6, `contract/SPEC.md` §4. Deliver: `wiremock` tests for every endpoint and error class, OAuth sequence, `reasons.json` shared with the UI, update-check tests with recorded release JSON. Acceptance: every row of SPEC §4.2 has a test; 429 on sign-in and 413 on `/ranks` behave as SPEC §5.2 says after A2.1. | M |
| A6 ∥ | **Uploader UI tests** (AD11). Deliver: `uploader/app/ui/e2e/` (Playwright pinned, Chromium), `fake-tauri.js`, one test per state with ARIA snapshot and the window-fit layout check, in the `ui` job; step summary. Acceptance: green; removing a button from one state fails exactly that test with a YAML diff. | M |
| A7 | **Native smoke** (AD12). Deliver: `--smoke` flag in the app writing state JSON and exiting 0; assertions in the `native` job on Windows (extend `launch-smoke.ps1`) and macOS (`launch-smoke.sh`); `docs/testing/native-checklist.md` for the rest. Acceptance: both OS jobs assert on the JSON; a broken window title fails. | M |
| A8 | **In-game release checklist** (AD13): `docs/testing/in-game-checklist.md` (only what wowless can't do), release PR section. Acceptance: under 15 steps, each with an expected result. | XS |
| A9 | **Import signatures** (SPEC §6 gap 3; the owner's 2026-11-04 deferral). Deliver: ship the site's public key in the build config and verify Ed25519 on `link-ok` before replacing a secret, with a Lua implementation tested against site-generated golden strings. **[Owner]** decides when (Q2). | M |

## Order
```
A0.2 → A1.1 → A1.2
A1.1 → A2 → (A2.1 ∥ A5 ∥ site P5.1)
A1.1 → A3 → A4
A1.1 → A6;  A1.1 → A7;  A8 any time;  A9 when the owner says
```
Before Tuesday 13:00 (Claude Code only): A0.2, A1.1, A1.2, A2 are realistic.

## Model guidance
Most capable model for A2, A4, A9 and reviews; a cheaper one is fine for A1.2, A3, A8.
