#!/usr/bin/env bash
#
# Builds a notarized, stapled FerrisPass.dmg from source.
#
# Apple Silicon (arm64) only. Intel Macs are not supported in v0.1.0 — adding
# x86_64 requires removing `target-cpu=native` from .cargo/config.toml because
# that flag mixes host CPU features into cross-compiles. Worth doing later if
# there's demand; for now the simpler path wins.
#
# Pipeline:
#   1. cargo build --release --target aarch64-apple-darwin
#   2. Generate AppIcon.icns from bundle/icon.png
#   3. Assemble FerrisPass.app bundle
#   4. Codesign with Hardened Runtime
#   5. Build DMG (create-dmg if available, else hdiutil)
#   6. Codesign the DMG
#   7. Submit to Apple's notarization service (--wait)
#   8. Staple notarization ticket onto DMG
#   9. Final spctl Gatekeeper assessment
#
# Flags:
#   --skip-notarize    Stop after step 6. Useful for local iteration where
#                      waiting ~3 min for Apple is friction.
#
# Requirements:
#   - Xcode Command Line Tools (lipo, sips, iconutil, codesign, notarytool)
#   - Developer ID Application certificate in Keychain
#   - Notarization credentials stored as keychain profile (see TEAM/PROFILE below)
#   - bundle/icon.png at 1024×1024 (recommend transparent corners outside squircle)
#   - Optional: brew install create-dmg  (else falls back to plain hdiutil)
#
# Output:
#   dist/FerrisPass-<version>-universal.dmg

set -euo pipefail

# ---------- configuration ----------
APP_NAME="FerrisPass"
BINARY_NAME="ferrispass"
BUNDLE_ID="rs.ferrispass.app"
TEAM_ID="5GAMHB3974"
SIGNING_IDENTITY="Developer ID Application: Sonar Analytics - FZCO (${TEAM_ID})"
NOTARIZE_PROFILE="ferrispass-notarize"
MIN_MACOS="12.0"

# ---------- args ----------
SKIP_NOTARIZE=false
for arg in "$@"; do
    case $arg in
        --skip-notarize) SKIP_NOTARIZE=true ;;
        -h|--help)
            sed -n '3,/^set -/p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *) echo "unknown flag: $arg" >&2; exit 2 ;;
    esac
done

# ---------- paths ----------
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE_DIR="${PROJECT_ROOT}/bundle/macos"
ICON_PNG="${PROJECT_ROOT}/bundle/icon.png"
DIST_DIR="${PROJECT_ROOT}/dist"

# ---------- preconditions ----------
if [ ! -f "${ICON_PNG}" ]; then
    echo "✗ ${ICON_PNG} not found. Drop your 1024×1024 master PNG there before running." >&2
    exit 1
fi

if ! security find-identity -v -p codesigning | grep -q "${SIGNING_IDENTITY}"; then
    echo "✗ Signing identity not in Keychain:" >&2
    echo "    ${SIGNING_IDENTITY}" >&2
    echo "  Create it via Xcode → Settings → Accounts → Manage Certificates → + Developer ID Application" >&2
    exit 1
fi

# ---------- read version from Cargo.toml ----------
VERSION=$(grep '^version' "${PROJECT_ROOT}/Cargo.toml" | head -1 \
    | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
if [ -z "${VERSION}" ]; then
    echo "✗ Could not parse version from Cargo.toml" >&2
    exit 1
fi

echo "▸ ${APP_NAME} ${VERSION}"
echo "  bundle id:   ${BUNDLE_ID}"
echo "  team id:     ${TEAM_ID}"
echo "  min macOS:   ${MIN_MACOS}"
echo "  notarize:    $([ "${SKIP_NOTARIZE}" = "true" ] && echo "skipped" || echo "yes")"
echo ""

# Build for the OS floor we declare in Info.plist so the binary actually runs there.
export MACOSX_DEPLOYMENT_TARGET="${MIN_MACOS}"

rm -rf "${DIST_DIR}"
mkdir -p "${DIST_DIR}"

# ---------- 1. arm64 binary ----------
echo "▸ [1/8] Building arm64 binary"
rustup target add aarch64-apple-darwin >/dev/null
cargo build --release --target aarch64-apple-darwin

BIN_PATH="${DIST_DIR}/${BINARY_NAME}"
cp "${PROJECT_ROOT}/target/aarch64-apple-darwin/release/${BINARY_NAME}" "${BIN_PATH}"
echo "    ✓ $(file "${BIN_PATH}" | sed 's|.*: ||')"

# ---------- 2. icon ----------
echo "▸ [2/8] Generating .icns from bundle/icon.png"
ICONSET="${DIST_DIR}/AppIcon.iconset"
mkdir -p "${ICONSET}"
for size in 16 32 128 256 512; do
    sips -z "${size}" "${size}" "${ICON_PNG}" \
        --out "${ICONSET}/icon_${size}x${size}.png" >/dev/null
    sips -z "$((size*2))" "$((size*2))" "${ICON_PNG}" \
        --out "${ICONSET}/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "${ICONSET}" -o "${DIST_DIR}/AppIcon.icns"
rm -rf "${ICONSET}"

# ---------- 3. .app bundle ----------
echo "▸ [3/8] Assembling .app bundle"
APP_BUNDLE="${DIST_DIR}/${APP_NAME}.app"
mkdir -p "${APP_BUNDLE}/Contents/MacOS" "${APP_BUNDLE}/Contents/Resources"
mv "${BIN_PATH}" "${APP_BUNDLE}/Contents/MacOS/${BINARY_NAME}"
mv "${DIST_DIR}/AppIcon.icns" "${APP_BUNDLE}/Contents/Resources/AppIcon.icns"
echo "APPL????" > "${APP_BUNDLE}/Contents/PkgInfo"
sed "s/__VERSION__/${VERSION}/g" "${BUNDLE_DIR}/Info.plist" \
    > "${APP_BUNDLE}/Contents/Info.plist"

# ---------- 4. codesign ----------
echo "▸ [4/8] Code-signing with Hardened Runtime"
codesign --force --options runtime --timestamp \
    --entitlements "${BUNDLE_DIR}/entitlements.plist" \
    --sign "${SIGNING_IDENTITY}" \
    "${APP_BUNDLE}"
codesign --verify --deep --strict --verbose=2 "${APP_BUNDLE}" 2>&1 | tail -3

# ---------- 5. DMG ----------
echo "▸ [5/8] Building DMG"
DMG_NAME="${APP_NAME}-${VERSION}-arm64.dmg"
DMG_PATH="${DIST_DIR}/${DMG_NAME}"

if command -v create-dmg >/dev/null 2>&1; then
    create-dmg \
        --volname "${APP_NAME} ${VERSION}" \
        --window-pos 200 120 \
        --window-size 600 400 \
        --icon-size 100 \
        --icon "${APP_NAME}.app" 175 190 \
        --hide-extension "${APP_NAME}.app" \
        --app-drop-link 425 190 \
        --no-internet-enable \
        "${DMG_PATH}" \
        "${APP_BUNDLE}" >/dev/null
else
    echo "    (tip: brew install create-dmg for a prettier window — falling back to hdiutil)"
    hdiutil create -volname "${APP_NAME} ${VERSION}" -srcfolder "${APP_BUNDLE}" \
        -ov -format UDZO "${DMG_PATH}" >/dev/null
fi

# ---------- 6. sign DMG ----------
echo "▸ [6/8] Code-signing DMG"
codesign --force --sign "${SIGNING_IDENTITY}" --timestamp "${DMG_PATH}"

if [ "${SKIP_NOTARIZE}" = "true" ]; then
    echo ""
    echo "✓ Local build complete (unnotarized — Gatekeeper will warn on first open)"
    echo "  ${DMG_PATH}"
    ls -lh "${DMG_PATH}"
    exit 0
fi

# ---------- 7. notarize ----------
echo "▸ [7/8] Submitting to Apple notarization (typically 1-5 min)"
xcrun notarytool submit "${DMG_PATH}" \
    --keychain-profile "${NOTARIZE_PROFILE}" \
    --wait

# ---------- 8. staple ----------
echo "▸ [8/8] Stapling notarization ticket"
xcrun stapler staple "${DMG_PATH}"

# ---------- final verification ----------
echo ""
echo "▸ Final Gatekeeper assessment"
spctl -a -t open --context context:primary-signature -v "${DMG_PATH}" 2>&1 | tail -5

echo ""
echo "✓ ${APP_NAME} ${VERSION} ready for distribution"
echo "  ${DMG_PATH}"
ls -lh "${DMG_PATH}"
