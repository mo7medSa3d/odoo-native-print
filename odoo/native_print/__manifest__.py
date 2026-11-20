{
    "name": "Yaseir Print",
    "version": "19.0.2.24.0",
    "category": "Productivity",
    "summary": "Zero-gateway local printing for Odoo reports, labels, and POS receipts",
    "description": """
Yaseir Print — direct local printing for Odoo reports, labels, and POS
receipts through the Yaseir Print desktop app, with no gateway, local
agent service, or cloud relay in the print path.

Printers are discovered live on each workstation by the desktop app.
Odoo maintains a per-company operational print job log (scoped by user
and company) with one-click reprints and an auto-synced printer picker
registry.
""",
    "author": "Yaseir",
    "depends": ["web"],
    "data": [
        "security/native_print_groups.xml",
        "security/ir.model.access.csv",
        "security/native_print_rules.xml",
        "views/native_print_views.xml",
        "views/login_fallback_templates.xml",
        "data/native_print_cron.xml",
    ],
    "assets": {
        "web.assets_backend": [
            "native_print/static/src/scss/native_print.scss",
            "native_print/static/src/js/native_print_client.js",
            "native_print/static/src/js/native_print_handler.js",
            "native_print/static/src/js/native_print_systray.js",
            "native_print/static/src/xml/native_print_systray.xml",
        ],
        # The POS bundle only exists when point_of_sale is installed and
        # only loads inside the POS app, so the POS extension needs no
        # hard dependency on point_of_sale for backend-only users.
        "point_of_sale._assets_pos": [
            "native_print/static/src/js/native_print_client.js",
            "native_print/static/src/js/pos_native_print.js",
        ],
    },
    "images": ["static/description/icon.png"],
    "installable": True,
    "application": True,
    "license": "LGPL-3",
    "post_init_hook": "post_init_hook",
}
