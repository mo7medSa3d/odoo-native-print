# Research: Odoo report handlers and download routes

Date accessed: 2026-11-02
Sources named in spec: Odoo PR 873 (odoo-dev), Odoo 19 source
addons/web/static/src/webclient/actions/action_service.js,
addons/web/controllers/report.py (routes /report/<converter>/<reportname>/<docids>, /report/download)

## Summary in my own words (verified via live search 2026-11-02)

1. The extension point is real and current: JS registers a handler with
   `registry.category("ir.actions.report handlers").add(name, handler)`.
   Handlers are iterated by the Action Manager; a handler returning a truthy
   result stops the default flow (including the default download). A falsy
   return lets Odoo continue as usual — exactly the normal-browser behavior
   the spec requires.
2. Upstream fix evidence: a handler returning `true` is an established pattern
   in Odoo's own webclient tests ("can use custom handlers for report actions",
   `report_action.test.js`), and `close_on_report_download` handling was fixed
   to work when a custom handler is used — so our handler must also respect
   `action.close_on_report_download`.
3. Custom report_type handlers (e.g. xlsx examples in the wild) follow the
   same shape: inspect `action.report_type`, act only when relevant, otherwise
   return falsy.
4. Odoo 19 notes for the addon: manifest version `19.0.x`, access rights in
   `security/ir.model.access.csv` (the `ir.access` change lands in 20, not 19),
   JSON routes use `type='jsonrpc'` style in newer versions — verify against
   the actual 19 server in spike S2 before shipping the addon routes.

## Still to verify on a real Odoo 19 (BLOCKED-NEEDS-ODOO, spike S2)
- Exact action fields delivered (report_name, ids/context/data) for form,
  list, and wizard launches.
- qweb-html and qweb-text behavior, multi-document reports, POS flows.
- Whether fetching the report URL with the user session from JS reliably
  returns PDF bytes for all report types we target.
