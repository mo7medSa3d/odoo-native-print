"""No-op (superseded before release).

19.0.2.5.0 briefly removed the server job log; the owner reversed
that decision the same day - the log stays and was rebuilt in
19.0.2.6.0 instead. This migration intentionally does nothing so
upgrades passing through this version keep their job history.
"""


def migrate(cr, version):
    pass
