# Plan: PR checks, a cross-repo contract, and evidence for untestable parts (addon + uploader)

> **Status: plan only.** This PR adds this file and nothing else. An agent will implement it later, one part per PR, in the order at the bottom. Remove this file when the work is done.
> **This repo is public.** Nothing below may add secrets, guild or owner identifiers, or production URLs to tracked files or CI logs. `scripts/check-no-branding.sh` must stay green.

## What exists today (keep it)

- `tests/run.py` loads the addon under **Lua 5.1 and LuaJIT** (lupa) with game API stubs and a fixed clock (`E2E_T0`). It checks SHA-256/HMAC vectors, bank, recipe and rank exports, round-trip decoding and the no-guild case.
- `tests/package-check.sh` / `package_check.py` check the built addon zip.
- `uploader/core` has `cargo test` coverage. `uploader/app/ui/*.test.js` are `node --test` unit tests for the uploader window logic.
- `uploader-app-release.yml` builds the Tauri app on Windows and macOS, runs `launch-smoke.ps1` on Windows and checks the universal dmg on macOS.

## Problems

1. **The addon tests and the core tests only run on release tags.** `addon-release.yml` runs on `addon-v*` and `uploader-release.yml` on `uploader-v*`. A PR that breaks `Codec.lua` or `envelope.rs` shows green until someone tags a release.
2. **Every PR, even a README fix, pays for a full Windows and macOS Tauri build.** `uploader-app-release.yml` has `pull_request:` with no path filter. This is the slowest feedback and the most expensive runner minutes. It is also the only check that runs on PRs, so it's acting as the de facto CI. Its top-level permissions are also `contents: write`, `id-token: write` and `attestations: write`, even on PR runs.
3. **There is no linting:** no `luacheck`, no `cargo fmt --check`, no `clippy`.
4. **The cross-repo contract is unchecked.** `run.py` writes `/tmp/addon-out.json` "for verify.mjs", but `verify.mjs` does not exist in this repo or the site repo. So nothing proves that the site still decodes what this addon exports, or still accepts what the uploader posts (`/api/uploader/ingest`, `ranks`, `logs`, `latest`, and oauth `authorize`/`consent`/`token`/`switch`).
5. **Native behaviour and in-game behaviour have no recorded evidence.** PR bodies say things like "not verified natively on Windows or macOS (CI builds both)". In-game behaviour (events, frames, slash commands, SavedVariables across reloads) can't run in CI at all, and nothing records what was checked by hand.

## Ground rules for the implementing agent

- Pin every new tool: action SHAs or major tags as the repo already does, `luacheck` version, Rust toolchain `1.90.0` (match the existing), and `lupa==2.4`.
- PR jobs get `permissions: contents: read`. Only tag and release jobs keep write, `id-token` and attestation permissions.
- No secrets are available to PR jobs, and none are needed. The release signing script stays release-only.
- One part per PR, each with its own tests.

---

## Part 1: A fast PR workflow (`.github/workflows/ci.yml`)

Trigger on `pull_request` and on `push` to `main`. Use `dorny/paths-filter` (pinned), or one workflow per area with `paths:`, so each job runs only when its area changes. Always run a final `ci-ok` job that `needs` all of them and treats "skipped" as success. **That single job is the required check**, so path filtering never blocks merge.

| Job | Runs when these change | Steps | Runner |
|---|---|---|---|
| `branding` | anything | `scripts/check-no-branding.sh` with `BRANDING_WORDS` from the repo secret (fork PRs get no secrets, so the script skips; it runs again on `push` to `main`) | ubuntu |
| `addon` | `addon/**`, `tests/**`, `scripts/build-addon.sh` | `pip install lupa==2.4`; `python tests/run.py`; `luacheck addon --std lua51` (with a `.luacheckrc` declaring WoW globals used and excluding `Libs/`); `scripts/build-addon.sh` + `tests/package-check.sh` | ubuntu |
| `core` | `uploader/core/**`, `uploader/Cargo.*` | `cargo fmt --check`; `cargo clippy -p guild-uploader-core --locked -- -D warnings`; `cargo test -p guild-uploader-core --locked` | ubuntu |
| `ui` | `uploader/app/ui/**` | `node --test uploader/app/ui/`; Part 4 Playwright | ubuntu |
| `app` | `uploader/app/src-tauri/**`, `uploader/Cargo.*`, `uploader/core/**` | `cargo clippy -p <app crate> -- -D warnings`; `cargo test` for the app crate | ubuntu (with the Tauri Linux deps) |
| `native` | `uploader/app/**`, `uploader/core/**`, `uploader/Cargo.*`, **or** the PR has the label `native-build` | call the existing Windows and macOS build as a reusable workflow (below) | windows-2022, macos-15 |

Notes:

- Turn the build part of `uploader-app-release.yml` into a **reusable workflow** (`workflow_call`) with an input `release: false|true`. `ci.yml` calls it with `release: false` (build plus `launch-smoke.ps1` plus the dmg check, no signing, no release). The tag trigger calls it with `release: true`. Remove `pull_request:` from the release workflow.
- If the first clippy and fmt run produces many findings, land the fixes in their own PR **before** making the jobs blocking. Do not add blanket `allow`s.
- Make `addon-release.yml` and `uploader-release.yml` **re-run** the same jobs (call `ci.yml` or the reusable pieces) instead of keeping their own copies of the test steps, so tag runs and PR runs can't drift.
- Add `dependabot.yml` for `cargo`, `github-actions` and `pip` (if a `requirements-test.txt` is added for lupa).

The owner sets `ci-ok` as the required check on `main` (settings → rules). List it in the PR.

---

## Part 2: Committed golden fixtures (replaces the missing `verify.mjs`)

The point is for this repo to **publish what it produces** and the site repo to **prove it can read it** (the site-side plan covers the reader).

1. `tests/golden/` is committed and generated by `python tests/run.py --write-golden`:
   - `exports/<type>-v<n>.txt`: one export string per type (`gbank`, `recipes`, `ranks`, and every other type `ns.export` supports), produced from the fixed inputs `run.py` already uses (fixed `E2E_T0`, `E2E_GUID`, the test secret `"11" * 32`).
   - `exports/<type>-v<n>.json`: the decoded payload the site is expected to see.
   - `uploader/<endpoint>.request.json` and `<endpoint>.response.json`: envelopes the uploader sends to each `/api/uploader/*` endpoint, with expected status and response shape. Generate them from `uploader/core` (`envelope.rs`, `ranks.rs`, `log.rs`) in a `cargo test` that writes them with a fixed clock and the test secret.
   - `MANIFEST.json`: lists every file, the addon export version(s), the uploader protocol version and the generating commit.
   - Only the test secret is used. **No real secrets, guild names or character names.**
2. `python tests/run.py` (no flag) **compares** its output with `tests/golden/` and fails on any difference. The same goes for the core `cargo test`. Changing what's on the wire then always shows as a diff in the PR, which a reviewer can read as text.
3. **Version rule (write it in the README and in `AGENTS.md`):** changing the encoding of an existing type requires a **new** `v<n+1>` file. The previous version's file stays in `tests/golden/` until the site has dropped support. Never edit an existing `v<n>` fixture in place. CI enforces it: a step fails if a PR modifies (rather than adds) a file under `tests/golden/exports/` unless the PR has the label `golden-break`.
4. Remove the `/tmp/addon-out.json` write and the `verify.mjs` comment from `run.py`, or point them at the golden directory.
5. A `contract` job in `ci.yml`, runnable once the site repo has `test:addon-contract`, checks out the site repo at `main` and runs its contract test against **this PR's** golden files. The site repo is private, so this needs a read-only deploy key or fine-grained token as a repo secret. Because PRs from forks don't get secrets, run this job only on `push` to `main` and on PRs from branches in this repo; skip it, don't fail it, for fork PRs. If the owner prefers not to add a secret here, the site's weekly sync job (site plan) is the fallback, and this job is left out.

---

## Part 3: The uploader ↔ site HTTP contract (Rust side)

1. In `uploader/core`, add tests that run the real HTTP client (`http.rs`) against a local mock server (`wiremock` or `httpmock`, pinned dev dependency) that serves the `tests/golden/uploader/*.response.json` files. Cover:
   - the happy path for each endpoint;
   - 401 (expired or revoked token, leading to the reconnect flow), 409/422 (rejected envelope), 429 with `Retry-After`, 5xx with retry and backoff, and a TLS or connection failure;
   - the oauth `authorize` → `consent` → `token` sequence and `switch`, with state and PKCE checks if used.
2. Assert the **user-visible reason codes** the UI maps (`error-reasons.test.js` already lists them), so the core and the UI agree on the reason names. Put the list of reason codes in one shared file (for example `tests/golden/uploader/reasons.json`), read by both the Rust test and the JS test.
3. `update.rs` and `app_update.rs`: test version comparison and the pre-release/draft filtering against recorded release JSON fixtures, so an update check can't offer a draft or a downgrade.

---

## Part 4: The uploader window, checked without screenshots by default

The uploader UI is plain `index.html` + `app.js` + `styles.css`. It can be rendered in a normal browser with a fake Tauri bridge, so no native build is needed.

1. Add `uploader/app/ui/e2e/` with `@playwright/test` (pinned, its own small `package.json` there, or at the repo root, with Chromium only):
   - `fake-tauri.js` is injected with `page.addInitScript`. It defines `window.__TAURI__` (or the `invoke` and `listen` the UI uses) with scripted responses per scenario.
   - A static server (`npx http-server` or Playwright's `webServer`) serves `uploader/app/ui/`.
2. Scenarios, one test each, taken from the states the existing unit tests already cover:
   - first run / not connected; connecting; connected idle; uploading with progress; upload done; each error reason (`error-reasons.test.js`); WoW not detected; addon not installed or outdated (`addon_auto.rs`); update available; account switch.
3. For each scenario, `expect(page.locator("body")).toMatchAriaSnapshot()` with the snapshot stored as `.aria.yml`. This is text and is the cheap way to show "this state shows these words and buttons".
4. Use **one** `toHaveScreenshot` per scenario of the window at its fixed size (from `tauri.conf.json`), with `animations: "disabled"`. Generate baselines **on Linux CI only** via a `workflow_dispatch` job that commits them to the PR branch. Real Windows and macOS rendering differs (fonts, WebView2 vs WKWebView); that is covered by Part 5, not pixels.
5. Add a measurement check for `window-fit.js`: no scrollbars and no clipped content at the window size, for the longest translated or error string.
6. `use: { screenshot: "only-on-failure", trace: "retain-on-failure" }`. Put a one-line-per-failure summary in `$GITHUB_STEP_SUMMARY`.

---

## Part 5: Native and in-game checks (manual, but recorded)

Some things can't be automated in hosted CI: WebView2 and WKWebView rendering, tray behaviour, autostart, the installer hook, macOS Gatekeeper with unsigned builds, and everything inside the game client. Make the manual part small, repeatable and visible.

1. **Make the existing smoke stronger first.** Have `launch-smoke.ps1` (Windows) and a new `launch-smoke.sh` (macOS) start the built app with a `--smoke` flag that renders the window, writes the UI's accessibility tree, or a JSON of visible text and state, to a file, and exits 0. Assert on that file. Add a single **window** screenshot artifact per OS. Then "renders natively" is proven by CI, and an agent can read the JSON, not the image.
2. `docs/testing/native-checklist.md`: short numbered steps per OS (install, first launch, connect, an upload, tray, quit and relaunch, autostart toggle, update prompt, uninstall). Each step has an expected result.
3. `docs/testing/in-game-checklist.md`: per addon area (`Bank`, `Recipes`, `Guild` ranks, `Comms`, slash commands, SavedVariables across `/reload` and relog, Lua errors with `/console scriptErrors 1`), with expected result per step and the game client version tested.
4. The PR template (Part 7) asks which checklist items the change affects, and the owner (or tester) ticks them with the client or OS version. If a change touches none, say "n/a".

---

## Part 6: Rules for agents (`AGENTS.md` and `.cursor/rules/verification.mdc`)

- **Default loop:** `python tests/run.py`, `cargo test -p guild-uploader-core`, `node --test uploader/app/ui/`, and `npx playwright test` in `uploader/app/ui/e2e`. Read the text output.
- **Do not open screenshots on passing runs.** On a pixel failure, open only that test's `*-diff.png`. Prefer the ARIA snapshot and the `--smoke` JSON.
- Wire-format changes: update or **add** golden fixtures (never edit an old version), bump the export or protocol version, and say so in the PR. Link the matching site PR if the site must change.
- Never claim native or in-game behaviour was verified unless the checklist was run. Write "not verified natively" plainly otherwise. Never claim CI passed without the run link for the head SHA.
- Never touch `scripts/uploader-sign-release.sh`, release workflows or permissions in a feature PR.
- Keep `check-no-branding.sh` green, and use neutral fixture names.

---

## Part 7: PR template (`.github/pull_request_template.md`)

```markdown
## What and why

## Area
- [ ] addon (Lua)   - [ ] uploader core   - [ ] uploader UI   - [ ] uploader native/app   - [ ] CI/release

## Requirements → tests
| Id | Requirement | Test (file › name) | Result |
|---|---|---|---|

## Wire format
- Export types/versions changed: none / list (new `tests/golden/...` files)
- Uploader endpoints/envelopes changed: none / list
- Site change needed: none / link to site PR

## Checks
- CI run for head SHA: <link>
- Pixel baselines changed: none / list (via the baseline workflow)
- Native checklist: n/a / items run on Windows __ / macOS __
- In-game checklist: n/a / items run on client __
```

---

## Order of work and acceptance

| Part | Depends on | Acceptance |
|---|---|---|
| 1 PR workflow | – | addon, core, ui, branding run on PRs with path filters; native build only when uploader paths change or labelled; `ci-ok` required; PR jobs read-only; release workflows reuse the same jobs |
| 2 Golden fixtures | 1 | `tests/golden/` committed; `run.py` and core tests fail on drift; "add, don't edit" rule enforced |
| 3 HTTP contract | 2 | mock-server tests for every `/api/uploader/*` endpoint and error class; shared reason list |
| 4 Uploader UI | 1 | ARIA snapshots for every state; Linux-only window baselines; no screenshots on pass |
| 5 Native/in-game | 1 | `--smoke` JSON asserted on Windows and macOS; both checklists committed |
| 6 Agent rules | 2, 4 | `AGENTS.md` + `.cursor/rules/verification.mdc` |
| 7 PR template | 2, 5 | present |
| Contract job | 2 + site `test:addon-contract` | runs on `main`/branch PRs, skipped for forks |
