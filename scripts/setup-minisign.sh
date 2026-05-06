#!/usr/bin/env bash
#
# One-time setup for the FerrisPass auto-update signing infrastructure.
#
# What this does:
#   1. Verifies `minisign` CLI is installed (offers brew install if not)
#   2. Generates a fresh Ed25519 keypair in ~/.ferrispass/minisign.{key,pub}
#   3. Copies the public key into bundle/minisign-pub.txt (committed to repo)
#   4. Prints instructions for setting GitHub Secrets so CI can sign releases
#
# Run this ONCE per project. Re-running rotates the keypair, which means every
# user's installed app stops accepting updates until they manually reinstall.
# Don't do that lightly.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PUBKEY_FILE="${PROJECT_ROOT}/bundle/minisign-pub.txt"
PRIVKEY_DIR="${HOME}/.ferrispass"
PRIVKEY_FILE="${PRIVKEY_DIR}/minisign.key"

# ---------- minisign installed? ----------
if ! command -v minisign >/dev/null 2>&1; then
    echo "minisign not found. Install with:"
    echo "  brew install minisign"
    exit 1
fi

# ---------- already a real key? ----------
if [ -f "${PUBKEY_FILE}" ] && ! grep -q "PLACEHOLDER" "${PUBKEY_FILE}"; then
    echo "✗ ${PUBKEY_FILE} already contains a real public key."
    echo "  Re-running this script would invalidate update signatures for"
    echo "  every existing FerrisPass install. Aborting."
    echo ""
    echo "  If you really need to rotate keys, delete ${PUBKEY_FILE}"
    echo "  manually and re-run."
    exit 1
fi

# ---------- generate keypair ----------
mkdir -p "${PRIVKEY_DIR}"
chmod 700 "${PRIVKEY_DIR}"

if [ -f "${PRIVKEY_FILE}" ]; then
    echo "✗ ${PRIVKEY_FILE} already exists. Move or delete it first."
    exit 1
fi

echo "▸ Generating Ed25519 keypair (you'll be prompted for a passphrase)"
minisign -G -s "${PRIVKEY_FILE}" -p "${PRIVKEY_DIR}/minisign.pub"

# ---------- install public key into repo ----------
cp "${PRIVKEY_DIR}/minisign.pub" "${PUBKEY_FILE}"
echo "✓ Public key written to ${PUBKEY_FILE}"

# ---------- next-step instructions ----------
cat <<EOF

═══════════════════════════════════════════════════════════════════════
NEXT STEPS — set GitHub Secrets so CI can sign releases:

1. Read the private key (you'll paste this into a GitHub Secret):
   cat ${PRIVKEY_FILE}

2. In GitHub → Settings → Secrets and variables → Actions, create:

   • MINISIGN_PRIVATE_KEY — the contents of ${PRIVKEY_FILE}
   • MINISIGN_PASSWORD    — the passphrase you just set

3. Commit the new public key:
   git add bundle/minisign-pub.txt
   git commit -m "set minisign public key for update signing"
   git push

4. Back up ${PRIVKEY_FILE} somewhere safe (1Password, encrypted external
   drive, etc). If you lose it, you cannot ship signed updates ever again
   for this app, and every existing install would need to be reinstalled
   from scratch with a new key.

═══════════════════════════════════════════════════════════════════════
EOF
