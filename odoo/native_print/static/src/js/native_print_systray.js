/** @odoo-module **/
import { Component, onMounted, onWillDestroy, onWillStart, useState } from "@odoo/owl";
import { registry } from "@web/core/registry";
import { useService } from "@web/core/utils/hooks";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import { getSelectedPrinter, setSelectedPrinter } from "./native_print_client";

// One-line source description for a discovered printer (owner
// brief, 2026-11-05): where it is attached and what it is, from the
// bridge's live discovery fields (kind/port/protocol/class) — e.g.
// "USB · Thermal · ESC/POS", "Network · 192.168.1.50 · Office · GDI".
// Windows cannot tell Wi-Fi from wired Ethernet for a queue, so a
// network printer is honestly labelled Network with its address.
function printerMeta(p) {
    if (!p) return "";
    const kind = String(p.kind || "");
    const port = String(p.port || "");
    const classes = {
        thermal: _t("Thermal"), label: _t("Label"),
        office: _t("Office"), virtual: _t("Virtual"),
    };
    const parts = [];
    if (kind === "virtual" || /^nul:/i.test(port)) {
        parts.push(_t("Virtual"));
    } else if (/^USB/i.test(port)) {
        parts.push("USB");
    } else if (/^\\\\/.test(port)) {
        parts.push(_t("Shared"));
    } else if (kind === "network" || /^(IP_|WSD|TCP)/i.test(port) ||
               /^\d{1,3}(\.\d{1,3}){3}/.test(port)) {
        parts.push(_t("Network"));
        const addr = port.replace(/^IP_/i, "");
        if (/^\d{1,3}(\.\d{1,3}){3}/.test(addr)) {
            parts.push(addr);
        }
    } else if (/^(LPT|COM)\d/i.test(port)) {
        parts.push(port);
    } else {
        parts.push(_t("Local"));
    }
    const cls = classes[String(p.printer_class || "")] || "";
    if (cls && parts[0] !== cls) parts.push(cls);
    const proto = String(p.protocol || "").toUpperCase();
    if (proto) parts.push(proto);
    return parts.join(" · ");
}

// The oldest desktop-app (bridge) version this panel is built for.
// Older apps still print, but the panel asks for an update so field
// issues fixed in newer apps do not linger unnoticed.
const MIN_BRIDGE_VERSION = "1.4.11";
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
            this.state.discoveredPrinters = printers.map(
                (p) => ({ ...p, meta: printerMeta(p) }));
            this.state.selectedPrinter = getSelectedPrinter();
            this.maybeSyncDeviceCaps(printers);
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

    // Report the active printer's driver capabilities (paper forms
    // + trays) to the device registry, so the printer is visible in
    // Odoo with its full sizes and trays (owner brief, 2026-11-05).
    // Throttled per printer: re-sent only when the capability set
    // changes or after 6 hours; failures stay in the console — the
    // panel never blocks on bookkeeping.
    async maybeSyncDeviceCaps(printers) {
        const bridge = window.__NATIVE_PRINT__ || null;
        if (!bridge || typeof bridge.printerCapabilities !== "function") return;
        const wanted = getSelectedPrinter() || this.state.defaultPrinter;
        const target = printers.find((p) => p.name === wanted) || printers[0];
        if (!target) return;
        let store = {};
        try {
            store = JSON.parse(localStorage.getItem("npCapsSyncV1") || "{}");
        } catch (_e) { store = {}; }
        const prev = store[target.name] || {};
        try {
            const caps = await bridge.printerCapabilities(target.name);
            if (!caps) return;
            const forms = (caps.forms || []).map((f) =>
                `${f.name} — ${Math.round(f.width_mm)}×${Math.round(f.height_mm)}mm` +
                (f.roll ? " (" + _t("Roll") + ")" : ""));
            const trays = Array.isArray(caps.bins) ? caps.bins.slice() : [];
            const hash = JSON.stringify(
                [target.protocol, target.printer_class, forms, trays]);
            const now = Date.now();
            if (prev.hash === hash && now - (prev.at || 0) < 6 * 3600 * 1000) return;
            const res = await rpc("/native_print/sync_device", {
                name: target.name,
                protocol: target.protocol || null,
                printer_class: target.printer_class || null,
                forms, trays,
            }, { silent: true });
            if (res && res.ok) {
                store[target.name] = { at: now, hash };
                localStorage.setItem("npCapsSyncV1", JSON.stringify(store));
            }
        } catch (error) {
            console.error(error);
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
