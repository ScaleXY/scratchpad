#!/usr/bin/env bash
# Packages Scratchpad for one Linux target as .tar.gz and .deb, with optional GPG signatures.
#
#   scripts/package-linux.sh <x86_64-unknown-linux-gnu|i686-unknown-linux-gnu|aarch64-unknown-linux-gnu> [out-dir]
#
# Signing (optional):
#   GPG_PRIVATE_KEY   ASCII armored private key
#   GPG_PASSPHRASE    its passphrase
# Produces detached `.asc` signatures next to each artifact.
set -euo pipefail

TARGET="${1:?usage: package-linux.sh <target> [out-dir]}"
OUT="${2:-dist}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$("$ROOT/scripts/version.sh")"
case "$TARGET" in
  x86_64-unknown-linux-gnu) ARCH=x64; DEB_ARCH=amd64 ;;
  i686-unknown-linux-gnu) ARCH=x86; DEB_ARCH=i386 ;;
  aarch64-unknown-linux-gnu) ARCH=arm64; DEB_ARCH=arm64 ;;
  *) echo "unsupported target $TARGET" >&2; exit 1 ;;
esac

BIN="$ROOT/target/$TARGET/release/scratchpad"
if [[ ! -x "$BIN" ]]; then
  cargo build --release --locked --target "$TARGET" -p scratchpad --manifest-path "$ROOT/Cargo.toml"
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
NAME="scratchpad-$VERSION-linux-$ARCH"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$NAME"
cp "$BIN" "$STAGE/$NAME/scratchpad"
cp "$ROOT/README.md" "$ROOT/packaging/linux/scratchpad.desktop" "$ROOT/packaging/linux/scratchpad.service" "$STAGE/$NAME/"
cp "$ROOT/packaging/icons/scratchpad-512.png" "$STAGE/$NAME/scratchpad.png"
tar -C "$STAGE" -czf "$OUT/$NAME.tar.gz" "$NAME"

if ! command -v cargo-deb >/dev/null 2>&1; then
  cargo install cargo-deb --locked --version "^3"
fi
# cargo-deb reads target/release/scratchpad relative to the crate; point it at the target dir.
# Debian sorts `1.3.0~rc.1` before `1.3.0` (a `-` would mean a package revision).
cargo deb --no-build --no-strip --target "$TARGET" -p scratchpad --manifest-path "$ROOT/Cargo.toml" \
  --deb-version "$(printf '%s' "$VERSION" | sed 's/-/~/')" --output "$OUT/$NAME.deb"

if [[ -n "${GPG_PRIVATE_KEY:-}" ]]; then
  export GNUPGHOME="$STAGE/gnupg"
  mkdir -m 700 "$GNUPGHOME"
  echo "$GPG_PRIVATE_KEY" | gpg --batch --import
  for file in "$OUT/$NAME.tar.gz" "$OUT/$NAME.deb"; do
    gpg --batch --yes --pinentry-mode loopback --passphrase "${GPG_PASSPHRASE:-}" --armor --detach-sign "$file"
  done
  echo "Signed artifacts with GPG"
else
  echo "GPG_PRIVATE_KEY not set: artifacts are not GPG signed"
fi
echo "Packaged $NAME ($DEB_ARCH)"
