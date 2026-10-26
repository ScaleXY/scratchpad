#!/usr/bin/env bash
# Builds Scratchpad.app and a .dmg for one macOS target, then signs and notarizes them.
#
#   scripts/package-macos.sh <aarch64-apple-darwin|x86_64-apple-darwin> [out-dir]
#
# Signing is controlled by environment variables (all optional):
#   APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_P12_BASE64
#       base64 encoded Developer ID Application .p12
#   APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_PASSWORD
#       password of the .p12
#   APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_IDENTITY
#       e.g. "Developer ID Application: ScaleXY (TEAMID)"
#   APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID, APPLE_DEVELOPER_ACCOUNT_TEAM_ID,
#   APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID_APP_PASSWORD
#       credentials for notarization
# Without a certificate the app is ad-hoc signed (runs locally, Gatekeeper will warn).
set -euo pipefail

TARGET="${1:?usage: package-macos.sh <target> [out-dir]}"
OUT="${2:-dist}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$("$ROOT/scripts/version.sh")"
case "$TARGET" in
  aarch64-apple-darwin) ARCH=arm64 ;;
  x86_64-apple-darwin) ARCH=x64 ;;
  *) echo "unsupported target $TARGET" >&2; exit 1 ;;
esac
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

BIN="$ROOT/target/$TARGET/release/scratchpad"
if [[ ! -x "$BIN" ]]; then
  cargo build --release --locked --target "$TARGET" -p scratchpad --manifest-path "$ROOT/Cargo.toml"
fi

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
STAGE="$(mktemp -d)"
APP="$STAGE/Scratchpad.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/scratchpad"
cp "$ROOT/packaging/icons/scratchpad.icns" "$APP/Contents/Resources/scratchpad.icns"
# Bundle versions must be dotted integers: 1.3.0-rc.1 and 1.3.0+2.gabc1234 become 1.3.0.
sed "s/__VERSION__/${VERSION%%[-+]*}/g" "$ROOT/packaging/macos/Info.plist" > "$APP/Contents/Info.plist"

KEYCHAIN=""
cleanup() {
  [[ -n "$KEYCHAIN" ]] && security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
  rm -rf "$STAGE"
}
trap cleanup EXIT

IDENTITY="-"
if [[ -n "${APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_P12_BASE64:-}" ]]; then
  KEYCHAIN="$STAGE/signing.keychain-db"
  KEYCHAIN_PASSWORD="$(openssl rand -hex 16)"
  echo "$APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_P12_BASE64" | base64 --decode > "$STAGE/cert.p12"
  security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
  security import "$STAGE/cert.p12" -k "$KEYCHAIN" -P "${APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign
  security set-key-partition-list -S apple-tool:,apple: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
  security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
  IDENTITY="${APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_IDENTITY:?APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_IDENTITY is required with APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_P12_BASE64}"
  # Notarization only accepts Developer ID; "Apple Distribution" is for the App Store
  # (scripts/package-macos-appstore.sh). A SHA-1 hash is checked against the keychain.
  if ! security find-identity -v -p codesigning | grep -F "$IDENTITY" | grep -q '"Developer ID Application: '; then
    echo "APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_IDENTITY must be a \"Developer ID Application\" certificate, not: $IDENTITY" >&2
    security find-identity -v -p codesigning >&2
    exit 1
  fi
  echo "Signing with: $IDENTITY"
else
  echo "APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_P12_BASE64 not set: ad-hoc signing (not notarized)"
fi

codesign --force --timestamp --options runtime --entitlements "$ROOT/packaging/macos/entitlements.plist" \
  --sign "$IDENTITY" "$APP/Contents/MacOS/scratchpad"
codesign --force --timestamp --options runtime --entitlements "$ROOT/packaging/macos/entitlements.plist" \
  --sign "$IDENTITY" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

NAME="scratchpad-$VERSION-macos-$ARCH"
DMG="$OUT/$NAME.dmg"
# diskutil names the volume after this folder; older versions have no --volumeName.
DMG_ROOT="$STAGE/dmg/Scratchpad"
mkdir -p "$DMG_ROOT"
cp -R "$APP" "$DMG_ROOT/"
ln -s /Applications "$DMG_ROOT/Applications"
rm -f "$DMG"
# `diskutil image` replaces the deprecated `hdiutil create` on newer macOS (not on macos-15 runners).
if diskutil image create from -h >/dev/null 2>&1; then
  # Its progress goes to stderr; show the output only if it fails.
  if ! LOG="$(diskutil image create from --format UDZO "$DMG_ROOT" "$DMG" 2>&1)"; then
    echo "$LOG" >&2
    exit 1
  fi
else
  hdiutil create -volname "Scratchpad" -srcfolder "$DMG_ROOT" -ov -format UDZO "$DMG" >/dev/null
fi
[[ "$IDENTITY" != "-" ]] && codesign --force --timestamp --sign "$IDENTITY" "$DMG"

if [[ "$IDENTITY" != "-" && -n "${APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID:-}" \
  && -n "${APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID_APP_PASSWORD:-}" && -n "${APPLE_DEVELOPER_ACCOUNT_TEAM_ID:-}" ]]; then
  echo "Notarizing $DMG"
  NOTARY_AUTH=(--apple-id "$APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID" --team-id "$APPLE_DEVELOPER_ACCOUNT_TEAM_ID"
    --password "$APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID_APP_PASSWORD")
  RESULT="$(xcrun notarytool submit "$DMG" "${NOTARY_AUTH[@]}" --wait --output-format json)"
  echo "$RESULT"
  read -r SUBMISSION_ID STATUS < <(python3 -c 'import json,sys; r=json.loads(sys.argv[1]); print(r.get("id",""), r.get("status",""))' "$RESULT")
  if [[ "$STATUS" != "Accepted" ]]; then
    echo "Notarization failed ($STATUS). Apple's log:" >&2
    [[ -n "$SUBMISSION_ID" ]] && xcrun notarytool log "$SUBMISSION_ID" "${NOTARY_AUTH[@]}" >&2
    exit 1
  fi
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  # The notarization ticket also covers the app inside the image.
  xcrun stapler staple "$APP" || true
fi

# Plain archives: the .app (for drag & drop) and the bare binary (for headless use).
(cd "$STAGE" && ditto -c -k --keepParent "Scratchpad.app" "$OUT/$NAME-app.zip")
tar -C "$APP/Contents/MacOS" -czf "$OUT/$NAME-cli.tar.gz" scratchpad
echo "Created $DMG"
