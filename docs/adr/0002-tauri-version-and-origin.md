# ADR-0002: Pin Tauri >= 2.11.1; exact-origin IPC with in-Rust validation
Date: 2026-11-02. Status: Accepted.
Context: CVE-2026-42184 origin confusion affects Tauri 2.0–2.11.0 on Windows/Android.
App-defined Tauri commands are not permission-gated by default.
Decision: (1) Cargo floor tauri >= 2.11.1, enforced in the manifest, not only the lockfile.
(2) One capability, exact configured Odoo origin (HTTPS, no wildcards), print commands only.
(3) Every Rust command independently validates the caller origin string and all arguments.
(4) Navigation and new-window requests outside the origin are blocked in Rust.
Consequences: Slightly more validation code; defense in depth if either Tauri or our config errs.
