#!/bin/sh
# Packaging check: build a zip with test config and verify what ships (slash commands, icon, TOC). Needs lupa + Pillow.
set -eu
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
python3 -c "from PIL import Image; Image.new('RGBA', (96, 80), (200, 20, 30, 255)).save('$tmp/logo.png')"
ADDON_NAME=Guildaddon ADDON_TITLE="Guild Addon" ADDON_SLASH=ga ADDON_ICON="$tmp/logo.png" SOURCE_DATE_EPOCH=1793836800 \
  sh scripts/build-addon.sh 9.9.9 >/dev/null
python3 tests/package_check.py dist/Guildaddon-9.9.9.zip Guildaddon /ga /guildaddon
# Without an icon the TOC has no IconTexture line.
ADDON_NAME=Guildaddon ADDON_SLASH=ga SOURCE_DATE_EPOCH=1793836800 sh scripts/build-addon.sh 9.9.9 >/dev/null
python3 - <<'PY'
import zipfile
z = zipfile.ZipFile("dist/Guildaddon-9.9.9.zip")
toc = z.read("Guildaddon/Guildaddon.toc").decode()
assert "## IconTexture" not in toc and "Guildaddon/Icon.tga" not in z.namelist(), "icon line without icon"
print("package check: no-icon build OK")
PY
rm -rf dist
