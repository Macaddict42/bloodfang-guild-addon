# Golden fixtures

What the addon and the uploader put on the wire, as files. The site repo vendors this folder and proves it can
read every file (site plan P5.1). Rules: `../SPEC.md` section 5.2. Inventory: `MANIFEST.json`.

| Folder | What | Made by |
|---|---|---|
| `strings/` | export strings (`<type>.v<n>.txt`) and their decoded envelopes (`.json`), the canonical-JSON case, the test secret, and `bad/` strings with the error each must give (`bad/expected.json`) | `python tests/run.py --write-golden` (Lua 5.1 writes, LuaJIT must decode the same, Python re-checks decoding and HMAC) |
| `http/<endpoint>/<case>.request.json` | what the uploader sends | `WRITE_GOLDEN=1 cargo test -p guild-uploader-core golden` |
| `http/<endpoint>/<case>.response.json` | what the site answers (`status`, `body`) | written by hand from SPEC 4.2; the core test proves the uploader reads each as SPEC says, the site test proves the site sends it |

**Add-only.** Never edit, rename or delete an existing file here. A changed encoding is a new `v<n+1>` file; a changed
request or response is a new case. The previous version stays until the site has dropped it; only then may a PR
labelled `golden-break` remove it. `tests/golden-add-only.sh` enforces this in CI. `MANIFEST.json` is regenerated
whenever files are added: run the cargo command first, then `run.py --write-golden`.

Test data only: the secret `"11" * 32`, `Player-4395-0ABCDEF1`, "Testchar", "Example Guild", fixed times.
