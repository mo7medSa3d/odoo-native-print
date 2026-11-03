# ADR-0002: Pin Tauri >= 2.11.1; exact-origin IPC with in-Rust validation
Date: 2026-11-02. Status: Accepted.
Context: CVE-2026-42184 origin confusion affects Tauri 2.0–2.11.0 on Windows/Android.
App-defined Tauri commands are not permission-gated by default.
Decision: (1) Cargo floor tauri >= 2.11.1, enforced in the manifest, not only the lockfile.
(2) One capability, exact configured Odoo origin (HTTPS, no wildcards), print commands only.
(3) Every Rust command independently validates the caller origin string and all arguments.
(4) Navigation and new-window requests outside the origin are blocked in Rust.
Consequences: Slightly more validation code; defense in depth if either Tauri or our config errs.

## Amendment (2026-11-03): HTTP origins on private IPs
At the product owner's decision, the exact-origin rule now reads:
HTTPS for any host, OR HTTP when the host is a private, loopback, or
link-local IP literal (RFC 1918, 127.0.0.0/8, 169.254.0.0/16, ::1,
fc00::/7, fe80::/10). Rationale: most LAN Odoo installs are reached by
IP (e.g. http://192.168.1.20:8069) and have no TLS certificate; the
exact-match, no-userinfo, no-path rules and the in-Rust validation on
every command are unchanged, and the capability URL patterns mirror
the same two schemes. Hostnames and public IPs over HTTP remain
rejected. Enforced in print-core::security::origin_is_allowed with
unit tests (http_private_ip_passes_http_others_fail).
