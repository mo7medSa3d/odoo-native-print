/** @odoo-module **/
import { Component, onMounted, onWillDestroy, onWillStart, useState } from "@odoo/owl";
import { registry } from "@web/core/registry";
import { useService } from "@web/core/utils/hooks";
import { rpc } from "@web/core/network/rpc";

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
            profilesCount: 0,
            rulesCount: 0,
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
        });

        onWillDestroy(() => {
            window.removeEventListener("click", this._onOutsideClick);
        });
    }

    async refreshStats() {
        const bridge = window.__NATIVE_PRINT__ || null;
        this.state.bridgeActive = Boolean(bridge);
        if (bridge && typeof bridge.appOverview === "function") {
            try {
                const local = await bridge.appOverview();
                if (local) {
                    this.state.printersCount = (local.printers || []).length;
                    this.state.defaultPrinter = local.defaultPrinter || null;
                }
            } catch (_err) {
                // Keep server stats even if local overview call fails.
            }
        }
        try {
            const srv = await rpc("/native_print/overview", {}, { silent: true });
            if (srv && srv.ok) {
                this.state.profilesCount = srv.profiles_count || 0;
                this.state.rulesCount = srv.rules_count || 0;
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

    async runQuickTestPrint() {
        const bridge = window.__NATIVE_PRINT__ || null;
        if (!bridge || typeof bridge.runTestPrint !== "function") {
            this.notification.add(
                "Open Odoo inside the Native Print app to send hardware calibration prints.",
                { title: "Native App Required", type: "warning" },
            );
            return;
        }
        const target = this.state.defaultPrinter || "Microsoft Print to PDF";
        try {
            const jobId = await bridge.runTestPrint(target, "pdf-a4", "A4");
            this.notification.add(
                `Queued A4 Calibration Test (${jobId}) on ${target}`,
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

    openProfiles() {
        this.closeMenu();
        this.action.doAction("native_print.action_profiles");
    }

    openRules() {
        this.closeMenu();
        this.action.doAction("native_print.action_rules");
    }
}

registry.category("systray").add(
    "native_print.SystrayItem",
    {
        Component: NativePrintSystray,
    },
    { sequence: 35 },
);
