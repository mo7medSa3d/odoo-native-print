{
    "name": "Yaseir Print",
    "version": "19.0.2.22.0",
    "category": "Productivity",
    "summary": "Zero-gateway local printing for Odoo reports, labels, and POS receipts",
    "description": """
Yaseir Print — local printing for Odoo from the desktop app, with no
gateway, no local agent service and no cloud in the print path.

The server side of this module is deliberately small: printers are
discovered live on each user's PC and never stored here. What the
server keeps is the print job LOG. Note that log rows are
CLIENT-REPORTED facts: the desktop app posts job outcomes to this
module, so a row records what the app reported, and managers should
treat it accordingly (it is an operational log, not a tamper-proof
audit trail). Job rows are scoped per user and per company; the
printer names in the log feed a per-company registry (pickers only).
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
}
