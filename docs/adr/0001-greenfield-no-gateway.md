# ADR-0001: Greenfield repo; no gateway or agent in the print path
Date: 2026-11-02. Status: Accepted.
Context: Spec says the repo may already contain a gateway/agent/Tauri shell/addon to inspect first.
Inspection (2026-11-02) found no pre-existing repo in the workspace — only the spec PDF.
Decision: Start a greenfield Rust workspace. No Next.js gateway, no Go agent, no localhost service
in the print path, per the locked architecture. The only network dependencies for printing are
the Odoo server on the company LAN and the printers themselves.
Consequences: Nothing to reuse; Tauri shell and Odoo addon are built fresh against the verified APIs.
