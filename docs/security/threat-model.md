# Threat Model (STRIDE) — v0.1, 2026-11-02

## Assets
Print job bytes (business documents), printer access, user Odoo session,
admin configuration (allowed origin), audit log, signing/update keys.

## Trust boundary (the critical one)
Remote web page (Odoo, semi-trusted, XSS-able)  --IPC-->  Rust (fully privileged)
                                     \--bytes-->  PDF renderer (hostile input)
                                     \--spool-->  Printers / LAN (TCP 9100, IPP)

## STRIDE
- Spoofing: evil origin / evil subdomain / iframe impersonating Odoo.
  Mitigations: exact-origin capability, tauri >= 2.11.1 (CVE-2026-42184),
  in-Rust exact origin check on every command, navigation allowlist, admin/GPO-locked origin.
- Tampering: page alters job params (copies, printer, paper) beyond server policy.
  Mitigations: Odoo is policy authority; Rust validates ranges; profile/rule resolution is server-driven; arguments schema-validated in Rust.
- Repudiation: Mitigations: append-only audit log per job (id, user, report, printer, pages, result), diagnostics bundle redacted.
- Information disclosure: job bytes/PII in logs, temp files.
  Mitigations: no job bytes or secrets in logs, redaction, per-session temp dirs, DPAPI/Credential Manager for tokens, no secrets to JS beyond need.
- Denial of service: 50MB+ PDFs, 10k pages, flood of jobs.
  Mitigations: hard limits on bytes/pages/dimensions/rate, bounded queue + backpressure, rendering in restricted worker (Windows job object — Windows phase), timeouts.
- Elevation: PDF/parser exploit, IPC command abuse.
  Mitigations: minimal command set (print only), no fs/shell/http/opener permissions, PDFium kept updated, fuzz parsing entry points, least-privilege spooler access.

## Residual risks (ranked, honest)
1. XSS on the Odoo origin itself can call the print IPC — printing is the
   intended capability, but policy must stay server-side so XSS cannot
   exfiltrate files or run code. Accepted with server-authority design.
2. Printer firmware / LAN printers are untrusted and weakly authenticated (9100/IPP).
3. PDFium 0-day — mitigated by worker isolation (Windows phase, not yet built).
