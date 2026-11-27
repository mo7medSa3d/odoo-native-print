"""Link existing job-log rows to the printer registry (19.0.2.6.0).

Jobs recorded before this version carry the printer as free text
(`device`). The log's printer field is now a Many2one picker fed by
the auto-synced registry, so convert each distinct legacy printer
name per company into a registry row and link the jobs to it.
"""
import logging

_logger = logging.getLogger(__name__)


def migrate(cr, version):
    from odoo import SUPERUSER_ID

    cr.execute(
        """
        INSERT INTO native_print_device
            (name, company_id, create_uid, write_uid, create_date, write_date)
        SELECT DISTINCT
            LEFT(BTRIM(j.device), 256),
            j.company_id,
            %s,
            %s,
            NOW() AT TIME ZONE 'UTC',
            NOW() AT TIME ZONE 'UTC'
        FROM native_print_job j
        WHERE j.device_id IS NULL
          AND j.device IS NOT NULL
          AND BTRIM(j.device) <> ''
          AND j.company_id IS NOT NULL
        ON CONFLICT (name, company_id) DO NOTHING
        """,
        (SUPERUSER_ID, SUPERUSER_ID),
    )
    cr.execute(
        """
        UPDATE native_print_job j
        SET device_id = d.id
        FROM native_print_device d
        WHERE j.device_id IS NULL
          AND j.device IS NOT NULL
          AND BTRIM(j.device) <> ''
          AND j.company_id IS NOT NULL
          AND d.name = LEFT(BTRIM(j.device), 256)
          AND d.company_id = j.company_id
        """
    )
    _logger.info(
        "native_print: linked %d legacy job(s) to the printer registry",
        cr.rowcount,
    )
