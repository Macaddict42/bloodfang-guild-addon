# Open questions (addon and uploader)

Each has a default. If unanswered when a task needs it, the agent uses the default and says so in the PR body.

| # | Question | Needed by | Default |
|---|---|---|---|
| Q1 | `signups` (site → addon) is produced by the site but refused by the addon; `signup`, `attendance` and `kills` are accepted by the site but produced by no part (SPEC §6 gaps 4–5). Build them in the addon, or remove them from the site? | A2.1 | Keep the site readers, add golden files only for what exists today; decide per type in a feature task. |
| Q2 | Import signatures (SPEC §6 gap 3): the addon doesn't verify Ed25519 on `link-ok` (deferred on 2026-11-04). Do it now? | A9 | After A4, so the change is tested in wowless too. |
| Q3 | Rank-name length: 160 bytes in the addon and uploader, 40 characters in the site's envelope path, 32 on `/api/uploader/ranks`. Which limit wins? | A2.1 | 32 characters everywhere, checked on both sides, matching what the in-game rank editor allows (agent confirms in wowless data). |
| Q4 | `/api/uploader/ranks` accepts an unsigned rank block while the signed `ranks` export also exists (SPEC §6 gap 9). Keep both paths? | A2.1 | Keep (it needs an uploader token), but cover both with golden fixtures and one shared validator on the site. |
| Q5 | Should the wowless Docker image be published to the repo's container registry (faster CI) or built and cached per run? | A4 | Cache per run with `actions/cache`; publish only if cold builds exceed 10 minutes. |
| Q6 | `/api/uploader/latest` exists on the site but the uploader never calls it. Remove it or use it? | A2.1 | Keep, documented as unused; remove in a later site cleanup if still unused after the next uploader release. |
