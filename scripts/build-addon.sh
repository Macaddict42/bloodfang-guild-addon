#!/bin/sh
# Builds <ADDON_NAME>-<version>.zip containing <ADDON_NAME>/ (SPEC §18b). Reproducible: fixed file order,
# timestamps from SOURCE_DATE_EPOCH (the tagged commit's time), no extra zip metadata.
set -eu
VERSION="$1"; NAME="${ADDON_NAME:?set ADDON_NAME}"; TITLE="${ADDON_TITLE:-$NAME}"; INTERFACE="${ADDON_INTERFACE:-16001}"
case "$VERSION" in *[!0-9.]*|"") echo "bad version"; exit 2;; esac
: "${SOURCE_DATE_EPOCH:=$(git log -1 --format=%ct)}"
out=dist; rm -rf "$out"; mkdir -p "$out/$NAME"
cp -r addon/Libs addon/*.lua "$out/$NAME/"
sed -e "s/@INTERFACE@/$INTERFACE/" -e "s/@ADDON_TITLE@/$TITLE/" -e "s/@VERSION@/$VERSION/" -e "s/@ADDON_NAME@/$NAME/" addon/Addon.toc.template > "$out/$NAME/$NAME.toc"
cp LICENSE "$out/$NAME/"
cd "$out"
python3 - "$NAME" "$NAME-$VERSION.zip" "$SOURCE_DATE_EPOCH" <<'PY'
import os, sys, time, zipfile
name, out, epoch = sys.argv[1], sys.argv[2], max(int(sys.argv[3]), 315532800)
stamp = time.gmtime(epoch)[:6]
files = sorted(os.path.join(d, f) for d, _, fs in os.walk(name) for f in fs)
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for f in files:
        i = zipfile.ZipInfo(f.replace(os.sep, "/"), stamp); i.external_attr = 0o644 << 16; i.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(i, open(f, "rb").read())
PY
sha256sum "$NAME-$VERSION.zip" > "$NAME-$VERSION.zip.sha256"
cat "$NAME-$VERSION.zip.sha256"
