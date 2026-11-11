#!/usr/bin/env bash
# Sign an uploader-app release's installers with the update key and (optionally) publish it.
#
#   GUILD_UPLOADER_SIGNING_PRIVATE_KEY_B64=... UPLOADER_UPDATE_PUBKEY=<64 hex> \
#     scripts/uploader-sign-release.sh uploader-app-vX.Y.Z [--repo OWNER/NAME] [--publish]
#
# - Refuses unless the release has a Windows setup .exe, a macOS .dmg AND a macOS .app.tar.gz
#   (the archive macOS Update now installs).
# - Each installer (.exe, .dmg, .app.tar.gz) gets a raw 64-byte Ed25519 <file>.sig over its bytes.
#   Existing signatures are verified, never replaced. Each new signature is verified before upload.
# - SHA256SUMS is rewritten to list every asset (existing lines are kept byte-for-byte).
# - --publish takes a draft live only after all of the above passed.
# The private seed is read from the environment only and is never printed or written to disk.
set -euo pipefail
TAG="" REPO="${GITHUB_REPOSITORY:-}" PUBLISH=0
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) REPO="$2"; shift 2 ;;
    --publish) PUBLISH=1; shift ;;
    uploader-app-v*) TAG="$1"; shift ;;
    *) echo "usage: $0 uploader-app-vX.Y.Z [--repo OWNER/NAME] [--publish]" >&2; exit 64 ;;
  esac
done
[ -n "$TAG" ] && [ -n "$REPO" ] || { echo "need a tag and --repo (or GITHUB_REPOSITORY)" >&2; exit 64; }
[ -n "${UPLOADER_UPDATE_PUBKEY:-}" ] || { echo "UPLOADER_UPDATE_PUBKEY (public key hex) is not set" >&2; exit 64; }
[ -n "${GUILD_UPLOADER_SIGNING_PRIVATE_KEY_B64:-}" ] || { echo "GUILD_UPLOADER_SIGNING_PRIVATE_KEY_B64 is not set" >&2; exit 64; }
python3 -c 'import cryptography' 2>/dev/null || { echo "python3 cryptography is required" >&2; exit 64; }

work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
gh release download "$TAG" --repo "$REPO" --dir "$work" >/dev/null
cd "$work"
win=$(ls | grep -E -- '-windows-.*-setup\.exe$' || true)
mac=$(ls | grep -E -- '-macos-.*\.dmg$' || true)
[ -n "$win" ] || { echo "no Windows setup .exe in $TAG" >&2; exit 1; }
[ -n "$mac" ] || { echo "no macOS .dmg in $TAG" >&2; exit 1; }
macapp=$(ls | grep -E -- '-macos-.*\.app\.tar\.gz$' || true)
[ -n "$macapp" ] || { echo "no macOS .app.tar.gz in $TAG (Update now on Mac needs it)" >&2; exit 1; }

new_sigs=()
for f in $(ls | grep -E '\.(exe|dmg|app\.tar\.gz)$'); do
  if [ -f "$f.sig" ]; then
    mode=verify
  else
    mode=sign; new_sigs+=("$f.sig")
  fi
  python3 - "$mode" "$f" "$f.sig" <<'PY'
import base64, os, sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
from cryptography.hazmat.primitives import serialization
from cryptography.exceptions import InvalidSignature
mode, src, sig_path = sys.argv[1:4]
pub_hex = os.environ["UPLOADER_UPDATE_PUBKEY"].strip().lower()
data = open(src, "rb").read()
if mode == "sign":
    seed = base64.b64decode(os.environ["GUILD_UPLOADER_SIGNING_PRIVATE_KEY_B64"])
    if len(seed) != 32: sys.exit("signing seed is not 32 bytes")
    sk = Ed25519PrivateKey.from_private_bytes(seed); del seed
    have = sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
    if have != pub_hex: sys.exit("signing key does not match UPLOADER_UPDATE_PUBKEY; refusing to sign")
    fd = os.open(sig_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    os.write(fd, sk.sign(data)); os.close(fd)
pk = Ed25519PublicKey.from_public_bytes(bytes.fromhex(pub_hex))
sig = open(sig_path, "rb").read()
try:
    if len(sig) != 64: raise InvalidSignature()
    pk.verify(sig, data)
except InvalidSignature:
    sys.exit(f"{sig_path}: signature does not verify")
bad = bytearray(data); bad[len(bad) // 2] ^= 1
try:
    pk.verify(sig, bytes(bad)); sys.exit("tampered copy verified; refusing")
except InvalidSignature:
    pass
print(f"{mode}: {src} ok (tampered copy rejected)")
PY
done

# SHA256SUMS: keep existing lines, add any missing asset.
touch SHA256SUMS; cp SHA256SUMS SHA256SUMS.old
for f in $(ls | grep -v -E '^SHA256SUMS(\.old)?$'); do
  grep -qE "  $f\$" SHA256SUMS.old || sha256sum "$f" >> SHA256SUMS
done
sha256sum -c SHA256SUMS >/dev/null
if [ ${#new_sigs[@]} -gt 0 ]; then gh release upload "$TAG" --repo "$REPO" "${new_sigs[@]}"; fi
if ! cmp -s SHA256SUMS SHA256SUMS.old; then gh release upload "$TAG" --repo "$REPO" SHA256SUMS --clobber; fi
echo "SHA256SUMS:"; cat SHA256SUMS
if [ "$PUBLISH" = 1 ]; then gh release edit "$TAG" --repo "$REPO" --draft=false >/dev/null; echo "published $TAG"; fi
