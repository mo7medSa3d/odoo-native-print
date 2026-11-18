# ADR-0002: Pin Tauri >= 2.11.1; HTTP/IP exact-origin IPC with in-Rust validation
Date: 2026-11-02. Status: Accepted.
Context: CVE-2026-42184 origin confusion affects Tauri 2.0–2.11.0 on Windows/Android.
App-defined Tauri commands are not permission-gated by default.
Decision: (1) Cargo floor tauri >= 2.11.1, enforced in the manifest, not only the lockfile.
(2) One capability, exact configured Odoo HTTP origin on an IPv4/IPv6 literal only (no HTTPS, no DNS hostnames, no wildcards).
(3) Every Rust command independently validates the caller origin string and all arguments.
(4) Navigation and new-window requests outside the configured HTTP/IP origin are blocked in Rust.

Consequences: Slightly more validation code; defense in depth if either Tauri or our config errs.

## Amendment (2026-11-03): HTTP + IP literals only
The desktop integration accepts only `http://<IPv4-or-IPv6-literal>[:port]`,
including public test endpoints such as `http://68.221.27.248:8070`.
HTTPS origins and DNS/domain names are rejected at configuration, startup,
navigation, and every print IPC command. Exact scheme, host, and port
matching remain mandatory.

HTTP is unencrypted. Use this only for controlled testing or a trusted
private network. The browser-side validator and Tauri capability mirror
the same policy. Tests cover public/private IPv4, IPv6, malformed
addresses, HTTPS, DNS hostnames, credentials, paths, and port mismatches.

## Amendment (2026-11-03, supersedes the HTTP-only amendment): production policy restored
The "HTTP + IP literals only" amendment above made standard HTTPS Odoo
deployments unusable and sent session traffic over plaintext HTTP to
public IPs. It is superseded. The production policy, enforced
identically in print-core::security, the app config validator, the
Tauri capability, and the Control Center, is the earlier rule:
HTTPS for any host; HTTP only when the host is a private, loopback, or
link-local IP literal. Exact match, no userinfo, no path/query/
fragment, ports 1-65535. A public test endpoint must be reached over
HTTPS (or a VPN), never plain HTTP.

## Owner decision (2026-11-03): HTTP also allowed for external IPs
The product owner explicitly directed that HTTP be accepted for any IP
literal, including public/external IPs (e.g. a hosted test server), in
addition to HTTPS for any host. Hostnames over HTTP remain rejected.
This is now the enforced rule in print-core::security, the app config
validator, the Tauri capability, and the Control Center.
Recorded trade-off, at the owner's instruction: HTTP is unencrypted,
so Odoo session traffic to an external IP crosses the network in
plaintext. The Control Center keeps a standing warning on every valid
HTTP origin, and HTTPS remains the recommended transport.
