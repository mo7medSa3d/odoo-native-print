# Yaseir Print Android App (`android-app/`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `user:subagent-driven-development` (recommended) or `user:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a production-grade Android companion browser and silent printing app (`android-app/`) inside `odoo-native-print` that delivers full functional, visual, and protocol parity with the Windows desktop app ("Yaseir Print" v1.5.7) — allowing `odoo19/native_print` and `odoo20/native_print` to print silently from Android phones and POS tablets with zero Odoo addon changes.

**Architecture:** Native Kotlin + Jetpack Compose + Android `WebView` (`minSdk 26`, `targetSdk 35`, Gradle 8.x / Kotlin 2.x). Instead of cross-compiling Tauri/Rust via NDK (which lacks reliable multi-tab WebView lifecycle, Bluetooth SPP, USB OTG bulk transfer, and native `PdfRenderer`), `android-app/` implements the exact `print-core` / `print-raw` / `print-ipp` / `print-render` semantics in clean, modular Kotlin backed by SQLite WAL (`Room` / `SQLiteOpenHelper`), Android `PdfRenderer` (zero 7.5 MB PDFium `.so` bloat), `NsdManager` mDNS/IPP discovery, TCP 9100 deadline sockets, Bluetooth Classic SPP (`RFCOMM`), USB Host OTG bulk transfer, and an `Android PrintManager` fallback. The Odoo WebView receives the exact frozen, top-frame-only `window.__NATIVE_PRINT__` v1 JS facade via `androidx.webkit.WebViewCompat.addWebMessageListener` with strict origin allowlisting (mirroring ADR-0002 & ADR-0005). All visual assets (`app-icon.png`, `Thmanyah Sans` 5 OTF weights, and `theme.css` Light/Dark tokens + Arabic RTL/English LTR) are reused directly from `app/dist/`.

**Tech Stack:** Kotlin 2.0+, Jetpack Compose (Material 3 customized strictly to `app/dist/theme.css` tokens), AndroidX WebKit (`WebViewCompat.addWebMessageListener`, `WebViewFeature.DOCUMENT_START_SCRIPT`), Kotlin Coroutines, SQLite WAL (via `SQLiteOpenHelper` — zero ORM bloat), Android `PdfRenderer`, `NsdManager`, `BluetoothAdapter`/`BluetoothSocket`, `UsbManager`, `ForegroundService` (`connectedDevice` + `dataSync`), JUnit 5 + Robolectric/MockWebServer for JVM unit tests, GitHub Actions CI (`.github/workflows/android.yml`).

**Spec:** Derived from audited Windows desktop repo (`app/src-tauri/tauri.conf.json` v1.5.7, `docs/api-contract.md`, `app/src-tauri/src/lib.rs` `BRIDGE_JS`, `app/dist/theme.css`, `crates/print-core`, `crates/print-raw`, `crates/print-ipp`, `crates/print-render`, `odoo19/native_print`, `odoo20/native_print`).

**Global Constraints:**
- `android-app/` lives cleanly at the root of `odoo-native-print-main/android-app/` and never modifies `crates/` or breaks `tools/check_command_surface.py` or `tools/check_versions.py`.
- Zero changes required in `odoo19/native_print` or `odoo20/native_print`: `window.__NATIVE_PRINT__` must expose the exact 13 remote methods, payload shapes, response fields, and `version = "1.5.7"` (kept in sync with `tools/check_versions.py`).
- Security parity (ADR-0002 & ADR-0005): HTTPS allowed for any host; HTTP allowed ONLY for IP literals (rejecting cloud metadata `169.254.169.254` and non-IP HTTP hostnames); `window.__NATIVE_PRINT__` injected ONLY on the configured origin's top frame (`window === window.top`), frozen & non-configurable; Control Center admin commands (`runTestPrint`, `retryJob`, `cancelJob`, `clearFinished`, `exportDiagnostics`, `saveOrigin`, `getPrinterSettings`, `setPrinterSettings`, `clearWebData`) are strictly native Kotlin UI actions and NEVER callable from remote WebView JS.
- Payload & backpressure parity: PDF `<= 50 MiB`, `<= 2,000` pages, `copies 1..=99`; PNG `<= 20 MiB`, `<= 40,000 px` side, `<= 50,000,000` pixels; text `<= 1 MiB`; max `500` active jobs/printer, `2,000` global active jobs.
- Lean `ponytail` design: no Hilt/Dagger DI graph, no Retrofit/Moshi ceremony, no unnecessary abstraction layers — plain Kotlin constructor injection (`AppContainer`), `org.json` / `kotlinx.serialization`, and direct Android platform APIs.
- Offline VM note: This workspace VM lacks `javac`/`gradle`/`adb`, so all unit tests and APK builds are verified via `./gradlew :app:testDebugUnitTest :app:assembleDebug` locally in Android Studio or on GitHub Actions (`android.yml`).

**Review Focus:**
- **Origin & Subframe Bridge Bypass** — an embedded third-party iframe or off-origin redirect in Odoo WebView must NEVER access `window.__NATIVE_PRINT__` or invoke admin actions; pinned to Task 7 (`WebBridgeSecurityTest`).
- **Silent Print Killed in Background / Screen Off** — Android Doze or app switching mid-receipt must not drop queued jobs; jobs persist in SQLite WAL before returning `jobId` and execute inside a bound `PrintForegroundService` with startup reconciliation (`SUBMITTED`/`IN_PROGRESS` recovery); pinned to Task 3 (`JobQueueStoreTest`) & Task 6 (`PrintWorkerCoordinatorTest`).
- **Stale Printer Cache After Bluetooth/USB/Wi-Fi Change** — identical to the desktop stale-cache bug: `PrinterDiscoveryService` uses a single 12-second TTL cache, invalidates immediately on `force = true` (`appOverview({ force: true })` / `listPrinters({ force: true })`), and bumps `capsEpoch` on refresh or USB/BT attach/detach broadcasts; pinned to Task 5 (`PrinterDiscoveryCacheTest`).
- **Thermal / Label Protocol Byte Fidelity & Arabic Rendering** — ESC/POS raster (`GS v 0`), ZPL (`^GFA`), TSPL (`BITMAP`), EPL (`GW`), and CPCL (`EG`) must emit exact byte headers and 1-bit Floyd-Steinberg/threshold mono bitmaps, and Arabic text/receipts must render via Android `Canvas`/`StaticLayout` (full Arabic shaping & RTL) to raster bitmap when raw codepages lack Arabic glyphs; pinned to Task 4 (`RawProtocolEncodersTest`).
- **TCP 9100 / Bluetooth SPP / USB OTG Partial Write or Hang** — offline/paper-out printers must never block other printers; per-printer serialized worker + strict connect/read/write deadlines (`5s` connect, `15s` write, `10s` drain) + bounded exponential backoff and dead-letter transition; pinned to Task 5 (`Tcp9100TransportTest`) & Task 6 (`PrintWorkerCoordinatorTest`).

---

## Target Directory Map (`android-app/`)

```text
android-app/
├── settings.gradle.kts
├── build.gradle.kts
├── gradle.properties
├── gradle/libs.versions.toml
└── app/
    ├── build.gradle.kts
    ├── proguard-rules.pro
    └── src/
        ├── main/
        │   ├── AndroidManifest.xml
        │   ├── assets/
        │   │   └── bridge.js                          # Exact window.__NATIVE_PRINT__ v1 facade + error tap + boot watchdog + HTTP strip
        │   ├── res/
        │   │   ├── font/
        │   │   │   ├── thmanyah_sans_light.otf        # Copied from app/dist/fonts/
        │   │   │   ├── thmanyah_sans_regular.otf
        │   │   │   ├── thmanyah_sans_medium.otf
        │   │   │   ├── thmanyah_sans_bold.otf
        │   │   │   └── thmanyah_sans_black.otf
        │   │   ├── drawable/ic_yaseir_logo.png        # Copied from app/dist/app-icon.png
        │   │   ├── mipmap-*/ic_launcher*.png          # Generated from app/src-tauri/icons/icon.png
        │   │   ├── xml/usb_device_filter.xml          # USB Printer Class (0x07) filter
        │   │   ├── values/strings.xml                 # English strings
        │   │   └── values-ar/strings.xml              # Arabic strings (primary POS/Odoo UX)
        │   └── java/com/company/odoonativeprint/
        │       ├── YaseirPrintApp.kt                  # Application class + lightweight AppContainer
        │       ├── MainActivity.kt                    # Single-Activity Compose host + USB/BT intents + permissions
        │       ├── core/
        │       │   ├── OriginPolicy.kt                # ADR-0002 HTTPS / IP-literal validator (blocks 169.254.169.254)
        │       │   ├── AppConfigStore.kt              # Configured origin, autoReprint, theme, language, per-printer settings
        │       │   ├── Models.kt                      # PrintJob, JobState (8 states), PrinterInfo, PrinterCapabilities, AppOverview
        │       │   ├── JobQueueStore.kt               # SQLite WAL durable queue, idempotency dedupe, backpressure, startup recovery
        │       │   ├── DiagnosticsExporter.kt         # Redacted ZIP/JSON diagnostics bundle via FileProvider share sheet
        │       │   └── PayloadLimits.kt               # 50 MiB PDF / 20 MiB PNG / 1 MiB text / 2000 pages / 1..99 copies
        │       ├── render/
        │       │   ├── MonoBitmap.kt                  # 1-bit monochrome packer + threshold & Floyd-Steinberg dithering
        │       │   ├── PdfPageRenderer.kt             # Android PdfRenderer -> ARGB_8888 Bitmap at target DPI & paper geometry
        │       │   ├── TextReceiptRenderer.kt         # Arabic/RTL-safe StaticLayout text -> Bitmap fallback for thermal printers
        │       │   └── CalibrationPages.kt            # Ruler/alignment/thermal test page generators matching desktop
        │       ├── protocol/
        │       │   ├── EscPosEncoder.kt               # ESC @, GS v 0 raster, paper cut (GS V), cash drawer pulse (ESC p)
        │       │   ├── ZplEncoder.kt                  # ^XA ... ^GFA ... ^XZ
        │       │   ├── TsplEncoder.kt                 # SIZE, GAP, CLS, BITMAP, PRINT
        │       │   ├── EplEncoder.kt                  # N, GW, P1
        │       │   ├── CpclEncoder.kt                 # ! 0 200 200 ... EG ... PRINT
        │       │   └── IppClient.kt                   # Pure-Kotlin RFC 8011 IPP Get-Printer-Attributes + Print-Job
        │       ├── transport/
        │       │   ├── PrinterTransport.kt            # Sealed interface + TransportResult
        │       │   ├── Tcp9100Transport.kt            # Raw TCP 9100 with connect/write/drain deadlines
        │       │   ├── BluetoothSppTransport.kt       # Classic BT RFCOMM (UUID 00001101-0000-1000-8000-00805F9B34FB)
        │       │   ├── UsbOtgTransport.kt             # Android UsbManager bulk OUT endpoint transfer (USB Class 7)
        │       │   ├── IppTransport.kt                # Network IPP/IPPS PDF/raster submission
        │       │   ├── AndroidPrintManagerAdapter.kt  # Optional fallback to Android system PrintManager (Mopria/Cloud)
        │       │   └── PrinterDiscoveryService.kt     # Unified mDNS/NSD + Paired BT + USB OTG + Manual TCP/IPP (12s TTL + force)
        │       ├── worker/
        │       │   ├── PrintWorkerCoordinator.kt      # Per-printer serialized coroutine actors + backoff + dead-letter
        │       │   └── PrintForegroundService.kt      # Keeps active print queue alive during screen-off / app switch
        │       ├── bridge/
        │       │   ├── NativePrintBridge.kt           # Origin-verified WebMessageListener dispatching the 13 remote commands
        │       │   └── OdooWebViewFactory.kt          # Hardened WebView config (cookies, DOM storage, mixed content policy, error tap)
        │       └── ui/
        │           ├── theme/
        │           │   ├── YaseirColors.kt            # Exact hex tokens from app/dist/theme.css (Light + Dark)
        │           │   ├── YaseirTypography.kt        # Thmanyah Sans 5-weight FontFamily + RTL line-height 1.65
        │           │   └── YaseirTheme.kt             # System/Light/Dark + RTL LayoutDirection wrapper
        │           ├── browser/
        │           │   ├── TabManager.kt              # Multi-tab WebView state (open, switch, close, reorder, back/forward/reload)
        │           │   ├── BrowserScreen.kt           # Desktop-style compact tab bar + URL/status indicator + WebView container
        │           │   └── BootScreen.kt              # First-run origin setup + live origin probe (matches app/dist/boot.html)
        │           ├── controlcenter/
        │           │   ├── ControlCenterScreen.kt     # Printers, Queue, Health, Diagnostics tabs (matches app/dist/index.html)
        │           │   ├── PrinterConfigSheet.kt      # Add/Edit TCP 9100 / IPP / BT / USB printer + paper/DPI/protocol/test print
        │           │   └── JobDetailSheet.kt          # Job timeline, error code, retry/cancel actions
        │           └── settings/
        │               └── SettingsScreen.kt          # Origin, auto-reprint, theme, language, clear web data (matches settings.html)
        └── test/java/com/company/odoonativeprint/
            ├── core/OriginPolicyTest.kt
            ├── core/JobQueueStoreTest.kt
            ├── protocol/RawProtocolEncodersTest.kt
            ├── protocol/IppClientTest.kt
            ├── transport/Tcp9100TransportTest.kt
            ├── transport/PrinterDiscoveryCacheTest.kt
            ├── worker/PrintWorkerCoordinatorTest.kt
            └── bridge/WebBridgeSecurityTest.kt
```

---

## Task 1: Gradle Project Scaffold, Branding Assets & Design System (`theme.css` Parity)

**Files:**
- Create: `android-app/settings.gradle.kts`, `android-app/build.gradle.kts`, `android-app/gradle.properties`, `android-app/gradle/libs.versions.toml`
- Create: `android-app/app/build.gradle.kts`, `android-app/app/proguard-rules.pro`, `android-app/app/src/main/AndroidManifest.xml`, `android-app/app/src/main/res/xml/usb_device_filter.xml`
- Copy: `app/dist/fonts/thmanyahsans-*.otf` -> `android-app/app/src/main/res/font/thmanyah_sans_*.otf`
- Copy: `app/dist/app-icon.png` -> `android-app/app/src/main/res/drawable/ic_yaseir_logo.png` and `app/src-tauri/icons/icon.png` -> mipmap launcher icons
- Create: `android-app/app/src/main/res/values/strings.xml`, `android-app/app/src/main/res/values-ar/strings.xml`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/ui/theme/{YaseirColors.kt,YaseirTypography.kt,YaseirTheme.kt}`

- [ ] **Step 1: Scaffold Gradle project with exact app identity**
  - `applicationId = "com.company.odoonativeprint"` (valid Android package name mirroring `com.company.odoo-native-print`)
  - `versionName = "1.5.7"`, `versionCode = 10507`, `minSdk = 26`, `targetSdk = 35`, `compileSdk = 35`
  - Dependencies in `libs.versions.toml`: `androidx.core:core-ktx`, `androidx.activity:activity-compose`, Compose BOM + Material 3 + Material Icons Extended, `androidx.webkit:webkit`, `org.jetbrains.kotlinx:kotlinx-coroutines-android`, `junit-jupiter`, `org.json:json` (for JVM unit tests), `okhttp3:mockwebserver` (for IPP/HTTP tests).
  - `AndroidManifest.xml` permissions: `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE` (mDNS), `BLUETOOTH`, `BLUETOOTH_ADMIN` (maxSdk 30), `BLUETOOTH_CONNECT`, `BLUETOOTH_SCAN` (API 31+, `neverForLocation`), `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `FOREGROUND_SERVICE_DATA_SYNC`, `POST_NOTIFICATIONS` (API 33+); `<uses-feature android:name="android.hardware.usb.host" android:required="false" />`.

- [ ] **Step 2: Copy desktop fonts and icon assets verbatim**
  - Copy all 5 `thmanyahsans-*.otf` files into `res/font/` with lowercase Android-compliant names (`thmanyah_sans_light.otf`, etc.).
  - Copy `app-icon.png` to `res/drawable/ic_yaseir_logo.png` and generate adaptive/legacy launcher mipmaps from `app/src-tauri/icons/icon.png` (using Pillow in Python).

- [ ] **Step 3: Implement exact Light/Dark Compose theme tokens from `app/dist/theme.css`**
  - `YaseirColors.kt`: exact hex palette (`#fcfcfc` / `#000000` canvas, `#ffffff` / `#0f0f0f` surface, `#fafafa` / `#141418` elevated, `#f3f4f6` / `#181818` subtle, `#e7e4e4` / `#26272e` border, `#0e142b` / `#ffffff` primary text, `#2e67ff` / `#2b65ff` brand, `#046c4e` / `#00eeba` ok, `#8c4a00` / `#f4ea55` warn, `#b8370a` / `#ff8559` err, `#ff6c35` destructive).
  - `YaseirTypography.kt`: `ThmanyahSans` `FontFamily` (Light 300, Normal 400, Medium 500, Bold 700, Black 900) with `letterSpacing = 0.sp` and comfortable Arabic line-height (`1.65em` body).
  - `YaseirTheme.kt`: `CompositionLocalProvider(LocalYaseirColors provides colors, LocalLayoutDirection provides ...)` supporting System / Light / Dark and Arabic RTL / English LTR.

- [ ] **Step 4: Verify asset integrity and Gradle syntax**
  - Run: `ls -la android-app/app/src/main/res/font/ android-app/app/src/main/res/drawable/`
  - Expected: 5 `.otf` files + `ic_yaseir_logo.png` present, non-zero sizes.

---

## Task 2: Origin Security Policy, Payload Limits & Config Store (ADR-0002 Parity)

**Files:**
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/core/OriginPolicy.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/core/PayloadLimits.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/core/Models.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/core/AppConfigStore.kt`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/core/OriginPolicyTest.kt`

- [ ] **Step 1: Write failing unit tests in `OriginPolicyTest.kt`**
  - Accepts `https://erp.company.com`, `https://erp.company.com:8443/web` (normalizes to `https://erp.company.com:8443`).
  - Accepts `http://192.168.1.50:8069`, `http://127.0.0.1:8069`, `http://[::1]:8069`, `http://45.10.20.30:8069`.
  - Rejects `http://erp.company.com` (HTTP hostname forbidden), `http://169.254.169.254` (cloud metadata IPv4), `http://[::ffff:169.254.169.254]` (IPv4-mapped metadata), `javascript:alert(1)`, `file:///etc/passwd`, `https://user:pass@erp.company.com` (embedded credentials).
  - Verifies `isSameOrigin(configuredOrigin, candidateUrl)` handles default ports (`:443` for https, `:80` for http) accurately.

- [ ] **Step 2: Implement `OriginPolicy.kt`, `PayloadLimits.kt`, `Models.kt`, and `AppConfigStore.kt`**
  - `OriginPolicy.validateAndNormalize(raw: String): Result<String>` mirroring `crates/print-core/src/security.rs` exactly.
  - `PayloadLimits`: `MAX_PDF_BYTES = 50 * 1024 * 1024`, `MAX_PDF_PAGES = 2_000`, `MIN_COPIES = 1`, `MAX_COPIES = 99`, `MAX_IMAGE_BYTES = 20 * 1024 * 1024`, `MAX_IMAGE_SIDE = 40_000`, `MAX_IMAGE_PIXELS = 50_000_000L`, `MAX_TEXT_BYTES = 1 * 1024 * 1024`, `MAX_ACTIVE_PER_PRINTER = 500`, `MAX_ACTIVE_GLOBAL = 2_000`.
  - `Models.kt`: `JobState` enum (`QUEUED`, `RENDERING`, `SPOOLING`, `SUBMITTED`, `IN_PROGRESS`, `COMPLETED`, `FAILED`, `CANCELLED`), `PrinterTransportType` (`TCP_9100`, `IPP`, `BLUETOOTH_SPP`, `USB_OTG`, `ANDROID_SYSTEM`), `PrinterProtocol` (`AUTO`, `PDF`, `ESCPOS`, `ZPL`, `TSPL`, `EPL`, `CPCL`), `PrinterInfo`, `PrinterCapabilities`, `PrinterSettings`, `PrintJobRecord`, `AppOverview`.
  - `AppConfigStore.kt`: persists `origin`, `autoReprint`, `themeMode` (`system`/`light`/`dark`), `language` (`ar`/`en`), manual printer entries, and per-printer `PrinterSettings` (paper size, DPI, protocol, darkness, cut, cash drawer, calibration offset) in `SharedPreferences` (mode `MODE_PRIVATE`) with atomic JSON serialization.

- [ ] **Step 3: Run unit tests (CI / Android Studio)**
  - Run: `./gradlew :app:testDebugUnitTest --tests "*.OriginPolicyTest"`
  - Expected: All 10+ security boundary cases PASS.

---

## Task 3: Durable SQLite WAL Job Queue, Idempotency & Startup Recovery

**Files:**
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/core/JobQueueStore.kt`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/core/JobQueueStoreTest.kt`

- [ ] **Step 1: Write failing tests in `JobQueueStoreTest.kt`**
  - Enqueue creates `QUEUED` job, persists payload file in app-private `filesDir/spool/<jobId>.bin`, and returns `jobId`.
  - Duplicate `idempotencyKey` within active/completed window returns existing `jobId` without creating a second job.
  - Backpressure: rejects 501st active job for the same printer with `PRINTER_QUEUE_FULL` and 2,001st global active job with `GLOBAL_QUEUE_FULL`.
  - Startup reconciliation (`reconcileInterruptedJobs()`): jobs stuck in `RENDERING`/`SPOOLING` reset to `QUEUED` (if `attempts < maxAttempts`) or `FAILED`; jobs stuck in `SUBMITTED`/`IN_PROGRESS` mark `FAILED` with `APP_RESTARTED_DURING_PRINT` (preventing blind duplicate receipts unless `autoReprint` is explicitly enabled).
  - `retryJob`, `cancelJob`, `clearFinished` obey valid state transitions and unlink orphaned spool payload files.

- [ ] **Step 2: Implement `JobQueueStore.kt`**
  - `SQLiteOpenHelper` with `enableWriteAheadLogging()` (`PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;`).
  - Table `print_jobs`: `id TEXT PRIMARY KEY`, `idempotency_key TEXT UNIQUE`, `printer_name TEXT NOT NULL`, `source_kind TEXT NOT NULL` (`pdf`/`image`/`text`/`test`), `state TEXT NOT NULL`, `attempts INTEGER NOT NULL DEFAULT 0`, `max_attempts INTEGER NOT NULL DEFAULT 3`, `pages INTEGER`, `copies INTEGER NOT NULL DEFAULT 1`, `options_json TEXT NOT NULL`, `payload_path TEXT NOT NULL`, `error_code TEXT`, `error_message TEXT`, `created_at_ms INTEGER NOT NULL`, `updated_at_ms INTEGER NOT NULL`, `next_retry_at_ms INTEGER NOT NULL DEFAULT 0`.
  - Strict indexes on `(printer_name, state)`, `(state, next_retry_at_ms)`, and `created_at_ms DESC`.
  - Clean `pruneOldCompletedJobs(keepLast = 500)` so POS tablets printing 1,000 receipts/day never bloat disk.

- [ ] **Step 3: Run unit tests (CI / Android Studio)**
  - Run: `./gradlew :app:testDebugUnitTest --tests "*.JobQueueStoreTest"`
  - Expected: All durability, dedupe, backpressure, and crash-recovery tests PASS.

---

## Task 4: Rendering Pipeline & Pure-Kotlin Raw Printer Encoders (ESC/POS, ZPL, TSPL, EPL, CPCL)

**Files:**
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/render/MonoBitmap.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/render/PdfPageRenderer.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/render/TextReceiptRenderer.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/render/CalibrationPages.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/protocol/{EscPosEncoder.kt,ZplEncoder.kt,TsplEncoder.kt,EplEncoder.kt,CpclEncoder.kt}`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/protocol/RawProtocolEncodersTest.kt`

- [ ] **Step 1: Write failing tests in `RawProtocolEncodersTest.kt`**
  - `MonoBitmap` packs an 8x2 test pattern into exact 1-bit big-endian row bytes (`0x80`, `0xFF`, etc.) and supports both threshold and Floyd-Steinberg dithering.
  - `EscPosEncoder.encodeRaster(mono, cut = true, openDrawer = true)` emits `ESC @` (`1B 40`), `GS v 0` (`1D 76 30 00 xL xH yL yH`), exact packed raster bytes, `ESC d 4` feed, `GS V 66 0` partial cut, and `ESC p 0 25 250` cash drawer pulse.
  - `ZplEncoder.encode(mono, widthDots, heightDots)` emits `^XA`, `^PW`, `^LL`, `^FO0,0^GFA,<bytes>,<bytes>,<rowBytes>,<hex>^FS`, `^XZ`.
  - `TsplEncoder.encode(mono, widthMm, heightMm)` emits `SIZE`, `GAP`, `CLS`, `BITMAP 0,0,<rowBytes>,<height>,0,<invertedRawBytes>`, `PRINT 1,1`.
  - `EplEncoder.encode(mono)` emits `N\n`, `GW0,0,<rowBytes>,<height>,<rawBytes>\n`, `P1\n`.
  - `CpclEncoder.encode(mono, heightDots)` emits `! 0 200 200 <height> 1\r\n`, `EG <rowBytes> <height> 0 0 <hex>\r\n`, `PRINT\r\n`.

- [ ] **Step 2: Implement `MonoBitmap.kt` and all 5 raw protocol encoders**
  - Match `crates/print-raw/src/{mono,escpos,zpl,tspl,epl,cpcl}.rs` byte-for-byte.
  - Ensure `EscPosEncoder` chunks tall receipts into `<= 256`-row bands so low-RAM 58mm/80mm Bluetooth thermal printers never overflow their receive buffer.

- [ ] **Step 3: Implement `PdfPageRenderer.kt`, `TextReceiptRenderer.kt`, and `CalibrationPages.kt`**
  - `PdfPageRenderer`: wraps `android.graphics.pdf.PdfRenderer` with a strict `Mutex` (Android `PdfRenderer` is single-threaded), validates `pageCount <= 2_000`, renders each requested page at target DPI (e.g., 203 DPI for thermal/label, 300 DPI for IPP/laser) onto white `ARGB_8888` bitmap, and applies per-printer scale/offset calibration.
  - `TextReceiptRenderer`: renders plain/Arabic text receipts using `StaticLayout` + `Thmanyah Sans` so Arabic ligatures, numerals, and RTL alignment print identically on every thermal printer regardless of hardware codepage.
  - `CalibrationPages`: generates the 80mm/58mm thermal test receipt and A4/label alignment ruler matching Windows `crates/print-render/src/calibration.rs`.

- [ ] **Step 4: Run unit tests (CI / Android Studio)**
  - Run: `./gradlew :app:testDebugUnitTest --tests "*.RawProtocolEncodersTest"`
  - Expected: All 5 protocol encoders + `MonoBitmap` tests PASS.

---

## Task 5: Printer Transports (TCP 9100, IPP, Bluetooth SPP, USB OTG, System) & 12s Single-Source Discovery

**Files:**
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/protocol/IppClient.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/transport/{PrinterTransport.kt,Tcp9100Transport.kt,IppTransport.kt,BluetoothSppTransport.kt,UsbOtgTransport.kt,AndroidPrintManagerAdapter.kt,PrinterDiscoveryService.kt}`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/protocol/IppClientTest.kt`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/transport/Tcp9100TransportTest.kt`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/transport/PrinterDiscoveryCacheTest.kt`

- [ ] **Step 1: Write failing tests (`IppClientTest`, `Tcp9100TransportTest`, `PrinterDiscoveryCacheTest`)**
  - `IppClientTest` (against `MockWebServer`): verifies RFC 8011 binary framing for `Get-Printer-Attributes` (0x000B) and `Print-Job` (0x0002) with `application/pdf`, `copies`, `media`, `sides`, and parses `successful-ok` vs `client-error-not-found`.
  - `Tcp9100TransportTest` (against local `ServerSocket`): verifies full byte delivery, chunked writes, and clean timeout failure (`PRINTER_TIMEOUT`) when peer stalls.
  - `PrinterDiscoveryCacheTest`: verifies 12-second TTL returns cached snapshot without re-probing, `discover(force = true)` immediately bypasses cache and increments `capsEpoch`, and `invalidate()` on USB/BT state change clears cached capabilities.

- [ ] **Step 2: Implement transports and `IppClient.kt`**
  - `Tcp9100Transport`: `Socket()` + `connect(addr, 5_000)` + `soTimeout = 15_000` + `tcpNoDelay = true` + 16 KiB chunked writes + flush + clean half-close drain (`shutdownOutput()`).
  - `IppClient` / `IppTransport`: pure-Kotlin RFC 8011 encoder/decoder over `HttpURLConnection` (supporting `ipp://`, `ipps://`, `http://`, `https://` on port 631) for network office/Mopria/AirPrint printers.
  - `BluetoothSppTransport`: Classic BT `createRfcommSocketToServiceRecord(SPP_UUID)` with fallback `createInsecureRfcommSocketToServiceRecord`, 512-byte paced writes (`10ms` pacing option for cheap thermal printers that drop bursts), explicit stream flush + `250ms` settle before close so the last cut command isn't truncated.
  - `UsbOtgTransport`: `UsbManager` permission check, finds `USB_CLASS_PRINTER (0x07)` bulk `USB_DIR_OUT` endpoint, claims interface with `force = true`, sends 4 KiB chunks via `bulkTransfer(..., timeout = 10_000)`, releases interface in `finally`.
  - `AndroidPrintManagerAdapter`: optional fallback for PDF jobs when user chooses "Android System Print" (opens Android's native Mopria/Cloud print dialog).

- [ ] **Step 3: Implement `PrinterDiscoveryService.kt`**
  - Aggregates 4 sources into one unified `List<PrinterInfo>`:
    1. Configured manual TCP 9100 / IPP printers (from `AppConfigStore`) with fast reachability probe.
    2. `NsdManager` mDNS discovery (`_ipp._tcp.` and `_pdl-datastream._tcp.`).
    3. Paired Classic Bluetooth devices filtered by printer Major/Minor device class (`0x0600` Imaging / Printer) or user-pinned BT devices.
    4. Connected USB OTG devices (`UsbManager.deviceList`, interface class `0x07`).
  - Single-source 12-second TTL cache + `force: Boolean` invalidation + `capsEpoch: AtomicLong` (exact fix carried over from the Windows audit).

- [ ] **Step 4: Run unit tests (CI / Android Studio)**
  - Run: `./gradlew :app:testDebugUnitTest --tests "*.IppClientTest" --tests "*.Tcp9100TransportTest" --tests "*.PrinterDiscoveryCacheTest"`
  - Expected: All transport, IPP framing, and discovery cache tests PASS.

---

## Task 6: Per-Printer Serialized Worker Coordinator & Foreground Service

**Files:**
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/worker/PrintWorkerCoordinator.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/worker/PrintForegroundService.kt`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/worker/PrintWorkerCoordinatorTest.kt`

- [ ] **Step 1: Write failing tests in `PrintWorkerCoordinatorTest.kt`**
  - Jobs for the **same** printer execute strictly FIFO (never interleaved on the socket/BT stream).
  - Jobs for **different** printers execute concurrently (a stalled Kitchen TCP printer does not block the Cashier USB printer).
  - Transient transport error (`IOException`) triggers exponential backoff (`2s`, `6s`, `15s`) up to `maxAttempts = 3`, then transitions to `FAILED` with structured `errorCode`.
  - Non-retryable errors (`PAYLOAD_INVALID`, `PRINTER_NOT_FOUND`, `USB_PERMISSION_DENIED`) fail immediately on attempt 1.
  - `cancelJob(id)` cancels queued jobs immediately and marks in-flight jobs cancelled cleanly.

- [ ] **Step 2: Implement `PrintWorkerCoordinator.kt` and `PrintForegroundService.kt`**
  - `PrintWorkerCoordinator`: manages a `ConcurrentHashMap<String, Job>` of per-printer coroutine workers on `Dispatchers.IO`, drives `JobQueueStore` transitions (`QUEUED -> RENDERING -> SPOOLING -> SUBMITTED -> COMPLETED/FAILED`), and exposes a `StateFlow<QueueSummary>` for live UI updates.
  - `PrintForegroundService`: starts automatically whenever `activeJobs > 0` with a low-priority persistent notification ("Yaseir Print — جاري الطباعة..."), and calls `stopSelf()` cleanly 5 seconds after the queue drains so battery impact is zero when idle.

- [ ] **Step 3: Run unit tests (CI / Android Studio)**
  - Run: `./gradlew :app:testDebugUnitTest --tests "*.PrintWorkerCoordinatorTest"`
  - Expected: Serialization, concurrency, backoff, and cancellation tests PASS.

---

## Task 7: Secure WebView Bridge (`window.__NATIVE_PRINT__` v1 Parity)

**Files:**
- Create: `android-app/app/src/main/assets/bridge.js`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/bridge/NativePrintBridge.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/bridge/OdooWebViewFactory.kt`
- Test: `android-app/app/src/test/java/com/company/odoonativeprint/bridge/WebBridgeSecurityTest.kt`

- [ ] **Step 1: Write failing security & contract tests in `WebBridgeSecurityTest.kt`**
  - Allows all 13 remote commands (`print_pdf`, `print_text`, `print_image`, `print_status`, `job_detail`, `app_overview`, `list_printers`, `printer_forms`, `printer_capabilities`, `open_control_center`, `open_settings`, `tab_open`, `report_environment`) ONLY when `isMainFrame == true` AND `sourceOrigin == configuredOrigin`.
  - Rejects every call when `isMainFrame == false` (third-party iframe) with `ERR_SUBFRAME_FORBIDDEN`.
  - Rejects every call when `sourceOrigin != configuredOrigin` with `ERR_ORIGIN_MISMATCH`.
  - Rejects every admin-only command (`run_test_print`, `retry_job_cmd`, `cancel_job_cmd`, `clear_finished_cmd`, `export_diagnostics`, `save_origin_cmd`, `get_printer_settings`, `set_printer_settings`, `clear_web_data_cmd`) from WebView JS with `ERR_ADMIN_LOCAL_ONLY`.
  - Validates `tab_open(url)` rejects off-origin URLs (e.g., `https://evil.example`).

- [ ] **Step 2: Author `android-app/app/src/main/assets/bridge.js`**
  - Exact port of `BRIDGE_JS` from `app/src-tauri/src/lib.rs`, replacing `__TAURI_INTERNALS__.invoke(cmd, args)` with a request-ID Promise multiplexer over `window.__NP_ANDROID_PORT__.postMessage(JSON.stringify({ id, cmd, args }))` (backed by `WebViewCompat.addWebMessageListener`).
  - Preserves verbatim:
    - Top-frame-only guard (`window === window.top`), `Object.freeze(__npFacade)`, non-writable/non-configurable `window.__NATIVE_PRINT__`.
    - `version: "1.5.7"` and `normalizeBytes()` (Base64 encoding for `ArrayBuffer`/`Uint8Array` payloads).
    - `window.print = function () {};` suppression on remote Odoo pages.
    - Error tap (`window.__NP_ERRORS__`), `npProbeEnvironment()`, `npBootWatchdog()` (corner diagnostic card in Arabic with Reload / Control Center buttons), and `npHttpWarning()` strip on HTTP IP origins.

- [ ] **Step 3: Implement `NativePrintBridge.kt` & `OdooWebViewFactory.kt`**
  - `WebViewCompat.addDocumentStartJavaScript(webView, bridgeJs, setOf(configuredOrigin))` when `WebViewFeature.DOCUMENT_START_SCRIPT` is supported (with safe `onPageStarted` fallback on older WebView builds).
  - `WebViewCompat.addWebMessageListener(webView, "__NP_ANDROID_PORT__", setOf(configuredOrigin), listener)` — Android WebView itself enforces origin scoping at the Chromium IPC layer, AND `NativePrintBridge` double-checks `isMainFrame` + `OriginPolicy.isSameOrigin(configuredOrigin, sourceOrigin.toString())` (defense-in-depth).
  - `OdooWebViewFactory`: enables JavaScript, DOM Storage, third-party cookies (`CookieManager.setAcceptThirdPartyCookies`), WebSocket, hardware acceleration, zoom/desktop viewport support for Odoo POS & backend, file upload (`onShowFileChooser`), and intercepts `shouldOverrideUrlLoading` so `target="_blank"` on the same origin opens a new in-app tab while external URLs open in the system browser.

- [ ] **Step 4: Run unit tests (CI / Android Studio)**
  - Run: `./gradlew :app:testDebugUnitTest --tests "*.WebBridgeSecurityTest"`
  - Expected: All 13 remote commands, subframe block, off-origin block, and admin-only block tests PASS.

---

## Task 8: Multi-Tab Browser, Boot Screen, Control Center & Settings UI (Jetpack Compose)

**Files:**
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/ui/browser/{TabManager.kt,BrowserScreen.kt,BootScreen.kt}`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/ui/controlcenter/{ControlCenterScreen.kt,PrinterConfigSheet.kt,JobDetailSheet.kt}`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/ui/settings/SettingsScreen.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/core/DiagnosticsExporter.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/YaseirPrintApp.kt`
- Create: `android-app/app/src/main/java/com/company/odoonativeprint/MainActivity.kt`

- [ ] **Step 1: Build `TabManager.kt` & `BrowserScreen.kt` (Desktop-style Browser UX)**
  - Compact top chrome bar matching `app/dist/tabbar.html`: Yaseir icon, Back / Forward / Reload buttons, horizontal scrollable pill tabs with titles + close (`×`) + `+` new tab button (capped cleanly at 8 live WebViews to protect tablet RAM), live printer/queue status pill (green/amber/red dot + active job count) that opens the Control Center, and Settings gear button.
  - Fullscreen POS toggle (collapses tab bar into a floating corner pill for 10" POS tablets running Odoo Point of Sale).
  - Properly preserves `WebView` instances across tab switches and configuration changes (orientation/keyboard) without reloading Odoo POS sessions!

- [ ] **Step 2: Build `BootScreen.kt` (matching `app/dist/boot.html`)**
  - Clean centered card with `ic_yaseir_logo.png`, Arabic/English title ("Yaseir Print — ربط خادم Odoo"), origin URL input with instant `OriginPolicy` validation, live "اختبار الاتصال" (`probe_origin` HTTP HEAD/GET check), and "حفظ وفتح Odoo" CTA.

- [ ] **Step 3: Build `ControlCenterScreen.kt`, `PrinterConfigSheet.kt`, and `JobDetailSheet.kt` (matching `app/dist/index.html`)**
  - Three clean tabs:
    1. **الطابعات (Printers):** Pull-to-refresh (`force = true`), grouped by Connected / Discovered (Network TCP/IPP, Bluetooth SPP, USB OTG, System), "+ إضافة طابعة شبكة/بلوتوث" button, per-printer settings sheet (paper width 58mm/80mm/A4/custom label, DPI 203/300, protocol ESC/POS/ZPL/TSPL/EPL/CPCL/IPP, auto-cut, cash drawer kick), and "طباعة تجريبية" (`runTestPrint`) button.
    2. **سجل الطباعة (Queue):** Live job list with status badges (`مكتملة`, `قيد الطباعة`, `فشلت`, `ملغاة`), filter pills, "إعادة المحاولة" (`retryJob`), "إلغاء" (`cancelJob`), and "مسح المكتملة" (`clearFinished`).
    3. **الحالة والتشخيص (Health & Diagnostics):** Active origin, WebView version, environment probe facts (`ES2022`, `localStorage`, `cookies`, `WebSocket`, captured JS errors), and "تصدير حزمة التشخيص" (`DiagnosticsExporter` -> Android Share Sheet via `FileProvider`, strictly redacting cookies/tokens).

- [ ] **Step 4: Build `SettingsScreen.kt` (matching `app/dist/settings.html`) & wire `MainActivity.kt`**
  - Edit Odoo origin, toggle `autoReprint`, Theme selector (System / Light / Dark), Language selector (العربية / English), and "مسح بيانات التصفح" (`clearWebData` with confirmation dialog wiping `CookieManager`, `WebStorage`, and WebView cache).
  - `MainActivity.kt` handles runtime permissions (`BLUETOOTH_CONNECT`, `BLUETOOTH_SCAN`, `POST_NOTIFICATIONS`) gracefully with Arabic explanations and USB OTG attach/detach broadcasts (`UsbManager.ACTION_USB_DEVICE_ATTACHED` / `DETACHED`) that immediately invalidate `PrinterDiscoveryService`.

---

## Task 9: Repo Integration, Version Sync & GitHub Actions CI (`android.yml`)

**Files:**
- Modify: `tools/check_versions.py` (add optional/verified check for `android-app/app/build.gradle.kts` `versionName` so desktop and Android versions never drift)
- Create: `.github/workflows/android.yml`
- Modify: `README.md` (add Android Companion App section with build & install instructions)

- [ ] **Step 1: Add `.github/workflows/android.yml`**
  - Triggers on `push` / `pull_request` touching `android-app/**` and on `workflow_dispatch`.
  - Runs on `ubuntu-latest` with JDK 17 (`actions/setup-java@v4`) and Gradle cache (`gradle/actions/setup-gradle@v4`):
    1. `./gradlew :app:testDebugUnitTest`
    2. `./gradlew :app:lintDebug`
    3. `./gradlew :app:assembleDebug`
    4. Uploads `app-debug.apk` as a workflow artifact.

- [ ] **Step 2: Sync `tools/check_versions.py` and run existing repo gates**
  - Add `android-app/app/build.gradle.kts` `versionName = "..."` check to `tools/check_versions.py`.
  - Run:
    - `python3 tools/check_versions.py`
    - `python3 tools/check_command_surface.py`
  - Expected: Both PASS with `android: 1.5.7` matching `app: 1.5.7`.

- [ ] **Step 3: Final verification checklist before delivery**
  - Verify all files under `android-app/` exist with zero placeholder `TODO` stubs in production paths.
  - Verify `odoo19/` and `odoo20/` are completely untouched.
  - Verify `tools/check_versions.py` and `tools/check_command_surface.py` pass cleanly.
  - Note clearly in final report that Gradle JVM tests and APK assembly must be executed via `./gradlew :app:testDebugUnitTest :app:assembleDebug` in Android Studio or GitHub Actions (`android.yml`) because the offline VM lacks `javac`/`gradle`/`adb`.
