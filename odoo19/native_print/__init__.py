from . import models
from . import controllers


def post_init_hook(env):
    # _register_hook returns early before the table exists on a fresh
    # install, so composite indexes would only appear on a later registry
    # reload — after the addon's tests. Create them right after install.
    env["native.print.job"]._register_hook()
