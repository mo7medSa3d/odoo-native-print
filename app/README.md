# App shell (Tauri 2) — Windows build only

Status: SCAFFOLD, not built. This crate is deliberately excluded from the
root Cargo workspace because it can only compile and run on Windows
(WebView2). On a Windows runner:

```
cargo install tauri-cli --version "^2"
cd app && cargo tauri build
```

Design rules already encoded in the scaffold config:
- `withGlobalTauri: false` — no global Tauri object leaks to pages.
- One capability (`capabilities/main.json`): print commands only, remote
  access only from the exact configured Odoo origin (placeholder
  `odoo.example.invalid` must be replaced by the installer/config step;
  the app refuses to start if it still contains `.invalid`).
- No fs / shell / http / opener permissions, anywhere.
- Every command in src/commands.rs validates the caller origin with
  print-core::security::origin_is_allowed before touching the queue.
- Navigation handler blocks anything outside the configured origin.
- Bundle targets MSI + NSIS, WebView2 offline installer mode (enterprise,
  offline sites), silent = true.
