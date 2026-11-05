#!/bin/sh
# Fails if guild names, domains or other branding appear in tracked source. The words come from the
# BRANDING_WORDS env var (repo secret in CI), so this file doesn't name them either.
set -eu
[ -n "${BRANDING_WORDS:-}" ] || { echo "BRANDING_WORDS not set; skipping"; exit 0; }
pat=$(printf '%s' "$BRANDING_WORDS" | tr ',' '|')
if git ls-files | grep -v -e '^addon/Libs/' -e '^LICENSE$' | xargs grep -I -n -i -E "$pat"; then echo "branding found"; exit 1; fi
echo "no branding"
