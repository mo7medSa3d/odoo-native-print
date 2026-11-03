# PDFium (vendored PDF renderer)

PDF pages are rendered to bitmaps with PDFium, loaded dynamically at
runtime by `print-render` (module `pdfium`, direct FFI via `libloading`).
No PDFium installation is ever required on a target machine:

- **Windows:** `pdfium.dll` (x64) is vendored in this repository at
  `app/src-tauri/resources/pdfium.dll` and bundled as a resource into
  both the MSI and NSIS installers (`bundle.resources` in
  `tauri.conf.json`). At runtime the loader searches, in order:
  `NATIVE_PRINT_PDFIUM_PATH` (file or directory), the directory beside
  the executable, its `resources/` subdirectory, then system paths.
  A missing library is a loud job error, never a silent fallback.
- **Linux (dev/CI only):** `tools/fetch-pdfium.sh` downloads the pinned
  Linux build into `tools/pdfium/` (gitignored) so the render tests run
  against the real library.

## Pinned version

| Item | Value |
| --- | --- |
| Upstream | https://github.com/bblanchon/pdfium-binaries |
| Release tag | `chromium/8125` |
| PDFium version | 158.0.8125.0 |
| Windows archive | `pdfium-win-x64.tgz`, 3,904,987 bytes |
| Windows archive SHA256 | `9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b` |
| `pdfium.dll` | 7,586,816 bytes, SHA256 `4b0dec92555d69437bef0df37be630181ab3e83a6823ca264ede355a4f2ef994` |
| Linux archive | `pdfium-linux-x64.tgz`, 3,828,800 bytes |
| Linux archive SHA256 | `e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659` |
| `libpdfium.so` | 8,049,768 bytes, SHA256 `1c251efce460d67437a4c76d4be9033ae2f17bf5603f5bff1af10209b94ca723` |

Non-V8 build (no JavaScript engine): smaller and one less attack
surface; report PDFs do not need JS.

## License

PDFium is BSD-style licensed (Google). The license text ships beside
the DLL in the installer resources as `pdfium-LICENSE.txt` (copy of the
upstream `LICENSE` from the pinned archive) and is also in this repo at
`app/src-tauri/resources/pdfium-LICENSE.txt`. Third-party component
licenses are listed in the upstream archive under `licenses/`.

## Updating

1. Pick the new `chromium/<build>` tag; download win-x64 and linux-x64
   archives; record their SHA256 from the GitHub release API digests.
2. Replace `app/src-tauri/resources/pdfium.dll`, update the hashes and
   sizes in this file and in `tools/fetch-pdfium.sh`.
3. Re-run the render tests (`cargo test -p print-render`) on Linux and
   let Windows CI compile/run the GDI path.
