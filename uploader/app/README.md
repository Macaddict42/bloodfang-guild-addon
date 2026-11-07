# Desktop uploader

Tray/menu-bar app that watches the addon’s SavedVariables and uploads new export
strings to the site. Also installs addon updates from GitHub releases after
verifying `SHA256SUMS`. Files only — it never reads or writes the game process.

Build-time non-secret defaults:

- `SITE_URL` — baked site address. CI uses the repo variable when it is set, otherwise `uploader/app/site.url`. Users do not type it.
- `ADDON_NAME` — SavedVariables / AddOns folder name
- `GITHUB_REPO` — `owner/name` for release lookups

Sign-in opens the system browser. The site session approves an uploader token, which is stored only in the OS credential store (Windows Credential Manager / macOS Keychain). Prefs JSON never contains it. The usual Windows game folder is detected. Start at login is on for a first install; a stored choice, including off, is kept.

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
