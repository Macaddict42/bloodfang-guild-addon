# Guild Uploader (desktop)

Tray/menu-bar app that watches the addon’s SavedVariables and uploads new export
strings to the site. Also installs addon updates from GitHub releases after
verifying `SHA256SUMS`. Files only — it never reads or writes the game process.

Build-time non-secret defaults (set in CI from repo variables):

- `SITE_URL` — pre-fills Setup
- `ADDON_NAME` — SavedVariables / AddOns folder name
- `GITHUB_REPO` — `owner/name` for release lookups

The uploader token is stored only in the OS credential store (Windows Credential
Manager / macOS Keychain). Prefs JSON never contains it.

## Dev

```bash
cd uploader
cargo test -p guild-uploader-core
# on Windows/macOS with WebView:
cd app/src-tauri && cargo tauri dev
```

## Release

Tag `uploader-app-vX.Y.Z` (or `workflow_dispatch` for a draft). Builds unsigned
NSIS (Windows) and DMG (macOS) plus SHA256SUMS and provenance attestations.
SignPath comes later.

## Releases
CI tag `uploader-app-vX.Y.Z` builds an unsigned Windows NSIS installer and a **universal** macOS `.dmg` (SPEC §18b). Do not publish without Retik.
