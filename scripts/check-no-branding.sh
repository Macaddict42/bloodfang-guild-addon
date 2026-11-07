#!/bin/sh
# Fails if guild names, domains or other branding appear in tracked source. The words come from the
# BRANDING_WORDS env var (repo secret in CI), so this file doesn't name them either.
set -eu
[ -n "${BRANDING_WORDS:-}" ] || { echo "BRANDING_WORDS not set; skipping"; exit 0; }
pat=$(printf '%s' "$BRANDING_WORDS" | tr ',' '|')
# uploader/app/site.url is the deployment address for the desktop build, not logic.
# The desktop product name and publisher live in the packaging config and the window heading.
if git ls-files | grep -v -e '^addon/Libs/' -e '^LICENSE$' -e '^uploader/app/site.url$' -e '^uploader/app/src-tauri/tauri.conf.json$' -e '^uploader/app/ui/index.html$' | xargs grep -I -n -i -E "$pat"; then echo "branding found"; exit 1; fi
echo "no branding"
