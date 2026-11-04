"""Link existing job-log rows to the printer registry (19.0.2.6.0).

Jobs recorded before this version carry the printer as free text
(`device`). The log's printer field is now a Many2one picker fed by
the auto-synced registry, so convert each distinct legacy printer
name per company into a registry row and link the jobs to it.
"""
import logging

_logger = logging.getLogger(__name__)


def migrate(cr, version):
    from odoo import SUPERUSER_ID, api

    env = api.Environment(cr, SUPERUSER_ID, {})
    Job = env["native.print.job"]
    Device = env["native.print.device"]
    jobs = Job.search([("device_id", "=", False),
                       ("device", "!=", False)])
    printers = {}
    linked = 0
    for job in jobs:
        key = (job.device, job.company_id.id)
        printer = printers.get(key)
        if printer is None:
            printer = Device.search([
                ("name", "=", job.device),
                ("company_id", "=", job.company_id.id),
            ], limit=1)
            if not printer:
                printer = Device.create({
                    "name": job.device,
                    "company_id": job.company_id.id,
                })
            printers[key] = printer
        job.with_context(native_print_status_sync=True).device_id = printer.id
        linked += 1
    _logger.info(
        "native_print: linked %d legacy job(s) to the printer registry",
        linked,
    )
