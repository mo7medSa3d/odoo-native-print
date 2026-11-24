#!/usr/bin/env bash
# Fetch the pinned PDFium binary for local / CI test runs (Linux x64).
#
# The Windows build does NOT use this script: pdfium.dll is vendored in
# the repository at app/src-tauri/resources/pdfium.dll and bundled into
# both installers (see docs/pdfium.md). This script exists so the Linux
# test suite can exercise the same renderer against libpdfium.so.
#
# Pinned release: bblanchon/pdfium-binaries chromium/8125
# (PDFium 158.0.8125.0). Both the archive and extracted libpdfium.so
# SHA256 digests are verified before use; a mismatch aborts the script.
# Installed layout after a run:
#   tools/pdfium/libpdfium.so
#   tools/pdfium/VERSION
# tools/pdfium/ is gitignored (a downloaded binary, not a source).
set -euo pipefail

TAG="chromium/8125"
TAG_ENC="chromium%2F8125"
TGZ="pdfium-linux-x64.tgz"
SHA256="e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659"
LIB_SHA256="1c251efce460d67437a4c76d4be9033ae2f17bf5603f5bff1af10209b94ca723"
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${TAG_ENC}/${TGZ}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${ROOT}/tools/pdfium"
mkdir -p "${DEST}"

if [ -f "${DEST}/libpdfium.so" ] && printf '%s  %s\n' "${LIB_SHA256}" "${DEST}/libpdfium.so" | sha256sum -c --status 2>/dev/null; then
    echo "pdfium ${TAG} already present and verified at ${DEST}/libpdfium.so"
    exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT INT TERM
echo "downloading ${URL}"
curl --proto '=https' --tlsv1.2 --connect-timeout 30 --max-time 300 --retry 3 -fsSL -o "${TMP}/${TGZ}" "${URL}"
printf '%s  %s\n' "${SHA256}" "${TMP}/${TGZ}" | sha256sum -c -
tar -xzf "${TMP}/${TGZ}" -C "${TMP}"
printf '%s  %s\n' "${LIB_SHA256}" "${TMP}/lib/libpdfium.so" | sha256sum -c -
cp "${TMP}/lib/libpdfium.so" "${DEST}/libpdfium.so.tmp"
mv -f "${DEST}/libpdfium.so.tmp" "${DEST}/libpdfium.so"
cp "${TMP}/VERSION" "${DEST}/VERSION.tmp"
mv -f "${DEST}/VERSION.tmp" "${DEST}/VERSION"
echo "installed PDFium ${TAG} -> ${DEST}/libpdfium.so"
