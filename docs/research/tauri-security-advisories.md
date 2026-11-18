# Research: Tauri security advisories relevant to remote IPC

Date accessed: 2026-11-02
Source named in spec: https://advisories.gitlab.com/pkg/cargo/tauri/
Live search findings 2026-11-02:

## CVE-2026-42184 / GHSA-7gmj-67g7-phm9 — Origin confusion (CRITICAL for us)
- Affects tauri >= 2.0.0, <= 2.11.0 on Windows and Android. Fixed in 2.11.1.
- `is_local_url()` only checked the first dot-separated label of the host.
  On Windows/Android custom schemes map to `http://<scheme>.localhost/`, so
  `http://app.evil.com/` could be misclassified as a local origin and invoke
  local-only IPC commands.
- Mitigation for this project: hard floor `tauri = ">=2.11.1"` in Cargo.toml
  (not just lockfile), cargo audit in CI, and our own exact-origin check in
  every command — never rely solely on Tauri's local/remote classification.

## CVE-2024-35222 / GHSA-57fm-592m-34r7 — iframe IPC bypass
- Remote-origin iframes could access Tauri IPC without being allowed in
  capabilities (v1 `dangerousRemoteDomainIpcAccess` / v2 capabilities).
  Patched in 1.6.7 and 2.0.0-beta.19+.
- Mitigation: current Tauri, block unknown iframes/navigation, CSP frame
  restrictions where Odoo allows, in-command origin validation.

## GHSA-4wm2-cwcf-wwvp — Open redirect exposing IPC
- Open redirect could expose IPC to external sites (fixed 1.0.9 era).
- Mitigation: no open redirects in our shell, navigation allowlist in Rust,
  never follow a page-supplied URL outside the configured origin except the
  print-fetch path (fallback C) which validates scheme/host/path shape.

## Rule adopted
Every advisory class above gets a negative test in the Windows spike plan
(S1): evil subdomain, iframe, redirect, and non-configured origin must all
be rejected. See docs/security/threat-model.md.
