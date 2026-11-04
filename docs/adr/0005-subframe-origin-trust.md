# ADR-0005: Subframe origin trust is an accepted, bounded risk

Date: 2026-11-04. Status: Accepted.

Context: The deep-debug audit (round 2, app H1) verified against the
pinned tauri-2.12.5 sources that a command invocation is attributed
to the WEBVIEW's top-level URL only — Tauri exposes no frame identity
for an invoke. The remote capability therefore technically extends
to any subframe of the configured Odoo origin (for example an
`<iframe>` embedded by a third-party Odoo module or a helpdesk
widget), not just to Odoo's own page code. There is no supported API
in Tauri 2.12.5 to distinguish frames.

Decision: Accept the risk, bounded by four independent limits:

1. The origin itself is the trust boundary everywhere else already:
   only the exact configured Odoo origin (ADR-0002 policy) can reach
   ANY remote command; navigation away from it is blocked in Rust.
2. The remote command set is print-only and small (11 commands after
   1.4.8 removed clear_web_data_cmd from it): enqueue jobs, read job
   status, list this PC's printers. No queue administration, no
   settings writes, no diagnostics export, no web-data reset — those
   are local-page only, and every privileged command additionally
   enforces `check_local_only` in Rust, so a capability mistake alone
   cannot open them.
3. Every command re-validates the caller origin and all arguments in
   Rust; payload, page, copy and dimension caps are enforced server
   side of the bridge.
4. The impact ceiling of a hostile subframe is therefore: print jobs
   on printers attached to THIS PC, and disclosure of this PC's
   printer names/job states to the Odoo origin that already serves
   the page. No file access, no code execution, no data exfiltration
   channel beyond the origin itself.

Consequences: Installing third-party Odoo modules that embed
untrusted iframes on the same origin widens who can print to the
local PC; that is inherent to running Odoo inside a webview and is
accepted for the product's threat model (the customer's own Odoo is
trusted infrastructure). If Tauri adds per-frame invoke attribution,
this decision must be revisited and the remote set re-audited.
