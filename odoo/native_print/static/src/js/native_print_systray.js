/** @odoo-module **/
import { Component, onMounted, onWillDestroy, onWillStart, useState } from "@odoo/owl";
import { registry } from "@web/core/registry";
import { useService } from "@web/core/utils/hooks";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import { getSelectedPrinter, setSelectedPrinter } from "./native_print_client";

// The oldest desktop-app (bridge) version this panel is built for.
// Older apps still print, but the panel asks for an update so field
// issues fixed in newer apps do not linger unnoticed.
const MIN_BRIDGE_VERSION = "1.4.7";
function bridgeIsOutdated(version) {
    if (!version || typeof version !== "string") return false;
    const have = version.split(".").map((x) => parseInt(x, 10) || 0);
    const want = MIN_BRIDGE_VERSION.split(".").map((x) => parseInt(x, 10) || 0);
    for (let i = 0; i < Math.max(have.length, want.length); i++) {
        const h = have[i] || 0;
        const w = want[i] || 0;
        if (h < w) return true;
        if (h > w) return false;
    }
    return false;
}

export class NativePrintSystray extends Component {
    static template = "native_print.SystrayItem";
    static props = {};

    setup() {
        this.action = useService("action");
        this.notification = useService("notification");
        this.state = useState({
            isOpen: false,
            bridgeActive: Boolean(window.__NATIVE_PRINT__),
            appOutdated: false,
            printersCount: 0,
            defaultPrinter: null,
            discoveredPrinters: [],
            selectedPrinter: null,
            syncingPrinters: false,
            activeJobs: 0,
            doneJobs: 0,
            failedJobs: 0,
        });
        // Every visible panel string comes from here so the whole
        // dropdown speaks one language: full, unambiguous phrases
        // (one-word labels collide with other modules' catalog terms).
        this.labels = {
            title: _t("Yaseir Print Status & Control"),
            appActive: _t("App Active"),
            browserMode: _t("Browser Mode"),
            localPrinters: _t("Local Printers"),
            defaultPrinter: _t("Default Printer"),
            browserNote: _t("Standard browser session: reports download normally unless opened inside the Yaseir Print app."),
            inUse: _t("In use"),
            usePrinter: _t("Use this printer"),
            discovering: _t("Discovering printers…"),
            refreshPrinters: _t("Refresh desktop printers"),
            activeJobs: _t("Active jobs"),
            doneJobs: _t("Done jobs"),
            failedJobs: _t("Failed jobs"),
            jobsLog: _t("Print Jobs Log"),
            controlCenter: _t("Open Native Control Center"),
            updateApp: _t("A newer desktop app is available. Update Yaseir Print for the latest fixes."),
        };

        this._onOutsideClick = () => {
            if (this.state.isOpen) {
                this.state.isOpen = false;
            }
        };

        onWillStart(async () => {
            await this.refreshStats();
        });

        onMounted(() => {
            window.addEventListener("click", this._onOutsideClick);
            // Keep workstation inventory and online/offline state current
            // without making every Odoo screen action trigger a full scan.
            this._printerRefreshTimer = window.setInterval(() => {
                if (document.visibilityState === "visible") {
                    this.refreshStats();
                }
            }, 60_000);
        });

        onWillDestroy(() => {
            window.removeEventListener("click", this._onOutsideClick);
            if (this._printerRefreshTimer) {
                window.clearInterval(this._printerRefreshTimer);
                this._printerRefreshTimer = null;
            }
        });
    }

    async syncDiscoveredPrinters(showNotice = false) {
        const bridge = window.__NATIVE_PRINT__ || null;
        if (!bridge || typeof bridge.appOverview !== "function") {
            if (showNotice) this.notification.add(
                _t("Open Odoo inside the Yaseir Print desktop app to see this PC's printers."),
                { title: _t("Desktop App Required"), type: "warning" },
            );
            return;
        }
        if (this.state.syncingPrinters) return;
        this.state.syncingPrinters = true;
        try {
            // Live discovery only: printers belong to this PC and are
            // never stored on the server.
            const local = await bridge.appOverview();
            const printers = Array.isArray(local && local.printers) ? local.printers : [];
            this.state.printersCount = printers.length;
            this.state.defaultPrinter = local && local.defaultPrinter || null;
            this.state.discoveredPrinters = printers;
            this.state.selectedPrinter = getSelectedPrinter();
            if (showNotice) this.notification.add(
                _t("%s printer(s) found on this PC.", printers.length),
                { title: _t("Printers Refreshed"), type: "success" },
            );
        } catch (error) {
            // Raw exception text stays in the console: users get a
            // localized explanation, not an English stack message
            // (audit JS-L3).
            console.error(error);
            if (showNotice) this.notification.add(
                _t("Could not read this PC's printers. Check the desktop app and try again."),
                { title: _t("Printer Refresh Failed"), type: "danger" });
        } finally {
            this.state.syncingPrinters = false;
        }
    }

    async refreshStats() {
        const bridge = window.__NATIVE_PRINT__ || null;
        this.state.bridgeActive = Boolean(bridge);
        this.state.appOutdated = bridgeIsOutdated(bridge && bridge.version);
        if (bridge) {
            await this.syncDiscoveredPrinters(false);
        }
        try {
            const srv = await rpc("/native_print/overview", {}, { silent: true });
            if (srv && srv.ok) {
                this.state.activeJobs = srv.active_jobs || 0;
                this.state.doneJobs = srv.done_jobs || 0;
                this.state.failedJobs = srv.failed_jobs || 0;
            }
        } catch (_err) {
            // Ignore background refresh error.
        }
    }

    async toggleMenu() {
        this.state.isOpen = !this.state.isOpen;
        if (this.state.isOpen) {
            await this.refreshStats();
        }
    }

    closeMenu() {
        this.state.isOpen = false;
    }

    async openControlCenter() {
        this.closeMenu();
        const bridge = window.__NATIVE_PRINT__ || null;
        if (bridge && typeof bridge.openControlCenter === "function") {
            await bridge.openControlCenter();
        } else {
            this.notification.add(
                _t("The Control Center is available when running inside the Yaseir Print desktop app (Ctrl+Shift+Y)."),
                { title: _t("Browser Fallback Mode"), type: "info" },
            );
        }
    }

    async refreshPrinters() {
        await this.syncDiscoveredPrinters(true);
    }

    selectPrinter(printer) {
        // Session-local choice on this PC only (browser storage);
        // nothing is saved on the server.
        setSelectedPrinter(printer.name);
        this.state.selectedPrinter = printer.name;
        this.notification.add(
            _t("%s will be used for printing from this PC.", printer.name),
            { title: _t("Printer Selected"), type: "success" },
        );
    }

    openJobs() {
        this.closeMenu();
        this.action.doAction("native_print.action_jobs");
    }

}

registry.category("systray").add(
    "native_print.SystrayItem",
    {
        Component: NativePrintSystray,
    },
    { sequence: 35 },
);
