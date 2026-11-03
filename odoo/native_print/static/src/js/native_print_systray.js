/** @odoo-module **/
import { Component, onMounted, onWillDestroy, onWillStart, useState } from "@odoo/owl";
import { registry } from "@web/core/registry";
import { useService } from "@web/core/utils/hooks";
import { rpc } from "@web/core/network/rpc";
import { getSelectedPrinter, setSelectedPrinter } from "./native_print_client";

export class NativePrintSystray extends Component {
    static template = "native_print.SystrayItem";
    static props = {};

    setup() {
        this.action = useService("action");
        this.notification = useService("notification");
        this.state = useState({
            isOpen: false,
            bridgeActive: Boolean(window.__NATIVE_PRINT__),
            printersCount: 0,
            defaultPrinter: null,
            discoveredPrinters: [],
            selectedPrinter: null,
            syncingPrinters: false,
            activeJobs: 0,
            doneJobs: 0,
            failedJobs: 0,
        });

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
                "Open Odoo inside the Native Print desktop app to see this PC's printers.",
                { title: "Desktop App Required", type: "warning" },
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
                `${printers.length} printer(s) found on this PC.`,
                { title: "Printers Refreshed", type: "success" },
            );
        } catch (error) {
            if (showNotice) this.notification.add(String(error && error.message || error),
                { title: "Printer Refresh Failed", type: "danger" });
        } finally {
            this.state.syncingPrinters = false;
        }
    }

    async refreshStats() {
        const bridge = window.__NATIVE_PRINT__ || null;
        this.state.bridgeActive = Boolean(bridge);
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
                "Native Control Center is available when running inside the Odoo Native Print desktop app (Ctrl+Shift+P).",
                { title: "Browser Fallback Mode", type: "info" },
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
            `${printer.name} will be used for printing from this PC.`,
            { title: "Printer Selected", type: "success" },
        );
    }

    async runQuickTestPrint() {
        const bridge = window.__NATIVE_PRINT__ || null;
        if (!bridge || typeof bridge.runTestPrint !== "function") {
            this.notification.add(
                "Open Odoo inside the Native Print app to send hardware calibration prints.",
                { title: "Native App Required", type: "warning" },
            );
            return;
        }
        const physicalPrinters = this.state.discoveredPrinters.filter(
            (printer) => printer.kind !== "virtual",
        );
        const chosen = physicalPrinters.find(
            (printer) => printer.name === getSelectedPrinter(),
        );
        const defaultPhysical = physicalPrinters.find((printer) => (
            printer.is_default || printer.isDefault
        ));
        const selected = chosen || defaultPhysical || physicalPrinters[0];
        if (!selected || !selected.name) {
            this.notification.add(
                "No physical printer was discovered. Connect or install a printer, then refresh the printer list.",
                { title: "No Printer Available", type: "warning" },
            );
            return;
        }

        let testKind = "pdf-a4";
        let paper = "A4";
        try {
            const capabilities = typeof bridge.printerCapabilities === "function"
                ? await bridge.printerCapabilities(selected.name)
                : { forms: [] };
            const forms = Array.isArray(capabilities && capabilities.forms)
                ? capabilities.forms
                : [];
            const roll = forms.find((form) => form && form.roll);
            const label = forms.find((form) => {
                const name = String(form && form.name || "");
                const size = form && form.size || {};
                return /label|100x150/i.test(name) ||
                    (size.width_mm >= 90 && size.height_mm >= 90 && size.height_mm <= 250);
            });
            if (roll) {
                const width = Number(roll.size && roll.size.width_mm) || 80;
                testKind = width <= 60 ? "thermal-58mm" : "thermal-80mm";
                paper = roll.name || (width <= 60 ? "58mm" : "80mm");
            } else if (label) {
                testKind = "zpl-label";
                paper = label.name || "100x150mm";
            } else if (forms.length) {
                const standard = forms.find((form) => /^a4$/i.test(form.name || "")) ||
                    forms.find((form) => /^letter$/i.test(form.name || "")) ||
                    forms[0];
                paper = standard.name || "A4";
            }

            const jobId = await bridge.runTestPrint(selected.name, testKind, paper);
            this.notification.add(
                `Queued ${paper} calibration test (${jobId}) on ${selected.name}`,
                { title: "Calibration Queued", type: "success" },
            );
            await this.refreshStats();
        } catch (err) {
            this.notification.add(String(err), {
                title: "Calibration Failed",
                type: "danger",
            });
        }
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
