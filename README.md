# bloodfang-guild-addon
Bloodfang guild addon and optional uploader for WoW: Forever. Open source so anyone can review it.

## What's here
- `addon/`: the in-game addon. Captures the guild bank (every tab you can view, with item icons and gold) when you open it, saves your profession recipes when you open a profession window, links your character to the site, and shares your zone with online guildmates (in game only; `<cmd> location off` to stop).
- `uploader/`: optional uploader. It reads only the addon's SavedVariables file and posts the signed strings to the site. It never opens, inspects or talks to the game process. Today it's a command-line tool; the desktop app is next.
- Releases are built by GitHub Actions from tags (`addon-vX.Y.Z`, `uploader-vX.Y.Z`) with a SHA-256 file and a build provenance attestation. Builds are unsigned for now.

## Install the addon
1. Download the zip from Releases and check it: `sha256sum -c <zip>.sha256` and `gh attestation verify <zip> --repo <owner>/<repo>`.
2. Unzip into `World of Warcraft/<client>/Interface/AddOns/`. You should get one addon folder.
3. In game: get a code from the site's Link page, type `<cmd> link <code>`, copy the string into the site, paste the string the site gives back with `<cmd> import`.
4. Open the guild bank or a profession window. Then `/reload` (or log out) so the game writes the file, and either run the uploader or copy with `<cmd> export` and paste on the site.

Commands: `<cmd> link <code>`, `<cmd> import`, `<cmd> export [gbank|recipes]`, `<cmd> ranks`, `<cmd> location on|off`, `<cmd> where`, `<cmd> version`.

`<cmd>` is either slash command the build registers: `/<ADDON_SLASH>` (the short one hints show) or `/<addon name in lower case>`. Both come from the build config (repository variables `ADDON_SLASH` and `ADDON_NAME`), like the addon's name. `ADDON_ICON` (a PNG path or https URL, e.g. the site's header logo) becomes the AddOns list icon (`Icon.tga`, 64x64, `## IconTexture`). `tests/package-check.sh` builds a zip with test values and checks what ships.

## Uploader (command line)
Make a token on the site (Settings → Uploader tokens; guild bank uploads need an officer role). Then:
```
UPLOADER_SITE=https://<site> UPLOADER_TOKEN=<token> UPLOADER_ADDON=<addon folder name> guild-uploader "<path>/World of Warcraft/<client>"
```
It remembers what it already sent next to `WTF/`. The token is read from the environment only.

## Rebuild and compare
- Addon: `ADDON_NAME=<folder> scripts/build-addon.sh X.Y.Z` at the tag gives a byte-identical zip (timestamps come from the commit).
- Uploader: `cd uploader && cargo build --locked --release` with Rust 1.85.1.
- Tests: `python3 -m venv v && v/bin/pip install lupa && v/bin/python tests/run.py` (runs the addon under Lua 5.1 and LuaJIT), and `cargo test` in `uploader/`.
