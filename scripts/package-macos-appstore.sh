#!/usr/bin/env bash
# Builds the Mac App Store package: a sandboxed, universal (arm64 + x64) Scratchpad.app
# inside a signed installer .pkg, and optionally uploads it to App Store Connect.
#
#   scripts/package-macos-appstore.sh [out-dir]          # build dist/scratchpad-<v>-macos-appstore.pkg
#   scripts/package-macos-appstore.sh --upload <pkg>     # validate and upload a built package
#
# Uses target/{aarch64,x86_64}-apple-darwin/release/scratchpad, building them if missing.
#
# Signing (see docs/mac-app-store.md):
#   MAS_PROVISIONING_PROFILE    path to the "Mac App Store Connect" .provisionprofile; the team and
#                               application identifier are read from it. Without a profile the
#                               app is ad-hoc signed with the sandbox entitlements and the .pkg is
#                               unsigned: a local test build that can't be uploaded.
#   APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_IDENTITY
#                               default "Apple Distribution"
#   APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_IDENTITY
#                               default "3rd Party Mac Developer Installer"
#   APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_P12_BASE64
#   APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_PASSWORD
#   APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_P12_BASE64
#   APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_PASSWORD
#                               base64 .p12 of each certificate with its key, and its password
#                               (CI); without them the login keychain is used
#   MAS_BUILD_NUMBER            CFBundleVersion (default: the version), to re-upload a version
# Upload (App Store Connect API key with the App Manager role):
#   APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY_ID, APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_ISSUER_ID,
#   APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY (.p8 contents)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_ID="com.scalexy.scratchpad"
STAGE="$(mktemp -d)"
KEYCHAIN=""
cleanup() {
  [[ -n "$KEYCHAIN" ]] && security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
  rm -rf "$STAGE"
}
trap cleanup EXIT

# Fails if any file in the package's payload is quarantined (ITMS-91109).
check_payload() {
  local expanded="$STAGE/expanded-$RANDOM"
  pkgutil --expand-full "$1" "$expanded"
  if xattr -r "$expanded" | grep com.apple.quarantine; then
    echo "$1 contains quarantined files; rebuild it with this script" >&2
    exit 1
  fi
  rm -rf "$expanded"
}

upload() {
  local pkg="$1"
  : "${APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY_ID:?APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY_ID is required}"
  : "${APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_ISSUER_ID:?APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_ISSUER_ID is required}"
  : "${APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY:?APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY (.p8 contents) is required}"
  if ! pkgutil --check-signature "$pkg" >/dev/null; then
    echo "$pkg isn't signed. Build it with MAS_PROVISIONING_PROFILE and the distribution certificates." >&2
    exit 1
  fi
  pkg="$(cd "$(dirname "$pkg")" && pwd)/$(basename "$pkg")"
  check_payload "$pkg"
  # altool looks for AuthKey_<id>.p8 in ./private_keys.
  mkdir -p "$STAGE/private_keys"
  printf '%s\n' "$APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY" > "$STAGE/private_keys/AuthKey_$APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY_ID.p8"
  cd "$STAGE"
  local auth=(-t macos --apiKey "$APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_KEY_ID"
    --apiIssuer "$APPLE_DEVELOPER_ACCOUNT_APP_STORE_CONNECT_ISSUER_ID")
  xcrun altool --validate-app -f "$pkg" "${auth[@]}"
  xcrun altool --upload-app -f "$pkg" "${auth[@]}"
  echo "Uploaded $(basename "$pkg"). It shows up under TestFlight in App Store Connect once processed."
}

if [[ "${1:-}" == "--upload" ]]; then
  upload "${2:?usage: package-macos-appstore.sh --upload <pkg>}"
  exit 0
fi

OUT="${1:-dist}"
VERSION="$(cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/Cargo.toml" | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="scratchpad"))')"
SHORT_VERSION="${VERSION%%-*}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

BINS=()
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  bin="$ROOT/target/$target/release/scratchpad"
  if [[ ! -x "$bin" ]]; then
    cargo build --release --locked --target "$target" -p scratchpad --manifest-path "$ROOT/Cargo.toml"
  fi
  BINS+=("$bin")
done

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
APP="$STAGE/Scratchpad.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/scratchpad" "${BINS[@]}"
cp "$ROOT/packaging/icons/scratchpad.icns" "$APP/Contents/Resources/scratchpad.icns"

PLIST="$APP/Contents/Info.plist"
sed "s/__VERSION__/$SHORT_VERSION/g" "$ROOT/packaging/macos/Info.plist" > "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${MAS_BUILD_NUMBER:-$SHORT_VERSION}" "$PLIST"
# Build environment keys that Xcode adds and App Store Connect checks.
SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version)"
SDK_BUILD="$(xcrun --sdk macosx --show-sdk-build-version)"
XCODE_VERSION="$(xcodebuild -version | awk 'NR==1 {print $2}')"
XCODE_BUILD="$(xcodebuild -version | awk 'NR==2 {print $3}')"
IFS=. read -r xc_major xc_minor xc_patch <<< "$XCODE_VERSION"
for entry in \
  "DTPlatformName string macosx" \
  "DTPlatformVersion string $SDK_VERSION" \
  "DTPlatformBuild string $SDK_BUILD" \
  "DTSDKName string macosx$SDK_VERSION" \
  "DTSDKBuild string $SDK_BUILD" \
  "DTXcode string $(printf '%02d%d%d' "$xc_major" "${xc_minor:-0}" "${xc_patch:-0}")" \
  "DTXcodeBuild string $XCODE_BUILD" \
  "DTCompiler string com.apple.compilers.llvm.clang.1_0" \
  "BuildMachineOSBuild string $(sw_vers -buildVersion)"; do
  read -r key type value <<< "$entry"
  /usr/libexec/PlistBuddy -c "Add :$key $type $value" "$PLIST"
done

ENTITLEMENTS="$STAGE/entitlements.plist"
cp "$ROOT/packaging/macos/entitlements-appstore.plist" "$ENTITLEMENTS"

if [[ -n "${MAS_PROVISIONING_PROFILE:-}" ]]; then
  security cms -D -i "$MAS_PROVISIONING_PROFILE" > "$STAGE/profile.plist"
  APP_ID="$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.application-identifier' "$STAGE/profile.plist")"
  TEAM_ID="$(/usr/libexec/PlistBuddy -c 'Print :TeamIdentifier:0' "$STAGE/profile.plist")"
  if [[ "$APP_ID" != "$TEAM_ID.$BUNDLE_ID" ]]; then
    echo "the provisioning profile is for $APP_ID, not $TEAM_ID.$BUNDLE_ID" >&2
    exit 1
  fi
  cp "$MAS_PROVISIONING_PROFILE" "$APP/Contents/embedded.provisionprofile"
  sed -i '' "s/__APPLICATION_IDENTIFIER__/$APP_ID/; s/__TEAM_ID__/$TEAM_ID/" "$ENTITLEMENTS"

  if [[ -n "${APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_P12_BASE64:-}" \
    || -n "${APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_P12_BASE64:-}" ]]; then
    : "${APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_P12_BASE64:?APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_P12_BASE64 is required}"
    : "${APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_P12_BASE64:?APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_P12_BASE64 is required}"
    KEYCHAIN="$STAGE/signing.keychain-db"
    KEYCHAIN_PASSWORD="$(openssl rand -hex 16)"
    security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    security set-keychain-settings -lut 21600 "$KEYCHAIN"
    security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    echo "$APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_P12_BASE64" | base64 --decode > "$STAGE/distribution.p12"
    security import "$STAGE/distribution.p12" -k "$KEYCHAIN" -P "${APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_PASSWORD:-}" \
      -T /usr/bin/codesign
    echo "$APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_P12_BASE64" | base64 --decode > "$STAGE/installer.p12"
    security import "$STAGE/installer.p12" -k "$KEYCHAIN" -P "${APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_PASSWORD:-}" \
      -T /usr/bin/productbuild
    security set-key-partition-list -S apple-tool:,apple: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
    security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
  fi
  APP_IDENTITY="${APPLE_DEVELOPER_ACCOUNT_APPLE_DISTRIBUTION_CERTIFICATE_IDENTITY:-Apple Distribution}"
  INSTALLER_IDENTITY="${APPLE_DEVELOPER_ACCOUNT_MAC_INSTALLER_CERTIFICATE_IDENTITY:-3rd Party Mac Developer Installer}"
  echo "Signing for the Mac App Store as $APP_ID"
else
  # The identifiers need a provisioning profile; without one, test the sandbox locally.
  /usr/libexec/PlistBuddy -c 'Delete :com.apple.application-identifier' \
    -c 'Delete :com.apple.developer.team-identifier' "$ENTITLEMENTS"
  APP_IDENTITY="-"
  INSTALLER_IDENTITY=""
  echo "MAS_PROVISIONING_PROFILE not set: ad-hoc signed test build (can't be uploaded)"
fi

# cp keeps extended attributes, such as the quarantine flag on a profile downloaded in a
# browser. App Store Connect rejects packages with quarantined files (ITMS-91109).
xattr -cr "$APP"
if xattr -r "$APP" | grep -q com.apple.quarantine; then
  echo "could not remove com.apple.quarantine from $APP" >&2
  exit 1
fi
codesign --force --sign "$APP_IDENTITY" --entitlements "$ENTITLEMENTS" "$APP"
codesign --verify --strict --verbose=2 "$APP"

PKG="$OUT/scratchpad-$VERSION-macos-appstore.pkg"
rm -f "$PKG"
if [[ -n "$INSTALLER_IDENTITY" ]]; then
  productbuild --component "$APP" /Applications --sign "$INSTALLER_IDENTITY" "$PKG"
  pkgutil --check-signature "$PKG"
else
  productbuild --component "$APP" /Applications "$PKG"
  # Keep the app so the sandboxed build can be run: open dist/appstore-test/Scratchpad.app
  rm -rf "$OUT/appstore-test"
  mkdir -p "$OUT/appstore-test"
  cp -R "$APP" "$OUT/appstore-test/"
fi
check_payload "$PKG"
echo "Created $PKG"
