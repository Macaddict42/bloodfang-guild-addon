# Desktop uploader

Tray/menu-bar app that watches the addon’s SavedVariables and uploads new export
strings to the site. Also installs addon updates from GitHub releases after
verifying `SHA256SUMS`. Files only — it never reads or writes the game process.

Build-time non-secret defaults:

- `SITE_URL` — baked site address. CI uses the repo variable when it is set, otherwise `uploader/app/site.url`. Users do not type it.
- `ADDON_NAME` — SavedVariables / AddOns folder name
- `GUILD_NAME` — guild whose in-game rank names are sent to `POST /api/uploader/ranks` (empty sends any guild's block; the site checks it)
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

Every release ships **both** a Windows NSIS installer and a universal macOS `.dmg`.

1. Tag `uploader-app-vX.Y.Z`. CI builds both platforms (with the public update key from the
   `UPLOADER_UPDATE_PUBKEY` repository variable) and creates a **draft** release with SHA256SUMS
   and provenance attestations. The release job fails if either platform is missing, and a
   release that already exists must carry both installers with their `.sig` files.
2. On the machine that holds the update key, with the key in the environment only:
   `scripts/uploader-sign-release.sh uploader-app-vX.Y.Z --repo OWNER/NAME --publish`
   signs each installer (raw Ed25519 `.sig`), verifies it, updates SHA256SUMS and publishes.
   It refuses a release without both platforms. Do not publish without the owner's approval.

`workflow_dispatch` (optional `version` input for artifact names) builds both platforms as
artifacts; `make_draft` also creates a CI draft release. Installers are not Authenticode-signed
or Apple-notarized yet (SignPath / Apple Developer ID later).
