/** @odoo-module **/
import { Component, onMounted, onWillUnmount, useState } from "@odoo/owl";
import { registry } from "@web/core/registry";
import { useService } from "@web/core/utils/hooks";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import { detectPrinterType, formatFormsSummary, getSelectedPosPrinter, getSelectedPrinter, JOB_UPDATED_EVENT, setSelectedPosPrinter, setSelectedPrinter } from "./native_print_client";

// One-line source description for a discovered printer:
// where it is attached and what it is, from the
// bridge's live discovery fields (kind/port/protocol/class) — e.g.
// "USB · Thermal · ESC/POS", "Network · 192.168.1.50 · Office · GDI".
// Windows cannot tell Wi-Fi from wired Ethernet for a queue, so a
// network printer is honestly labelled Network with its address.
function printerMeta(p) {
    if (!p) return "";
    const kind = String(p.kind || "");
    const port = String(p.port || p.address || "");
    const transport = String(p.transport || "").toLowerCase();
    const classes = {
        thermal: _t("Thermal"), label: _t("Label"),
        office: _t("Office"), virtual: _t("Virtual"),
    };
    const parts = [];
    if (kind === "virtual" || /^nul:/i.test(port)) {
        parts.push(_t("Virtual"));
    } else if (transport === "bluetooth_spp") {
        parts.push("Bluetooth");
    } else if (transport === "usb_otg" || /^USB/i.test(port)) {
        parts.push("USB");
    } else if (/^\\\\/.test(port)) {
        parts.push(_t("Shared"));
    } else if (transport === "tcp9100" || transport === "ipp" ||
               kind === "network" || /^(IP_|WSD|TCP)/i.test(port) ||
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
    const cls = classes[String(p.printer_class || p.printerClass || "")] || "";
    if (cls && parts[0] !== cls) parts.push(cls);
    const rawProto = String(p.protocol || "").toLowerCase();
    const proto = rawProto && rawProto !== "auto" ? rawProto.toUpperCase() : "";
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
            selectedPosPrinter: null,
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
            browserNote: _t("Browser mode: reports download as files. Open Odoo in the Yaseir Print app to print directly."),
            noPrinters: _t("No printers found on this PC. Connect a printer and refresh."),
            inUse: _t("In use"),
            usePrinter: _t("Use for Reports"),
            inUsePos: _t("POS"),
            usePosPrinter: _t("Use for POS"),
            discovering: _t("Discovering printers…"),
            refreshPrinters: _t("Refresh desktop printers"),
            activeJobs: _t("Active jobs"),
            doneJobs: _t("Done jobs"),
            failedJobs: _t("Failed jobs"),
            jobsLog: _t("Print Jobs Log"),
            controlCenter: _t("Open Control Center"),
            updateApp: _t("A newer desktop app is available. Update Yaseir Print for the latest fixes."),
        };

        this._onOutsideClick = () => {
            if (this.state.isOpen) {
                this.state.isOpen = false;
            }
        };
        this._onJobUpdated = () => {
            if (!this._unmounted) {
                this.refreshStats().catch(() => {});
            }
        };
        this._onVisibilityChange = () => {
            if (!this._unmounted && !document.hidden) {
                this.refreshStats().catch(() => {});
            }
        };

        this._unmounted = false;
        this._pendingRefresh = false;
        this._pendingForceSync = false;

        onMounted(() => {
            this.refreshStats().catch(() => {});
            window.addEventListener("click", this._onOutsideClick);
            window.addEventListener(JOB_UPDATED_EVENT, this._onJobUpdated);
            if (typeof document !== "undefined" && typeof document.addEventListener === "function") {
                document.addEventListener("visibilitychange", this._onVisibilityChange);
            }
            // Keep workstation inventory and online/offline state current
            // without making every Odoo screen action trigger a full scan.
            this._printerRefreshTimer = window.setInterval(() => {
                if (!document.hidden) {
                    this.refreshStats().catch(() => {});
                }
            }, 60_000);
        });

        onWillUnmount(() => {
            this._unmounted = true;
            this._pendingRefresh = false;
            this._pendingForceSync = false;
            window.removeEventListener("click", this._onOutsideClick);
            window.removeEventListener(JOB_UPDATED_EVENT, this._onJobUpdated);
            if (typeof document !== "undefined" && typeof document.removeEventListener === "function") {
                document.removeEventListener("visibilitychange", this._onVisibilityChange);
            }
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
        if (this.state.syncingPrinters) {
            if (showNotice) {
                this._pendingForceSync = true;
            }
            return;
        }
        this.state.syncingPrinters = true;
        try {
            // Live discovery only: printers belong to this PC and are
            // never stored on the server.
            const syncSeq = (this._printerSyncSeq = (this._printerSyncSeq || 0) + 1);
            const local = await bridge.appOverview(showNotice ? { force: true } : undefined);
            if (this._unmounted || syncSeq !== this._printerSyncSeq) return;
            const printers = Array.isArray(local && local.printers) ? local.printers : [];
            this._bridgeSyncFailed = false;
            this.state.bridgeActive = true;
            this.state.printersCount = printers.length;
            const defaultEntry = printers.find((p) => p && (p.is_default || p.isDefault));
            this.state.defaultPrinter = (local && local.defaultPrinter) || (defaultEntry && defaultEntry.name) || null;
            this.state.discoveredPrinters = printers.map((p) => {
                const inferred = detectPrinterType(p);
                const printerClass = p.printer_class || p.printerClass || inferred.printerClass || "";
                const proto = p.protocol && String(p.protocol).toLowerCase() !== "auto" ? p.protocol : inferred.mode;
                return { ...p, printer_class: printerClass, meta: printerMeta({ ...p, printer_class: printerClass, protocol: proto }) };
            });
            this.state.selectedPrinter = getSelectedPrinter();
            this.state.selectedPosPrinter = getSelectedPosPrinter();
            this.maybeSyncDeviceCaps(printers, showNotice).catch(() => {});
            if (showNotice) this.notification.add(
                _t("%s printer(s) found on this PC.", printers.length),
                { title: _t("Printers Refreshed"), type: "success" },
            );
        } catch (error) {
            // Keep raw exception text in console; show localized notice to users.
            console.error("Yaseir Print: printer refresh failed.", error);
            if (!this._unmounted) {
                this._bridgeSyncFailed = true;
                this.state.bridgeActive = false;
                this.state.printersCount = 0;
                this.state.defaultPrinter = null;
                this.state.discoveredPrinters = [];
            }
            if (showNotice && !this._unmounted) this.notification.add(
                _t("Could not read this PC's printers. Check the desktop app and try again."),
                { title: _t("Printer Refresh Failed"), type: "danger" });
        } finally {
            if (!this._unmounted) {
                this.state.syncingPrinters = false;
                if (this._pendingForceSync) {
                    this._pendingForceSync = false;
                    this.syncDiscoveredPrinters(true).catch(() => {});
                }
            }
        }
    }

    // Report the active printer's driver capabilities (paper forms
    // and trays) to the device registry for read-only display in Odoo.
    // Throttled per printer: re-sent only when the capability set
    // changes or after 6 hours; failures stay in the console — the
    // panel never blocks on bookkeeping.
    async maybeSyncDeviceCaps(printers, force = false) {
        const bridge = window.__NATIVE_PRINT__ || null;
        if (!bridge || typeof bridge.printerCapabilities !== "function") return;
        const wanted = getSelectedPrinter() || this.state.defaultPrinter;
        const target = printers.find((p) => p.name === wanted) || printers[0];
        if (!target) return;
        let store = {};
        try {
            const parsed = JSON.parse(localStorage.getItem("npCapsSyncV1") || "{}");
            store = parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
        } catch (_e) { store = {}; }
        const companyId = (this.env && this.env.services && this.env.services.company && this.env.services.company.currentCompany && this.env.services.company.currentCompany.id) || 0;
        const cacheKey = `${companyId}:${target.name}`;
        const prev = store[cacheKey] || {};
        const now = Date.now();
        const inferred = detectPrinterType(target);
        const printerClass = target.printer_class || target.printerClass || inferred.printerClass || null;
        const protocol = target.protocol && String(target.protocol).toLowerCase() !== "auto" ? target.protocol : inferred.mode;
        const metaKey = `${protocol || ""}|${printerClass || ""}`;
        // Skip querying Windows driver capabilities on 60s background polls
        // when this printer's capabilities were already synced within 6 hours
        // and its protocol/class did not change. Manual refresh passes force=true.
        if (!force && prev.at && now - prev.at < 6 * 3600 * 1000 && prev.meta === metaKey) {
            return;
        }
        try {
            const caps = await bridge.printerCapabilities(target.name);
            if (!caps || this._unmounted) return;
            const forms = formatFormsSummary(caps.forms, _t("Roll"));
            const trays = Array.isArray(caps.bins) ? caps.bins.slice() : [];
            const hash = JSON.stringify(
                [protocol, printerClass, forms, trays]);
            if (!force && prev.hash === hash && now - (prev.at || 0) < 6 * 3600 * 1000) return;
            const res = await rpc("/native_print/sync_device", {
                name: target.name,
                protocol: protocol || null,
                printer_class: printerClass,
                forms, trays,
            }, { silent: true });
            if (res && res.ok) {
                store[cacheKey] = { at: now, hash, meta: metaKey };
                try {
                    localStorage.setItem("npCapsSyncV1", JSON.stringify(store));
                } catch (_e) { /* ignore storage quota/privacy errors */ }
            }
        } catch (error) {
            console.error("Yaseir Print: device capability sync failed.", error);
        }
    }

    async refreshStats() {
        if (this._unmounted) return;
        if (this._refreshingStats) {
            this._pendingRefresh = true;
            return;
        }
        this._refreshingStats = true;
        try {
            const bridge = window.__NATIVE_PRINT__ || null;
            this.state.bridgeActive = Boolean(bridge) && !this._bridgeSyncFailed;
            this.state.appOutdated = bridgeIsOutdated(bridge && bridge.version);
            const overviewTask = rpc("/native_print/overview", {}, { silent: true })
                .then((srv) => {
                    if (!this._unmounted && srv && srv.ok) {
                        this.state.activeJobs = srv.active_jobs || 0;
                        this.state.doneJobs = srv.done_jobs || 0;
                        this.state.failedJobs = srv.failed_jobs || 0;
                    }
                })
                .catch(() => {
                    // Ignore background refresh error.
                });
            if (bridge) {
                await Promise.all([this.syncDiscoveredPrinters(false), overviewTask]);
            } else {
                await overviewTask;
            }
        } finally {
            this._refreshingStats = false;
            if (this._pendingRefresh && !this._unmounted) {
                this._pendingRefresh = false;
                this.refreshStats().catch(() => {});
            }
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

    onKeyDown(ev) {
        if (ev && ev.key === "Escape" && this.state.isOpen) {
            if (typeof ev.stopPropagation === "function") ev.stopPropagation();
            this.closeMenu();
        }
    }

    async openControlCenter() {
        this.closeMenu();
        const bridge = window.__NATIVE_PRINT__ || null;
        if (bridge && typeof bridge.openControlCenter === "function") {
            try {
                await bridge.openControlCenter();
            } catch (err) {
                console.error("Yaseir Print: opening Control Center failed.", err);
            }
        } else {
            this.notification.add(
                _t("Open Odoo in the Yaseir Print desktop app to use the Control Center (Ctrl+Shift+Y)."),
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

    selectPosPrinter(printer) {
        setSelectedPosPrinter(printer.name);
        this.state.selectedPosPrinter = printer.name;
        this.notification.add(
            _t("%s will be used for POS receipts on this PC.", printer.name),
            { title: _t("POS Printer Selected"), type: "success" },
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
