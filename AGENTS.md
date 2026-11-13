# Agent rules (addon and uploader)

The single source of rules for every agent working in this repo. `CLAUDE.md` imports this file.
Spec and plan: `docs/workflow/`. Wire format: `contract/SPEC.md`. If these disagree with older notes, they win.

**This repo is public.** Never add guild, character, domain, owner or machine names, secrets or production URLs.
Use neutral names ("Example Guild", "Testchar", `https://guild.example`) and only the test secret `"11" * 32`.

## Repo map
| Path | What |
|---|---|
| `addon/` | In-game addon (Lua 5.1). `Codec.lua` = export strings, `Core.lua` = commands and imports, `Bank.lua`, `Recipes.lua`, `Guild.lua`, `Comms.lua`. `Libs/` is vendored; don't edit. |
| `addon/Addon.toc.template`, `scripts/build-addon.sh` | Build config comes from repo variables (`ADDON_NAME`, `ADDON_SLASH`, …), never hard-coded. |
| `tests/run.py` | Loads the addon under Lua 5.1 and LuaJIT (lupa) with WoW API stubs and a fixed clock. |
| `tests/package-check.sh` | Builds a zip with test config and checks what ships. |
| `uploader/core/` | Rust crate `guild-uploader-core`: envelope, HTTP, OAuth sign-in, ranks, logs, updates, game detection. |
| `uploader/app/src-tauri/` | Rust crate `guild-uploader-app` (Tauri desktop app, updater binary). |
| `uploader/app/ui/` | Plain HTML/JS window UI, tested with `node --test`. |
| `contract/` | `SPEC.md` (the wire format) and, after A2, `golden/` fixtures. |
| `.github/workflows/` | Release workflows (tags) and, after A1.1, `ci.yml` for PRs. |

## Commands
Run the ones for the area you touched. Each prints failures and a summary line.
```sh
python3 -m venv v && v/bin/pip install lupa==2.4 pillow==11.0.0   # once
v/bin/python tests/run.py                                       # addon, Lua 5.1 + LuaJIT (< 1 s)
PATH=$PWD/v/bin:$PATH sh tests/package-check.sh                 # addon packaging
(cd uploader && cargo test -p guild-uploader-core --locked)     # core (~1 s after the first build)
(cd uploader && cargo test -p guild-uploader-app --locked)      # app (needs Tauri system libs on Linux)
(cd uploader/app/ui && node --test)                             # window UI logic
BRANDING_WORDS=... scripts/check-no-branding.sh                 # CI runs it with a repo secret
```
Planned (see `docs/workflow/06-plan.md`; use them once they exist): luacheck, `cargo fmt --check`, clippy (A1.2),
`tests/run.py --write-golden` (A2), `tests/api-check.py` (A3), `tests/wowless/run.sh` (A4), uploader UI Playwright (A6).

## Rules
1. **One task per PR.** Take it from `docs/workflow/06-plan.md` by ID (A1.1, A2, …). Follow `.github/pull_request_template.md`.
2. **Spec-first.** The first commit holds the failing requirement tests, titled `R<n>: <requirement>`. Implement until green.
   Never loosen an approved assertion without the owner's OK written in the PR body.
3. **CI is the only judge.** Never claim "tests pass" without the CI run link for the head SHA. Read the CI step summary,
   not raw logs. Never skip, disable or loosen a test, and never make a blocking job non-blocking, to get green.
4. **Wire format** (`contract/SPEC.md` §5.2): a changed encoding is a new version `v<n+1>` with new golden files.
   Never edit an existing golden file. Link the matching site PR when the site must change, and the site must accept
   a new version before an addon release produces it.
5. **No pixel tests.** No `toHaveScreenshot`, baseline images or full-page screenshots. UI is proven by ARIA snapshots
   and layout assertions. Screenshots only when a human asks, one element at a time.
6. **Never update ARIA snapshots** without explaining each changed line in the PR body.
7. **Say what was not verified.** In-game and native behaviour count as verified only through wowless, the `--smoke`
   JSON or the release checklists. Otherwise write "not verified in game" or "not verified natively".
8. **Don't touch release machinery in a feature PR:** `scripts/uploader-sign-release.sh`, release workflows, workflow
   permissions, signing keys.
9. **Pin tools:** action versions, Rust 1.90.0, `lupa==2.4`, and any new dev dependency.
10. **Keep `scripts/check-no-branding.sh` green.**
11. Commit messages say what changed and why, in plain words. No model names.
12. Merging is a human action. Never merge, and never change repository settings.

## Escalate to the owner instead of guessing
Anything in `docs/workflow/07-open-questions.md` that your task depends on and has no default; a test that seems wrong;
a change that needs a secret, a repo setting, or a release.
