{
    "name": "Native Print",
    "version": "19.0.1.1.0",
    "category": "Tools",
    "summary": "Print Odoo reports on local printers from the Native Print app",
    "depends": ["web"],
    "data": [
        "security/native_print_groups.xml",
        "security/ir.model.access.csv",
        "security/native_print_rules.xml",
        "views/native_print_views.xml",
    ],
    "assets": {
        "web.assets_backend": [
            "native_print/static/src/js/native_print_handler.js",
        ],
        # The POS bundle only exists when point_of_sale is installed and
        # only loads inside the POS app, so the POS extension needs no
        # hard dependency on point_of_sale for backend-only users.
        "point_of_sale._assets_pos": [
            "native_print/static/src/js/pos_native_print.js",
        ],
    },
    "installable": True,
    "license": "LGPL-3",
}
