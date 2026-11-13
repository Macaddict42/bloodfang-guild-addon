#!/bin/sh
# contract/SPEC.md 5.2: files under contract/golden are add-only. A changed encoding or request is a NEW file
# (new version or case); existing files are never edited, renamed or deleted. MANIFEST.json and README.md may change.
# Usage: tests/golden-add-only.sh <base-ref>. GOLDEN_BREAK=1 (the PR label `golden-break`) allows it.
set -eu
base="${1:?usage: tests/golden-add-only.sh <base-ref>}"
changed=$(git diff --name-status --diff-filter=MDR "$base"...HEAD -- contract/golden \
  | grep -v -e 'contract/golden/MANIFEST.json$' -e 'contract/golden/README.md$' || true)
if [ -z "$changed" ]; then echo "golden add-only: ok"; exit 0; fi
echo "$changed" | sed 's/^/golden add-only: /'
if [ "${GOLDEN_BREAK:-}" = 1 ]; then echo "golden add-only: allowed by the golden-break label"; exit 0; fi
echo "golden add-only: existing golden files changed. Add a new version or case instead (contract/SPEC.md 5.2)," >&2
echo "or, if the site has dropped the old version, label the PR golden-break." >&2
exit 1
