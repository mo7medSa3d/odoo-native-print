/** @odoo-module **/
import { _t } from "@web/core/l10n/translation";

// Live printer model: printers are never stored on the server. The
// desktop app on THIS PC is the only printer authority; these helpers
// turn its live inventory into a print target at click time.

const SELECTION_KEY = "native_print.selected_printer";
const POS_SELECTION_KEY = "native_print.selected_pos_printer";

// Odoo renders <html dir> from the language record. Some staging databases
// carry Arabic with direction=ltr, so the whole backend appears mirrored
// even though the strings are Arabic. Keep direction tied to the language:
// Arabic (and other RTL languages) -> rtl, English -> ltr.
const RTL_LANG = /^(ar|he|fa|ur|ps|sd|yi)([_-]|$)/i;

export function directionForLang(lang) {
    return RTL_LANG.test(String(lang || "")) ? "rtl" : "ltr";
}

export function syncDocumentDirection() {
    if (typeof document === "undefined" || !document.documentElement) {
        return null;
    }
    const root = document.documentElement;
    const globalOdoo = typeof odoo !== "undefined" ? odoo : null;
    const lang = String(
        root.lang ||
            (globalOdoo && (globalOdoo.lang || globalOdoo.session_info?.lang)) ||
            "",
    );
    if (!lang) {
        return null;
    }
    const dir = directionForLang(lang);
    if (root.dir !== dir) {
        root.dir = dir;
    }
    return dir;
}

syncDocumentDirection();
if (typeof document !== "undefined" && typeof document.addEventListener === "function") {
    document.addEventListener("DOMContentLoaded", syncDocumentDirection, { once: true });
}

// Shared terminal job states matching the desktop queue and server log.
// Watchers decide whether to keep observing after retryable outcomes (failed/dead_letter).
export const JOB_TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);

// Map Android bridge wire states onto the canonical Odoo/desktop job states.
const BRIDGE_STATE_ALIASES = {
    completed: "done",
    cancelled: "canceled",
    submitted: "printing",
    in_progress: "printing",
};

export function normalizeBridgeJobState(state) {
    if (typeof state !== "string") return null;
    const norm = state.trim().toLowerCase();
    if (!norm || norm === "not_found") return null;
    return BRIDGE_STATE_ALIASES[norm] || norm;
}

// Desktop enqueue returns the idempotencyKey string; Android returns a PrintJobRecord object with a UUID `id`.
export function resolveBridgeJobId(enqueueResult, fallbackKey) {
    if (enqueueResult && typeof enqueueResult === "object") {
        const id = enqueueResult.id || enqueueResult.job_id || enqueueResult.jobId;
        if (typeof id === "string" && id.trim()) {
            return id.trim();
        }
    }
    if (typeof enqueueResult === "string" && enqueueResult.trim()) {
        return enqueueResult.trim();
    }
    return fallbackKey;
}

/** Allow only short machine error codes in UI toasts so raw traces never leak. */
export function safeErrorCode(code, fallback = "error") {
    const text = typeof code === "string" ? code.trim() : "";
    return /^[A-Za-z0-9_.-]{1,64}$/.test(text) ? text : fallback;
}
export const JOB_UPDATED_EVENT = "native_print:job_updated";

export function notifyJobUpdated() {
    if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") {
        const ev = typeof CustomEvent === "function"
            ? new CustomEvent(JOB_UPDATED_EVENT)
            : (typeof Event === "function" ? new Event(JOB_UPDATED_EVENT) : { type: JOB_UPDATED_EVENT });
        window.dispatchEvent(ev);
    }
}

function readStorage(key) {
    try {
        return window.localStorage.getItem(key) || null;
    } catch (_error) {
        return null;
    }
}

function writeStorage(key, name) {
    try {
        if (name) {
            window.localStorage.setItem(key, name);
        } else {
            window.localStorage.removeItem(key);
        }
    } catch (_error) {
        // Selection is a convenience; printing must not depend on it.
    }
}

export function getSelectedPrinter() {
    return readStorage(SELECTION_KEY);
}

export function setSelectedPrinter(name) {
    writeStorage(SELECTION_KEY, name);
}

export function getSelectedPosPrinter() {
    return readStorage(POS_SELECTION_KEY);
}

export function setSelectedPosPrinter(name) {
    writeStorage(POS_SELECTION_KEY, name);
}

const WORD_RE_ZPL = /(^|[^a-z0-9])zpl([^a-z0-9]|$)/;
const WORD_RE_TSC = /(^|[^a-z0-9])tsc([^a-z0-9]|$)/;
const WORD_RE_TSPL = /(^|[^a-z0-9])tspl([^a-z0-9]|$)/;
const WORD_RE_EPL = /(^|[^a-z0-9])epl([^a-z0-9]|$)/;
const WORD_RE_CPCL = /(^|[^a-z0-9])cpcl([^a-z0-9]|$)/;

/** Conservative protocol inference from the installed driver identity. */
export function detectPrinterType(printer) {
    // Virtual writers are an unconditional safety override. Even a
    // stale or inconsistent bridge payload must never make a PDF/XPS
    // writer look like a RAW thermal or label target.
    const kind = (printer && printer.kind) || "local";
    if (kind === "virtual") {
        return { mode: "gdi", printerClass: "virtual" };
    }
    // Rust print-core detection is authoritative for non-virtual
    // printers; the name heuristic below is only a fallback for older
    // bridge payloads that do not include protocol/class fields.
    const rawProto =
        printer && typeof printer.protocol === "string"
            ? printer.protocol.trim().toLowerCase()
            : "";
    if (rawProto && rawProto !== "auto") {
        const mode = rawProto === "pdf" ? "gdi" : rawProto;
        const inferredClass =
            mode === "escpos"
                ? "thermal"
                : (mode === "zpl" || mode === "tspl" || mode === "epl" || mode === "cpcl")
                    ? "label"
                    : "office";
        return {
            mode,
            printerClass:
                printer.printer_class || printer.printerClass || inferredClass,
        };
    }
    const signature =
        `${(printer && printer.name) || ""} ${(printer && printer.driver) || ""}`
            .toLowerCase();
    // Match short protocol tokens on word boundaries so substrings like
    // "Repl..." do not misclassify office printers as raw label targets.
    if (signature.includes("zdesigner") || signature.includes("zebra") || WORD_RE_ZPL.test(signature)) {
        return { mode: "zpl", printerClass: "label" };
    }
    const tsplLabelModels = [
        "xp-235", "xp-350", "xp-365", "xp-370", "xp-420", "xp-450", "xp-460",
        "xp-470", "xp-490", "gp-1124", "gp-1324", "gp-1924", "gp-2120", "gp-3120",
    ];
    if (WORD_RE_TSC.test(signature) || WORD_RE_TSPL.test(signature) || tsplLabelModels.some((m) => signature.includes(m))) {
        return { mode: "tspl", printerClass: "label" };
    }
    if (WORD_RE_EPL.test(signature)) {
        return { mode: "epl", printerClass: "label" };
    }
    if (WORD_RE_CPCL.test(signature)) {
        return { mode: "cpcl", printerClass: "label" };
    }
    const officeExclusions = [
        "laserjet", "deskjet", "officejet", "inkjet", "ecosys", "bizhub",
        "ricoh", "xerox", "kyocera", "brother hl", "brother dcp", "brother mfc",
    ];
    if (officeExclusions.some((t) => signature.includes(t))) {
        return { mode: "gdi", printerClass: "office" };
    }
    // Mirrors the Rust THERMAL_TOKENS table (print-core
    // detect_protocol) — keep the two in sync.
    const thermalTokens = [
        "epson tm-", "epson tm ", "xprinter", "rongta", "bixolon",
        "star tsp", "star micronics", "tsp100", "tsp143", "tsp650",
        "citizen ct-", "citizen", "sewoo", "esc/pos", "escpos",
        "thermal receipt", "thermal printer", "receipt printer",
        "receipt", "pos printer", "pos-58", "pos-80", "pos-90",
        "pos58", "pos80", "tm-t", "tm-m", "tm-u", "sunmi", "munbyn",
        "hprt", "cashino", "snbc", "gprinter", "sprt", "gainscha",
        "zjiang", "btp-", "srp-", "posiflex", "mc-print",
        "xp-80", "xp-58", "xp-q", "xp-n", "xp-v", "xp-c", "xp-t",
        "rp80", "rp326", "tp80", "zkp80", "gp-u80", "gp-l80",
        "sam4s", "metapace", "hoin", "ocom", "goojprt", "zonerich",
        "58mm", "80mm", "usb thermal", "mini printer",
        "热敏", "票据", "小票",
    ];
    if (thermalTokens.some((t) => signature.includes(t))) {
        return { mode: "escpos", printerClass: "thermal" };
    }
    return { mode: "gdi", printerClass: "office" };
}

function isIpLiteral(value) {
    const text = String(value || "").trim();
    // IPv4: validate octet ranges (0-255) so hostname-shaped strings are rejected.
    const v4 = text.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
    if (v4) return v4.slice(1).every((o) => Number(o) <= 255);
    if (!text.includes(":")) return false;
    // IPv6: hex groups with at most one "::" compression and no
    // stray leading/trailing single colon.
    return /^[0-9a-fA-F:]+$/.test(text)
        && !text.includes(":::")
        && (text.match(/::/g) || []).length <= 1
        && (!text.startsWith(":") || text.startsWith("::"))
        && (!text.endsWith(":") || text.endsWith("::"));
}

/** Format driver forms into one-line summaries (accepts nested f.size or flat fields). */
export function formatFormsSummary(forms, rollLabel) {
    return (forms || []).map((f) => {
        if (typeof f === "string") {
            return f.trim() || "Form";
        }
        const size = (f && (f.size || f)) || {};
        const dim = (v) => (v != null && v !== "" && Number.isFinite(Number(v)) ? Math.round(Number(v)) : "?");
        const w = dim(size.width_mm);
        const h = dim(size.height_mm);
        return (
            `${(f && f.name) || "Form"} — ${w}×${h}mm` +
            (f && f.roll ? " (" + rollLabel + ")" : "")
        );
    });
}

export function printerPatternFor(printer, mode) {
    const rawModes = ["escpos", "zpl", "tspl", "epl", "cpcl"];
    const port = String((printer && printer.port) || "").trim();
    if (rawModes.includes(mode) && isIpLiteral(port)) {
        // Bracket IPv6 literals in URI authority so the port suffix is unambiguous.
        const host = port.includes(":") ? `[${port}]` : port;
        return `tcp://${host}:9100`;
    }
    return (printer && printer.name) || "";
}

/** Receipt roll width from the driver-reported forms, if any. */
export function rollWidthMm(capabilities) {
    const forms = (capabilities && capabilities.forms) || [];
    for (const form of forms) {
        if (typeof form === "string") {
            const match = form.match(/(\d+(?:\.\d+)?)\s*mm\s+roll/i);
            if (match && Number(match[1]) > 0) {
                return Number(match[1]);
            }
            continue;
        }
        const size = form && (form.size || form);
        const w = size && Number(size.width_mm);
        if (form && form.roll && Number.isFinite(w) && w > 0) {
            return w;
        }
    }
    const paperWidth = capabilities && Number(capabilities.paper_width_mm);
    if (Number.isFinite(paperWidth) && paperWidth > 0 && paperWidth <= 112) {
        return paperWidth;
    }
    return null;
}

const NON_BLOCKING_STATUSES = new Set(["ready", "printing", "unknown"]);

function isHealthyPrinter(printer) {
    if (!printer || !printer.status) return true;
    return NON_BLOCKING_STATUSES.has(String(printer.status).toLowerCase());
}

function findPrinterByName(printers, wantedName) {
    if (!wantedName) return null;
    const exact = printers.find((p) => p && p.name === wantedName);
    if (exact) return exact;
    const norm = String(wantedName).trim().toLowerCase();
    return norm
        ? (printers.find((p) => p && String(p.name || "").trim().toLowerCase() === norm) || null)
        : null;
}

/**
 * Resolve the printer to use RIGHT NOW on this PC: the user's
 * selection when it is still connected, else a preferred-class
 * printer when asked (POS receipts prefer thermal/label, favoring
 * healthy printers over offline ones), else the healthy OS default,
 * else the first healthy printer. Returns null when this PC has
 * no printer at the moment — callers then keep Odoo's normal flow.
 */
const CAPABILITIES_MEMO_TTL_MS = 120_000;
const capabilitiesMemo = new WeakMap();

async function cachedPrinterCapabilities(bridge, printerName) {
    if (!bridge || typeof bridge.printerCapabilities !== "function") {
        return null;
    }
    let map = capabilitiesMemo.get(bridge);
    if (!map) {
        map = new Map();
        capabilitiesMemo.set(bridge, map);
    }
    const now = Date.now();
    const cached = map.get(printerName);
    if (cached && now - cached.ts < CAPABILITIES_MEMO_TTL_MS) {
        return cached.value;
    }
    try {
        const caps = await bridge.printerCapabilities(printerName);
        map.set(printerName, { ts: now, value: caps });
        return caps;
    } catch (_error) {
        return null;
    }
}

export async function resolveBridgeTarget(bridge, options = {}) {
    if (!bridge || typeof bridge.listPrinters !== "function") return null;
    let printers = await bridge.listPrinters(options.force ? { force: true } : undefined);
    if ((!Array.isArray(printers) || !printers.length) && !options.force) {
        printers = await bridge.listPrinters({ force: true });
    }
    if (!Array.isArray(printers) || !printers.length) return null;
    const prefer = options.preferClasses || [];
    const classOf = (p) => detectPrinterType(p).printerClass;
    const isDefault = (p) => Boolean(p && (p.is_default || p.isDefault));
    const selected = findPrinterByName(printers, getSelectedPrinter());
    const selectedPos = options.isPos
        ? findPrinterByName(printers, getSelectedPosPrinter())
        : null;
    let printer = null;
    // A reprint from the job log names the printer the job originally
    // ran on: when that printer is connected to this PC right now it
    // wins over every other preference; when it is not, resolution
    // falls through to the normal order below.
    if (options.preferPrinterName) {
        printer = findPrinterByName(printers, options.preferPrinterName);
    }
    // Dedicated POS printer selection (when explicitly configured) wins
    // for POS jobs without affecting normal report printing.
    if (!printer && selectedPos) {
        printer = selectedPos;
    }
    // Explicit user selection wins over class preference; class preference
    // only breaks ties when nothing is explicitly selected.
    if (!printer && selected) {
        printer = selected;
    }
    if (!printer && prefer.length) {
        const classMatches = printers.filter((p) => prefer.includes(classOf(p)));
        if (classMatches.length) {
            printer =
                classMatches.find((p) => isDefault(p) && isHealthyPrinter(p)) ||
                classMatches.find((p) => isHealthyPrinter(p)) ||
                classMatches.find((p) => isDefault(p)) ||
                classMatches[0];
        }
    }
    if (!printer) {
        printer =
            printers.find((p) => isDefault(p) && isHealthyPrinter(p)) ||
            printers.find((p) => isHealthyPrinter(p)) ||
            printers.find((p) => isDefault(p)) ||
            printers[0];
    }
    const capabilities = await cachedPrinterCapabilities(bridge, printer.name);
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
            // Tri-state: null leaves saved per-printer or driver default in charge.
            duplex: null,
            color: !capabilities || (capabilities.color !== false && capabilities.supports_color !== false),
            scale_policy: "custom_allowed",
            media: { roll_width_mm: rollWidthMm(capabilities) },
        },
    };
}

// User notifications: one helper for every Yaseir Print message, so
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

const NOTIFICATION_DEDUPE_MS = 4000;
const shownNotifications = new Map();

function nativeBridge() {
    return typeof window !== "undefined" ? window.__NATIVE_PRINT__ || null : null;
}

function isNativeApp() {
    return Boolean(nativeBridge());
}

export function recoveryButtons() {
    if (isNativeApp() && typeof nativeBridge()?.openControlCenter === "function") {
        return [
            {
                name: _t("Open Control Center"),
                primary: true,
                onClick: () => Promise.resolve().then(() => nativeBridge()?.openControlCenter?.()).catch(() => {}),
            },
        ];
    }
    return null;
}

export function notifyUser(env, message, type = "info", title = _t("Yaseir Print"), buttons = null) {
    try {
        const notification =
            env && typeof env.add === "function"
                ? env
                : env && env.services && env.services.notification;
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
            className: "o_native_print_notification",
            sticky: type === "danger" || type === "warning",
            ...(Array.isArray(buttons) && buttons.length ? { buttons } : {}),
        });
    } catch (_err) {
        // Informational only; never block printing.
    }
}
