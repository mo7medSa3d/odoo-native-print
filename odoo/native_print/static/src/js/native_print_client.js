/** @odoo-module **/
import { _t } from "@web/core/l10n/translation";

// ---------------------------------------------------------------------
// Live printer model: printers are never stored on the server. The
// desktop app on THIS PC is the only printer authority; these helpers
// turn its live inventory into a print target at click time.
// ---------------------------------------------------------------------

const SELECTION_KEY = "native_print.selected_printer";

export function getSelectedPrinter() {
    try {
        return window.localStorage.getItem(SELECTION_KEY) || null;
    } catch (_error) {
        return null;
    }
}

export function setSelectedPrinter(name) {
    try {
        if (name) {
            window.localStorage.setItem(SELECTION_KEY, name);
        } else {
            window.localStorage.removeItem(SELECTION_KEY);
        }
    } catch (_error) {
        // Selection is a convenience; printing must not depend on it.
    }
}

/** Conservative protocol inference from the installed driver identity. */
export function detectPrinterType(printer) {
    // The desktop bridge ships the app's own protocol detection on
    // every printer (Rust print-core detect_protocol, present
    // since app 1.4.11): when it is there it is the ONLY detection
    // — a second name heuristic in the page disagreed with it in
    // the field and valid POS printers were routed to the wrong
    // mode and refused. The heuristic below remains only as a
    // fallback for bridges too old to ship the fields.
    if (printer && printer.protocol) {
        return {
            mode: printer.protocol,
            printerClass:
                printer.printer_class || printer.printerClass || "office",
        };
    }
    const signature =
        `${(printer && printer.name) || ""} ${(printer && printer.driver) || ""}`
            .toLowerCase();
    const kind = (printer && printer.kind) || "local";
    if (kind === "virtual") {
        return { mode: "gdi", printerClass: "virtual" };
    }
    // Short protocol tokens match on word boundaries only (audit
    // L10): a bare substring search classifies any name containing
    // "epl" (e.g. "Repl…") or "tsc" as a raw label printer, and an
    // office printer misclassified that way gets raw jobs instead
    // of GDI.
    const hasWord = (tok) =>
        new RegExp(`(^|[^a-z0-9])${tok}([^a-z0-9]|$)`).test(signature);
    if (["zdesigner", "zebra"].some((t) => signature.includes(t)) || hasWord("zpl")) {
        return { mode: "zpl", printerClass: "label" };
    }
    if (hasWord("tsc") || hasWord("tspl")) {
        return { mode: "tspl", printerClass: "label" };
    }
    if (hasWord("epl")) {
        return { mode: "epl", printerClass: "label" };
    }
    // Mirrors the Rust THERMAL_TOKENS table (print-core
    // detect_protocol) — keep the two in sync. The generic
    // identities matter as much as the brands: clone POS drivers
    // very often identify only as "POS Printer" / "POS80" /
    // "Thermal Printer".
    const thermalTokens = [
        "epson tm-", "epson tm ", "xprinter", "rongta", "bixolon",
        "star tsp", "citizen ct-", "citizen", "sewoo", "esc/pos", "escpos",
        "thermal receipt", "thermal printer", "receipt printer",
        "receipt", "pos printer", "pos-58", "pos-80", "pos58", "pos80",
        "tm-t", "tm-m", "tm-u", "sunmi", "munbyn", "hprt", "cashino",
        "snbc", "gprinter", "sprt", "gainscha", "zjiang", "btp-",
        "srp-", "posiflex", "mc-print", "xp-80", "xp-58",
        "58mm", "80mm", "usb thermal", "mini printer",
        "热敏", "票据", "小票",
    ];
    if (thermalTokens.some((t) => signature.includes(t))) {
        return { mode: "escpos", printerClass: "thermal" };
    }
    if (
        kind === "network" &&
        (signature.includes("ipp") || signature.includes("airprint"))
    ) {
        return { mode: "gdi", printerClass: "office" };
    }
    return { mode: "gdi", printerClass: "office" };
}

function isIpLiteral(value) {
    const text = String(value || "").trim();
    // IPv4: dotted quads with real octet ranges — "999.1.1.1" is a
    // hostname-shaped string, not an IP (audit L9).
    const v4 = text.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
    if (v4) return v4.slice(1).every((o) => Number(o) <= 255);
    if (!text.includes(":")) return false;
    // IPv6: hex groups with at most one "::" compression and no
    // stray leading/trailing single colon.
    return /^[0-9a-fA-F:]+$/.test(text)
        && (text.match(/::/g) || []).length <= 1
        && (!text.startsWith(":") || text.startsWith("::"))
        && (!text.endsWith(":") || text.endsWith("::"));
}

/** Raw printers reachable at an IP print directly; everything else
 * goes through its installed Windows queue by name. */
export function printerPatternFor(printer, mode) {
    const rawModes = ["escpos", "zpl", "tspl", "epl"];
    const port = (printer && printer.port) || "";
    if (rawModes.includes(mode) && isIpLiteral(port)) {
        // IPv6 literals need brackets in a URI authority, or the
        // port suffix is ambiguous (audit L9).
        const host = port.includes(":") ? `[${port}]` : port;
        return `tcp://${host}:9100`;
    }
    return printer.name;
}

/** Receipt roll width from the driver-reported forms, if any. */
export function rollWidthMm(capabilities) {
    const forms = (capabilities && capabilities.forms) || [];
    for (const form of forms) {
        if (form && form.roll && form.size && form.size.width_mm) {
            return form.size.width_mm;
        }
    }
    return null;
}

/**
 * Resolve the printer to use RIGHT NOW on this PC: the user's
 * selection when it is still connected, else a preferred-class
 * printer when asked (POS receipts prefer thermal/label), else the
 * OS default, else the first printer. Returns null when this PC has
 * no printer at the moment — callers then keep Odoo's normal flow.
 */
export async function resolveBridgeTarget(bridge, options = {}) {
    if (!bridge || typeof bridge.listPrinters !== "function") return null;
    const printers = await bridge.listPrinters();
    if (!Array.isArray(printers) || !printers.length) return null;
    const prefer = options.preferClasses || [];
    const classOf = (p) => detectPrinterType(p).printerClass;
    const selected = printers.find((p) => p.name === getSelectedPrinter()) || null;
    let printer = null;
    // A reprint from the job log names the printer the job originally
    // ran on: when that printer is connected to this PC right now it
    // wins over every other preference; when it is not, resolution
    // falls through to the normal order below.
    if (options.preferPrinterName) {
        printer = printers.find((p) => p.name === options.preferPrinterName) || null;
    }
    // The owner's model (2026-11-04): the printer picked in the
    // panel is THE printer — for reports and receipts alike, real
    // or virtual. An explicit selection therefore wins over the
    // class preference; the preference only breaks the tie when
    // nothing is selected.
    if (!printer && selected) {
        printer = selected;
    }
    if (!printer && prefer.length) {
        printer = printers.find((p) => prefer.includes(classOf(p))) || null;
    }
    if (!printer) {
        printer =
            selected ||
            printers.find((p) => p.is_default || p.isDefault) ||
            printers[0];
    }
    let capabilities = null;
    if (typeof bridge.printerCapabilities === "function") {
        try {
            capabilities = await bridge.printerCapabilities(printer.name);
        } catch (_error) {
            capabilities = null;
        }
    }
    const detection = detectPrinterType(printer);
    return {
        printer,
        capabilities,
        rule: {
            printer_pattern: printerPatternFor(printer, detection.mode),
            mode: detection.mode,
            paper: null,
            copies: 1,
            tray: null,
            duplex: Boolean(capabilities && capabilities.duplex),
            color: !capabilities || capabilities.color !== false,
            scale_policy: "custom_allowed",
            media: { roll_width_mm: rollWidthMm(capabilities) },
        },
    };
}

// ---------------------------------------------------------------------
// User notifications: one helper for every Native Print message, so
// report prints and POS receipts behave identically.
// - Success/info stay quiet: short-lived and auto-dismissed.
// - Warnings and failures are sticky: a print that did not happen
//   (or silently turned into a download) must stay on screen until
//   the user has actually read it.
// - The same message is never stacked twice: an identical repeat
//   within a few seconds (double-fire, or a watcher and a fallback
//   reporting the same failure) is shown only once.
// Notification problems must never block printing, so every failure
// inside this helper is swallowed.
// ---------------------------------------------------------------------

const NOTIFICATION_DEDUPE_MS = 4000;
const shownNotifications = new Map();

export function notifyUser(env, message, type = "info", title = _t("Yaseir Print")) {
    try {
        const notification =
            env && env.services && env.services.notification;
        if (!notification || typeof notification.add !== "function") {
            return;
        }
        const key = `${type}|${title}|${message}`;
        const now = Date.now();
        if (now - (shownNotifications.get(key) || 0) < NOTIFICATION_DEDUPE_MS) {
            return;
        }
        shownNotifications.set(key, now);
        if (shownNotifications.size > 50) {
            for (const [k, ts] of shownNotifications) {
                if (now - ts >= NOTIFICATION_DEDUPE_MS) {
                    shownNotifications.delete(k);
                }
            }
        }
        notification.add(message, {
            title,
            type,
            sticky: type === "danger" || type === "warning",
        });
    } catch (_err) {
        // Informational only; never block printing.
    }
}
