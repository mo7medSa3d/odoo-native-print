# ADR-0006: The embedded browser is a tabbed browser in one window

Date: 2026-11-05. Status: Accepted (owner brief, 2026-11-05).

Context: Until 1.4.x the app was a single webview. Odoo is a suite
of full-screen apps — above all the Point of Sale, which takes over
the screen and runs its own session logic — and a single webview
forced destructive navigation: opening the Control Center replaced
the Odoo page, and a floating Back/Forward/Reload pill injected
into pages was the only browser chrome. The owner ordered a real
tabbed browser: tabs at the top, restricted to the company's Odoo,
POS in its own tab, and a top navigation bar instead of the pill.

Decision (app 1.5.0):

1. One window hosts a local tab-bar webview (tabbar.html, 46 px)
   plus one child webview per tab, via Tauri's multi-webview API
   (the `unstable` feature, already enabled in Cargo.toml). The
   v1.4.1 caution that removed a child-webview toolbar is
   superseded by the owner's explicit order; the Windows build of
   this exact tree is verified on CI.
2. All content webviews share ONE data directory. Cookies and
   storage are therefore shared across tabs exactly like a normal
   browser profile, which matches how Odoo expects its session to
   be used (one login, every app). A tab is a view, never a
   separate identity.
3. Navigation stays restricted to the configured origin (the
   ADR-0002 policy enforced in Rust per webview). Two tab rules sit
   on top: a navigation into /pos* from a non-POS tab is diverted
   into a new POS tab, and an on-origin window.open becomes a tab;
   off-origin popups are denied.
4. Webviews are created only from setup or async commands (the
   documented Windows deadlock rule for WebView2 creation).
5. Tab titles are derived from routes in Rust, never from
   page-supplied text; the tab bar polls tab_list and renders
   from Rust snapshots — the bar is chrome, not content.
6. Command surface: 30 commands; remote reaches 12 (tab_open
   added to allow-app-shell — it validates the URL against the
   configured origin in Rust); tab management
   (tab_list/tab_switch/tab_close) is granted only via
   allow-toolbar to the tab bar and local pages; nav_back /
   nav_forward / nav_reload now act on the ACTIVE tab.
7. The floating nav pill is removed from the bridge. The Control
   Center opens in its own tab and never replaces an Odoo page.

Consequences: the Odoo page state survives Control Center visits;
POS and backend coexist; a one-time re-login is possible on
upgrade if the platform's default webview data directory differed
from the app data directory now used explicitly for all tabs.
