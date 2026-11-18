"""19.0.2.24.0: ensure duplex_configured and capability columns exist."""


def migrate(cr, version):
    for col, col_type in (
        ("device_id", "integer"),
        ("source_type", "varchar DEFAULT 'report'"),
        ("source_model", "varchar"),
        ("source_ids", "varchar"),
        ("copies", "integer DEFAULT 1"),
        ("paper", "varchar"),
        ("duplex", "boolean DEFAULT false"),
        ("duplex_configured", "boolean DEFAULT false"),
        ("color", "boolean DEFAULT true"),
        ("mode", "varchar"),
    ):
        cr.execute(
            f'ALTER TABLE IF EXISTS "native_print_job" ADD COLUMN IF NOT EXISTS "{col}" {col_type}'
        )
    for col, col_type in (
        ("protocol", "varchar"),
        ("printer_class", "varchar"),
        ("forms_summary", "text"),
        ("trays_summary", "text"),
        ("caps_synced_at", "timestamp without time zone"),
    ):
        cr.execute(
            f'ALTER TABLE IF EXISTS "native_print_device" ADD COLUMN IF NOT EXISTS "{col}" {col_type}'
        )
