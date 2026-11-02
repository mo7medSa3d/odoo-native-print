# Research: Tauri 2 remote IPC, capabilities, on_download

Date accessed: 2026-11-02
Primary sources named in spec:
- https://v2.tauri.app/reference/acl/capability
- https://v2.tauri.app/security/capabilities/
- https://docs.rs/tauri/2/tauri/webview/struct.WebviewWindowBuilder.html

## Summary in my own words (verified via live search 2026-11-02)

1. Remote origins get NO IPC by default. IPC is local-app-only unless a capability
   file explicitly opts a remote origin in via `remote.urls` (URLPattern-based).
   Missing either the permission definition or the capability match means every
   `invoke` from that origin fails.
2. CRITICAL PITFALL: App-defined commands (`#[tauri::command]`) are NOT
   permission-gated by default, unlike core/plugin commands. Granting a remote
   origin any IPC therefore exposes every app command to that page unless each
   command validates origin and arguments itself in Rust. We must do both:
   capability scoping AND in-command validation.
3. Capabilities bind permissions to specific window/webview labels, optionally
   per platform. Least privilege: one capability, one window label, exact Odoo
   origin only, print commands only. No fs, shell, http, opener, dialog.
4. On Linux/Android, Tauri cannot distinguish an embedded iframe from the top
   window for remote capability purposes (documented caveat) — relevant to
   CVE-2024-35222 class (see threat model). On Windows/WebView2 we still must
   block iframes/navigation we do not control.
5. URL pattern syntax pitfalls: patterns are URLPattern, not simple globs.
   A pattern like `https://*.example.com` differs from an exact origin.
   We will use a single exact origin string from admin-locked config, tested
   with positive and negative cases in spike S1 (Windows required).

## Implications for this project
- Pin exact origin, HTTPS only, no wildcard subdomains.
- `on_navigation` handler in Rust: allow only the configured origin; open
  everything else externally or block it. Block new windows.
- Disable devtools in release builds.
- Fallbacks B (on_download) and C (Rust fetches report URL) remain in the
  design if remote IPC proves unreliable on the target Tauri version.
