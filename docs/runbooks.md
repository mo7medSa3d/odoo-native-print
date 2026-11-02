# Runbooks

## Install (per machine, enterprise)
1. Deploy the MSI silently: `msiexec /i OdooNativePrint.msi /qn ODOO_ORIGIN=https://odoo.company.local:8069`
   (the installer writes config.json with odoo_origin; or set the GPO policy).
2. WebView2: the bundle uses the offline installer mode so no internet is
   needed at install time. Trade-off: with a fixed/offline runtime, the IT
   team owns WebView2 patching; with Evergreen, Microsoft patches it.
   Choose per site policy and record the choice.
3. Install the Odoo addon (see docs/odoo-setup.md) and create at least one
   profile + rule, or nothing will route to native printing (by design).
4. First run: sign in to Odoo inside the app, print the calibration page
   for each printer (app includes the generator), file the hardware kit.

## Upgrade
1. Push the new MSI over the old (same product family, upgrade in place).
   The print queue (per-user data dir) and config survive upgrades.
2. Upgrade the Odoo addon by updating the module in Apps.
3. Re-run one calibration print per printer class after a major upgrade.

## Rollback
1. Redeploy the previous MSI (downgrade is supported by the installer
   family; if a site blocks downgrades, uninstall then install previous).
2. The queue database is backward compatible within v1 (state names and
   idempotency keys are stable); jobs queued at rollback time are recovered
   on next start per ADR-0004.
3. Roll the addon back by restoring the previous module version.

## Troubleshooting
- App shows a configuration error: config.json missing/invalid or the
  origin is not HTTPS or is still a placeholder. Fix the origin, relaunch.
- Report downloads instead of printing: the user is in a normal browser
  (expected), OR no rule matches that report (check Rules), OR the addon
  is not installed/upgraded.
- Job failed with PAPER_MISMATCH: the paper ladder found no acceptable
  form. Do not "fix" it by loosening to a wrong size silently: add the
  right form, enable custom sizes on the profile, or change the rule's
  scale policy deliberately.
- Job in Dead letter: open the job log, read the error code, fix the cause
  (usually printer offline/paper), then retry from dead letter (state
  transition dead_letter -> queued is supported).
- RECOVERY_AMBIGUOUS after a crash: the job may already be printed. Check
  the printer/output before reprinting; this prompt is deliberate
  (ADR-0004) so duplicates never happen silently.
- Diagnostics: export the one-click bundle (versions, printer list,
  last 200 job summaries, redacted) and attach it to the support ticket.

## Printer onboarding (field staff, ~5 minutes)
1. Install the printer driver; print a Windows test page.
2. Add a profile in Odoo (pattern must match the Windows printer name).
3. Print the calibration page; measure the 100 mm square; check edges.
4. If content is clipped/shifted, record the offset in the profile quirks
   (docs/hardware/validation-kit.md) instead of scaling the document.
5. File the hardware kit result sheet for that printer class.
