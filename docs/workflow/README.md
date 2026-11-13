# Addon and uploader workflow: spec and plan

Status: **draft for the owner's review** (2026-11-13). Nothing here is built yet. This folder replaces the plan in PR #8. If it disagrees with older notes, this folder wins.

**This is a workflow fix, not a rewrite (AD1).** The addon, the uploader core and the uploader app all work, and their tests pass quickly. What's missing: the tests run only on release tags, every PR pays for a full Windows + macOS build, nothing checks the wire format against the site, and nothing tests in-game behaviour.

**This repo is public.** Nothing in it may name the guild, its domain, its people or the owner's machines. `scripts/check-no-branding.sh` must stay green.

| File | What it answers |
|---|---|
| [`../../contract/SPEC.md`](../../contract/SPEC.md) | **The sync contract** between the addon, the uploader and the site: export strings, signing, types, HTTP endpoints, versioning, known gaps. The single source; the site repo points here. |
| [01-decisions.md](01-decisions.md) | What we decided and why |
| [05-verification.md](05-verification.md) | How work is proven: CI jobs, golden fixtures, wowless, uploader UI and native checks, agent rules (replaces PR #8) |
| [06-plan.md](06-plan.md) | Tasks with goal, read, deliver, acceptance |
| [07-open-questions.md](07-open-questions.md) | What still needs the owner, with defaults |

The site repo (private) has the matching folder `docs/workflow/` with the site's architecture, design system and plan. Its tasks are `P<n>`; this repo's are `A<n>`. The site's contract tests (its P5) depend on A2 here.

## Measured on 2026-11-13 (main @ 11d6b50)
| What | Result |
|---|---|
| Size | addon ~700 lines of Lua (+ LibDeflate); uploader ~8k lines of Rust (core + Tauri app); UI plain HTML/JS |
| `tests/run.py` (Lua 5.1 + LuaJIT via lupa) | pass, **0.3 s** |
| `cargo test -p guild-uploader-core` | 82 pass in 0.8 s (31 s cold compile) |
| `node --test uploader/app/ui` | 7 pass, 0.15 s |
| CI on PRs | only `uploader-app.yml`: full Windows + macOS Tauri build, no path filter, `contents: write` on PR runs |
| Addon/core tests in CI | only on `addon-v*` / `uploader-v*` tags |
| Lint | none (no luacheck, `cargo fmt --check`, clippy) |
| Contract | cited as `export-format.md` in 15 places; the file doesn't exist (now `contract/SPEC.md`) |
| In-game emulator | [wowless](https://github.com/wowless/wowless) has a `wowf` product at client 1.60.1.70888, `tocversion: 16001` = this addon's interface. Pre-alpha. |
