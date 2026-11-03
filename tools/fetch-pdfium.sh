#!/bin/sh
# Fetch the pinned PDFium binary for local / CI test runs (Linux x64).
#
# The Windows build does NOT use this script: pdfium.dll is vendored in
# the repository at app/src-tauri/resources/pdfium.dll and bundled into
# both installers (see docs/pdfium.md). This script exists so the Linux
# test suite can exercise the same renderer against libpdfium.so.
#
# Pinned release: bblanchon/pdfium-binaries chromium/8125
# (PDFium 158.0.8125.0). The archive SHA256 is verified before use;
# a mismatch aborts the script. Installed layout after a run:
#   tools/pdfium/libpdfium.so
#   tools/pdfium/VERSION
# tools/pdfium/ is gitignored (a downloaded binary, not a source).
set -eu

TAG="chromium/8125"
TAG_ENC="chromium%2F8125"
TGZ="pdfium-linux-x64.tgz"
SHA256="e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659"
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${TAG_ENC}/${TGZ}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${ROOT}/tools/pdfium"
mkdir -p "${DEST}"

if [ -f "${DEST}/libpdfium.so" ] && [ -f "${DEST}/VERSION" ] && grep -q "${TAG}" "${DEST}/VERSION"; then
    echo "pdfium ${TAG} already present at ${DEST}/libpdfium.so"
    exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
echo "downloading ${URL}"
curl -fsSL -o "${TMP}/${TGZ}" "${URL}"
echo "${SHA256}  ${TMP}/${TGZ}" | sha256sum -c -
tar -xzf "${TMP}/${TGZ}" -C "${TMP}"
cp "${TMP}/lib/libpdfium.so" "${DEST}/libpdfium.so"
cp "${TMP}/VERSION" "${DEST}/VERSION"
echo "installed PDFium ${TAG} -> ${DEST}/libpdfium.so"
