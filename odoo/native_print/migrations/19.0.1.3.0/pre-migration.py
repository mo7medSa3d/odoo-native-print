"""19.0.1.3.0: enforce unique(idempotency key, company) on native_print_job.

Older versions could accumulate duplicate log rows for one idempotency
key when two status posts raced. Keep the newest row per (name,
company_id) — it carries the latest reported state — and remove only
the older duplicate shadows so the unique index can be created. No job
is lost: exactly one row per job remains.
"""
import logging

_logger = logging.getLogger(__name__)


def migrate(cr, version):
    cr.execute(
        """
        DELETE FROM native_print_job a
        USING native_print_job b
        WHERE a.id < b.id
          AND a.name = b.name
          AND a.company_id = b.company_id
        """
    )
    if cr.rowcount:
        _logger.info(
            "native_print: removed %s duplicate job log rows", cr.rowcount
        )
