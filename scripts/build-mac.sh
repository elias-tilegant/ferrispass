#!/usr/bin/env bash
#
# Builds a notarized, stapled FerrisPass.dmg from source.
#
# Apple Silicon (arm64) only. Intel Macs are not supported in v0.1.0 - adding
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
#   9. Tarball + update manifest (optionally minisign-sign both, the DMG
#      too, and write SHA256SUMS)
#  10. Final spctl Gatekeeper assessment
#
# Flags:
#   --skip-notarize    Skip steps 7, 8 and 10. Useful for local iteration where
#                      waiting ~3 min for Apple is friction. Step 9 still runs,
#                      so the updater artefacts exist, but they stay unsigned
#                      unless `--sign-update` is run on them.
#   --sign-update      Sign existing updater artefacts only. This deliberately
#                      runs separately from `cargo build` in CI so build scripts
#                      cannot read the updater signing key or passphrase. It
#                      signs only when the disk image carries a valid
#                      notarization ticket; a dry run with a throwaway key
#                      sets FERRISPASS_ALLOW_UNSTAPLED=1 to skip that check.
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
CLI_BINARY_NAME="ferrispass-cli"
BUNDLE_ID="rs.ferrispass.app"
TEAM_ID="5GAMHB3974"
SIGNING_IDENTITY="Developer ID Application: Sonar Analytics - FZCO (${TEAM_ID})"
NOTARIZE_PROFILE="ferrispass-notarize"
MIN_MACOS="26.0"

# ---------- args ----------
SKIP_NOTARIZE=false
SIGN_UPDATE_ONLY=false
for arg in "$@"; do
    case $arg in
        --skip-notarize) SKIP_NOTARIZE=true ;;
        --sign-update) SIGN_UPDATE_ONLY=true ;;
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

# ---------- read version from Cargo.toml ----------
VERSION=$(grep '^version' "${PROJECT_ROOT}/Cargo.toml" | head -1 \
    | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
if [ -z "${VERSION}" ]; then
    echo "✗ Could not parse version from Cargo.toml" >&2
    exit 1
fi
# Info.plist takes numbers only: CFBundleShortVersionString and
# CFBundleVersion are one to three integers separated by periods. A
# pre-release such as 0.10.0-pre.3 is therefore 0.10.0 in both keys. The
# full version stays in the binary, which the updater compares and the About
# box shows, and in the file names, the manifest and the download URL.
BUNDLE_VERSION="${VERSION%%[-+]*}"
if ! [[ "${BUNDLE_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "✗ Version ${VERSION} does not start with MAJOR.MINOR.PATCH" >&2
    exit 1
fi

MINISIGN_KEY="${MINISIGN_KEY:-${HOME}/.ferrispass/minisign.key}"
TAR_NAME="${APP_NAME}-${VERSION}-arm64.app.tar.gz"
TAR_PATH="${DIST_DIR}/${TAR_NAME}"
SIG_PATH="${TAR_PATH}.minisig"
MANIFEST="${DIST_DIR}/update.json"
MANIFEST_SIG="${MANIFEST}.minisig"
CLI_TAR_NAME="FerrisPass-CLI-${VERSION}-aarch64-apple-darwin.tar.gz"
CLI_TAR_PATH="${DIST_DIR}/${CLI_TAR_NAME}"
CLI_SIG_PATH="${CLI_TAR_PATH}.minisig"
DMG_NAME="${APP_NAME}-${VERSION}-arm64.dmg"
DMG_PATH="${DIST_DIR}/${DMG_NAME}"
DMG_SIG_PATH="${DMG_PATH}.minisig"
SUMS_PATH="${DIST_DIR}/SHA256SUMS"

# Signs one file with the updater key. The passphrase comes from
# MINISIGN_PASSWORD when it is set (CI), otherwise minisign asks for it.
minisign_sign() {
    if [ -n "${MINISIGN_PASSWORD:-}" ]; then
        printf '%s\n' "${MINISIGN_PASSWORD}" \
            | minisign -S -s "${MINISIGN_KEY}" -m "$1" >/dev/null
    else
        minisign -S -s "${MINISIGN_KEY}" -m "$1"
    fi
}

sign_update_artifacts() {
    if ! command -v minisign >/dev/null 2>&1; then
        echo "✗ minisign is required to sign updater artefacts" >&2
        exit 1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        echo "✗ jq is required to sign updater artefacts" >&2
        exit 1
    fi
    if [ ! -f "${MINISIGN_KEY}" ]; then
        echo "✗ Minisign private key not found: ${MINISIGN_KEY}" >&2
        exit 1
    fi
    if [ ! -f "${TAR_PATH}" ] || [ ! -f "${CLI_TAR_PATH}" ] || [ ! -f "${MANIFEST}" ] \
        || [ ! -f "${DMG_PATH}" ]; then
        echo "✗ Missing updater artefacts in ${DIST_DIR}; run a release build first" >&2
        exit 1
    fi
    # Installed apps accept whatever this key signs, so it signs only what
    # Apple has notarized, and the stapled ticket on the disk image is the
    # proof. Only a dry run with a throwaway key may skip this.
    if [ "${FERRISPASS_ALLOW_UNSTAPLED:-}" = "1" ]; then
        echo "    ! FERRISPASS_ALLOW_UNSTAPLED=1: signing without a notarization ticket" >&2
    elif ! xcrun stapler validate -q "${DMG_PATH}"; then
        echo "✗ ${DMG_NAME} carries no valid notarization ticket; refusing to sign" >&2
        echo "  (a dry run with a throwaway key may set FERRISPASS_ALLOW_UNSTAPLED=1)" >&2
        exit 1
    fi

    local manifest_version manifest_size expected_url actual_size
    manifest_version="$(jq -er '.version' "${MANIFEST}")"
    manifest_size="$(jq -er '.platforms["macos-aarch64"].size' "${MANIFEST}")"
    actual_size="$(wc -c < "${TAR_PATH}" | tr -d '[:space:]')"
    expected_url="https://github.com/elias-tilegant/ferrispass/releases/download/v${VERSION}/${TAR_NAME}"
    if [ "${manifest_version}" != "${VERSION}" ]; then
        echo "✗ Refusing to sign manifest ${manifest_version} as release ${VERSION}" >&2
        exit 1
    fi
    if [ "${manifest_size}" != "${actual_size}" ]; then
        echo "✗ Refusing to sign manifest size ${manifest_size}; payload is ${actual_size} bytes" >&2
        exit 1
    fi
    if ! jq -e --arg url "${expected_url}" \
        '.platforms["macos-aarch64"].url == $url and .platforms["macos-aarch64"].format == "app"' \
        "${MANIFEST}" >/dev/null; then
        echo "✗ Manifest target does not match ${TAR_NAME}" >&2
        exit 1
    fi

    rm -f "${SIG_PATH}" "${CLI_SIG_PATH}" "${MANIFEST_SIG}" "${DMG_SIG_PATH}" "${SUMS_PATH}"
    minisign_sign "${TAR_PATH}"
    minisign_sign "${CLI_TAR_PATH}"

    # cargo-packager-updater expects base64(contents-of-.minisig) in the
    # platform record. Updating only that field preserves signed release notes.
    local sig_b64
    sig_b64="$(base64 < "${SIG_PATH}" | tr -d '\n')"
    jq --arg sig "${sig_b64}" \
        '.platforms["macos-aarch64"].signature = $sig' \
        "${MANIFEST}" > "${MANIFEST}.tmp"
    mv "${MANIFEST}.tmp" "${MANIFEST}"

    # Sign the final manifest bytes after every field, including release notes
    # and the payload signature, has been fixed.
    minisign_sign "${MANIFEST}"

    # For people who download the disk image by hand rather than through the
    # updater, which reads neither: a signature with the same key, and the
    # sums of everything they might download. Relative names, so
    # `shasum -a 256 -c SHA256SUMS` works in the download folder.
    minisign_sign "${DMG_PATH}"
    (cd "${DIST_DIR}" && shasum -a 256 "${DMG_NAME}" "${TAR_NAME}" "${CLI_TAR_NAME}") \
        > "${SUMS_PATH}"

    minisign -V -q -p "${PROJECT_ROOT}/bundle/minisign-pub.txt" -m "${TAR_PATH}"
    minisign -V -q -p "${PROJECT_ROOT}/bundle/minisign-pub.txt" -m "${CLI_TAR_PATH}"
    minisign -V -q -p "${PROJECT_ROOT}/bundle/minisign-pub.txt" -m "${MANIFEST}"
    minisign -V -q -p "${PROJECT_ROOT}/bundle/minisign-pub.txt" -m "${DMG_PATH}"
    (cd "${DIST_DIR}" && shasum -a 256 -c -s SHA256SUMS)
    echo "    ✓ ${TAR_NAME}.minisig"
    echo "    ✓ ${CLI_TAR_NAME}.minisig"
    echo "    ✓ update.json.minisig"
    echo "    ✓ ${DMG_NAME}.minisig"
    echo "    ✓ SHA256SUMS"
}

if [ "${SIGN_UPDATE_ONLY}" = "true" ]; then
    sign_update_artifacts
    exit 0
fi

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

echo "▸ ${APP_NAME} ${VERSION}"
echo "  Info.plist:  ${BUNDLE_VERSION}"
echo "  bundle id:   ${BUNDLE_ID}"
echo "  team id:     ${TEAM_ID}"
echo "  min macOS:   ${MIN_MACOS}"
echo "  notarize:    $([ "${SKIP_NOTARIZE}" = "true" ] && echo "skipped" || echo "yes")"
echo ""

# Build for the OS floor we declare in Info.plist so the binary actually runs there.
export MACOSX_DEPLOYMENT_TARGET="${MIN_MACOS}"

# A macOS 26 floor needs the macOS 26 SDK (Xcode 26) or newer. Which Xcode is
# selected depends on the machine and, in CI, on the runner image, so check it
# here, before anything is built or signed, instead of finding out on a tag.
SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version)"
if [ "${SDK_VERSION%%.*}" -lt "${MIN_MACOS%%.*}" ]; then
    echo "✗ macOS SDK ${SDK_VERSION} is older than the deployment target ${MIN_MACOS}." >&2
    echo "  Select Xcode ${MIN_MACOS%%.*} or newer with xcode-select, then build again." >&2
    exit 1
fi
echo "  macOS SDK:   ${SDK_VERSION}"

rm -rf "${DIST_DIR}"
mkdir -p "${DIST_DIR}"

# ---------- 1. arm64 binary ----------
echo "▸ [1/9] Building arm64 binary"
rustup target add aarch64-apple-darwin >/dev/null
cargo build --release --target aarch64-apple-darwin

BIN_PATH="${DIST_DIR}/${BINARY_NAME}"
CLI_BIN_PATH="${DIST_DIR}/${CLI_BINARY_NAME}"
# Cargo writes to CARGO_TARGET_DIR when it is set, so the binaries are read
# from there too; a relative path means the same to both, since both run in
# the current directory.
RELEASE_DIR="${CARGO_TARGET_DIR:-${PROJECT_ROOT}/target}/aarch64-apple-darwin/release"
cp "${RELEASE_DIR}/${BINARY_NAME}" "${BIN_PATH}"
cp "${RELEASE_DIR}/${CLI_BINARY_NAME}" "${CLI_BIN_PATH}"
echo "    ✓ $(file "${BIN_PATH}" | sed 's|.*: ||')"

# ---------- 2. icon ----------
echo "▸ [2/9] Generating .icns from bundle/icon.png"
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
echo "▸ [3/9] Assembling .app bundle"
APP_BUNDLE="${DIST_DIR}/${APP_NAME}.app"
mkdir -p "${APP_BUNDLE}/Contents/MacOS" "${APP_BUNDLE}/Contents/Resources"
mv "${BIN_PATH}" "${APP_BUNDLE}/Contents/MacOS/${BINARY_NAME}"
cp "${CLI_BIN_PATH}" "${APP_BUNDLE}/Contents/MacOS/${CLI_BINARY_NAME}"
mv "${DIST_DIR}/AppIcon.icns" "${APP_BUNDLE}/Contents/Resources/AppIcon.icns"
# The OFL (JetBrains Mono) and the ISC license (Lucide icons) require their
# texts to travel with every copy. Copied before signing, so the seal covers them.
mkdir -p "${APP_BUNDLE}/Contents/Resources/Licenses"
cp "${PROJECT_ROOT}/assets/licenses/"*.txt "${APP_BUNDLE}/Contents/Resources/Licenses/"
echo "APPL????" > "${APP_BUNDLE}/Contents/PkgInfo"
sed "s/__VERSION__/${BUNDLE_VERSION}/g" "${BUNDLE_DIR}/Info.plist" \
    > "${APP_BUNDLE}/Contents/Info.plist"

# ---------- 4. codesign ----------
echo "▸ [4/9] Code-signing with Hardened Runtime"
codesign --force --options runtime --timestamp \
    --sign "${SIGNING_IDENTITY}" "${CLI_BIN_PATH}"
codesign --force --options runtime --timestamp \
    --sign "${SIGNING_IDENTITY}" "${APP_BUNDLE}/Contents/MacOS/${CLI_BINARY_NAME}"
codesign --force --options runtime --timestamp \
    --entitlements "${BUNDLE_DIR}/entitlements.plist" \
    --sign "${SIGNING_IDENTITY}" \
    "${APP_BUNDLE}"
codesign --verify --deep --strict --verbose=2 "${APP_BUNDLE}" 2>&1 | tail -3

# ---------- 5. DMG ----------
echo "▸ [5/9] Building DMG"

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
    echo "    (tip: brew install create-dmg for a prettier window - falling back to hdiutil)"
    hdiutil create -volname "${APP_NAME} ${VERSION}" -srcfolder "${APP_BUNDLE}" \
        -ov -format UDZO "${DMG_PATH}" >/dev/null
fi

# ---------- 6. sign DMG ----------
echo "▸ [6/9] Code-signing DMG"
codesign --force --sign "${SIGNING_IDENTITY}" --timestamp "${DMG_PATH}"

if [ "${SKIP_NOTARIZE}" = "true" ]; then
    echo "▸ [7-8/9] Skipping notarization and stapling (--skip-notarize)"
else
    # ---------- 7. notarize ----------
    echo "▸ [7/9] Submitting to Apple notarization (typically 1-5 min)"
    xcrun notarytool submit "${DMG_PATH}" \
        --keychain-profile "${NOTARIZE_PROFILE}" \
        --wait

    # ---------- 8. staple ----------
    echo "▸ [8/9] Stapling notarization ticket"
    xcrun stapler staple "${DMG_PATH}"
fi

# ---------- 9. update tarball + manifest ----------
# Produces the artefacts the in-app auto-updater fetches:
#   - ${APP_NAME}-${VERSION}-arm64.app.tar.gz   (the bundle to install)
#   - update.json                               (manifest read by the updater)
# When a minisign key is available locally and the build is notarized, this
# also signs the payload, the complete manifest and the disk image, and writes
# SHA256SUMS. CI calls `--sign-update` in a later secret-bearing step.
#
echo "▸ [9/9] Generating .app.tar.gz + update manifest"

if ! command -v jq >/dev/null 2>&1; then
    echo "✗ jq is required to generate the update manifest" >&2
    exit 1
fi

tar czf "${TAR_PATH}" -C "${DIST_DIR}" "${APP_NAME}.app"
tar czf "${CLI_TAR_PATH}" -C "${DIST_DIR}" "${CLI_BINARY_NAME}"
DOWNLOAD_URL="https://github.com/elias-tilegant/ferrispass/releases/download/v${VERSION}/${TAR_NAME}"
BUNDLE_SIZE="$(wc -c < "${TAR_PATH}" | tr -d '[:space:]')"

# The signature is filled by `sign_update_artifacts` after release notes are
# injected. An empty signature is never accepted by the client.
jq -n \
    --arg version "${VERSION}" \
    --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg url "${DOWNLOAD_URL}" \
    --argjson size "${BUNDLE_SIZE}" \
    '{
        version: $version,
        pub_date: $pub_date,
        platforms: {
            "macos-aarch64": {
                signature: "",
                url: $url,
                format: "app",
                size: $size
            }
        }
    }' > "${MANIFEST}"

echo "    ✓ ${TAR_NAME}"
echo "    ✓ update.json (unsigned)"

# An unnotarized build is not signed for the updater automatically, even
# with a key at the default path: installed apps accept a signed manifest,
# and one should only exist for a build Apple has checked. `--sign-update`
# refuses it as well, unless a dry run with a throwaway key sets
# FERRISPASS_ALLOW_UNSTAPLED=1.
if [ "${SKIP_NOTARIZE}" != "true" ] \
    && command -v minisign >/dev/null 2>&1 && [ -f "${MINISIGN_KEY}" ]; then
    sign_update_artifacts
else
    echo "    (updater signing deferred; run '$0 --sign-update')"
fi

if [ "${SKIP_NOTARIZE}" = "true" ]; then
    echo ""
    echo "✓ Local build complete (unnotarized - Gatekeeper will warn on first open)"
    ls -lh "${DMG_PATH}" "${TAR_PATH}" "${CLI_TAR_PATH}" "${MANIFEST}"
    exit 0
fi

# ---------- final verification ----------
echo ""
echo "▸ Final Gatekeeper assessment"
spctl -a -t open --context context:primary-signature -v "${DMG_PATH}" 2>&1 | tail -5

echo ""
echo "✓ ${APP_NAME} ${VERSION} ready for distribution"
ls -lh "${DIST_DIR}/"*.dmg 2>/dev/null || true
ls -lh "${DIST_DIR}/"*.tar.gz 2>/dev/null || true
ls -lh "${DIST_DIR}/"update.json 2>/dev/null || true
ls -lh "${DIST_DIR}/"*.minisig 2>/dev/null || true
ls -lh "${SUMS_PATH}" 2>/dev/null || true
