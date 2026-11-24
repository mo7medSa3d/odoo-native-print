# Design System — iOS 27 Liquid Glass Tokens

Canonical reference for the visual tokens implemented across:
- **Desktop Control Center:** [`app/dist/theme.css`](../app/dist/theme.css)
- **Odoo 19 Systray Popover:** [`odoo/native_print/static/src/scss/native_print.scss`, `odoo/native_print/static/src/scss/native_print_dark.scss`](../odoo/native_print/static/src/scss/native_print.scss)
- **Odoo 20 Systray Popover:** [`odoo20/native_print/static/src/scss/native_print.scss`](../odoo20/native_print/static/src/scss/native_print.scss) & [`odoo20/native_print/static/src/scss/native_print_dark.scss`](../odoo20/native_print/static/src/scss/native_print_dark.scss)

## 1. Color Tokens (Light & Dark)

### Desktop App (`app/dist/theme.css`)

| Role | CSS Variable | Light (`:root`) | Dark (`:root[data-theme="dark"]`) |
| --- | --- | --- | --- |
| Canvas | `--bg-canvas` | `#fcfcfc` | `#000000` |
| Surface | `--bg-surface` | `#ffffff` | `#0f0f0f` |
| Elevated | `--bg-elevated` | `#fafafa` | `#141418` |
| Subtle | `--bg-subtle` | `#f3f4f6` | `#181818` |
| Hover | `--bg-hover` | `#eaedf2` | `#212432` |
| Border (hairline) | `--border` | `#e7e4e4` | `#26272e` |
| Border (control, >= 3:1) | `--border-strong` | `#858894` | `#6b728e` |
| Text primary | `--text-primary` | `#0e142b` | `#ffffff` |
| Text secondary | `--text-secondary` | `#2d344d` | `#d9d9d9` |
| Text muted | `--text-muted` | `#555c72` | `#9397a8` |
| Brand fill | `--brand` | `#2e67ff` | `#2b65ff` |
| Brand hover | `--brand-hover` | `#1d54e8` | `#1d54e8` |
| Brand text | `--brand-text` | `#1e54e6` | `#6b96ff` |
| Brand soft | `--brand-soft` | `rgba(46, 103, 255, 0.10)` | `rgba(43, 101, 255, 0.18)` |
| Brand foreground | `--brand-fg` | `#ffffff` | `#ffffff` |
| Accent cyan | `--accent-cyan` | `#007575` | `#00eeba` |
| Focus ring | `--focus-ring` | `0 0 0 3px rgba(46, 106, 255, 0.28)` | `0 0 0 3px rgba(46, 106, 255, 0.36)` |
| Destructive accent | `--destructive` | `#ff6c35` | `#ff6c35` |
| Danger fill / hover / fg | `--danger` / `--danger-hover` / `--danger-fg` | `#c43d0e` / `#9e2f08` / `#ffffff` | `#c83e10` / `#a5310a` / `#ffffff` |
| Status OK (bg / fg / border) | `--ok-bg` / `--ok-fg` / `--ok-border` | `#e6fbf5` / `#046c4e` / `#9be8d2` | `rgba(0, 238, 186, 0.13)` / `#00eeba` / `rgba(0, 238, 186, 0.32)` |
| Status Warn (bg / fg / border) | `--warn-bg` / `--warn-fg` / `--warn-border` | `#fff8e6` / `#8c4a00` / `#f5cf76` | `rgba(249, 255, 64, 0.12)` / `#f4ea55` / `rgba(249, 255, 64, 0.30)` |
| Status Error (bg / fg / border) | `--err-bg` / `--err-fg` / `--err-border` | `#fff0eb` / `#b8370a` / `#ffc4b0` | `rgba(255, 108, 53, 0.16)` / `#ff8559` / `rgba(255, 108, 53, 0.36)` |
| Status Info (bg / fg / border) | `--info-bg` / `--info-fg` / `--info-border` | `#edf3ff` / `#1e54e6` / `#c2d6ff` | `#000e39` / `#7aa2ff` / `#323e62` |
| Chart palette (Light root) | `--chart-1` / `--chart-2` / `--chart-4` | `#2e67ff` / `#00cdcc` / `#00eeba` | Inherits `:root` |

### Odoo Systray & Notifications (`native_print.scss`)

Odoo-side styles are strictly scoped to `.o_native_print_systray` and `.o_notification_manager .o_notification.o_native_print_notification` (never global `.o_dialog` or core `.o_notification` overrides). Both scopes follow Odoo's active theme (Light by default; Dark when inside `.o_dark_mode`, `body.o_web_client_dark`, `[data-color-scheme="dark"]`, `[data-bs-theme="dark"]`, or `.np-theme-dark` — never forced dark from OS `prefers-color-scheme` on a Light Odoo page).

| Role | SCSS Variable | Light (`np-light-tokens`) | Dark (`np-dark-tokens`) |
| --- | --- | --- | --- |
| Popover glass surface | `--np-surface` | `rgba(255, 255, 255, 0.90)` | `rgba(15, 15, 15, 0.90)` |
| Card / Subtle | `--np-card` / `--np-subtle` | `#fafafa` / `#f3f4f6` | `#0f0f0f` / `#181818` |
| Border / Strong | `--np-border` / `--np-border-strong` | `#e7e4e4` / `#858894` | `#26272e` / `#323e62` |
| Text (primary / secondary / muted) | `--np-text` / `--np-text-secondary` / `--np-text-muted` | `#0e142b` / `#2d344d` / `#555c72` | `#ffffff` / `#d9d9d9` / `#9397a8` |
| Brand (text / fill / hover / fg) | `--np-brand` / `--np-brand-fill` / `--np-brand-hover` / `--np-brand-fg` | `#1e54e6` / `#2e67ff` / `#1d54e8` / `#ffffff` | `#6b96ff` / `#2b65ff` / `#1d54e8` / `#ffffff` |
| Brand soft / shadow | `--np-brand-soft` / `--np-brand-shadow` | `rgba(46, 103, 255, 0.10)` / `0 6px 16px -6px rgba(46, 103, 255, 0.45)` | `rgba(43, 101, 255, 0.18)` / `0 6px 16px -6px rgba(46, 103, 255, 0.45)` |
| Glass highlight / shadow | `--np-glass-highlight` / `--np-shadow` | `inset 0 1px 0 rgba(255, 255, 255, 0.90)` / `0 20px 48px rgba(14, 20, 43, 0.14), 0 4px 12px rgba(14, 20, 43, 0.06)` | `inset 0 1px 0 rgba(255, 255, 255, 0.09)` / `0 22px 54px rgba(0, 0, 0, 0.80)` |
| Status OK (fg / bg / border / ring) | `--np-ok` / `--np-ok-bg` / `--np-ok-border` / `--np-ok-ring` | `#046c4e` / `#e6fbf5` / `#9be8d2` / `rgba(4, 108, 78, 0.22)` | `#00eeba` / `rgba(0, 238, 186, 0.14)` / `rgba(0, 238, 186, 0.34)` / `rgba(0, 238, 186, 0.28)` |
| Status Error (fg / bg / border) | `--np-err` / `--np-err-bg` / `--np-err-border` | `#b8370a` / `#fff0eb` / `#ffc4b0` | `#ff8559` / `rgba(255, 108, 53, 0.16)` / `rgba(255, 108, 53, 0.36)` |
| Info (bg / border) | `--np-info-bg` / `--np-info-border` | `#edf3ff` / `#c2d6ff` | `#000e39` / `#323e62` |

## 2. Glass Surfaces & Elevation

| Token | Light | Dark | Usage |
| --- | --- | --- | --- |
| `--glass-bg` | `rgba(250, 250, 250, 0.78)` | `rgba(15, 15, 15, 0.78)` | Translucent sidebars, toolbars, cards |
| `--glass-bg-strong` | `rgba(255, 255, 255, 0.86)` | `rgba(15, 15, 15, 0.90)` | Elevated popovers & overlays (`--np-surface` uses `0.90` alpha) |
| `--glass-highlight` | `inset 0 1px 0 rgba(255, 255, 255, 0.85)` | `inset 0 1px 0 rgba(255, 255, 255, 0.09)` | Top specular rim (Odoo light uses `0.90` alpha) |
| `--glass-blur` | `blur(22px) saturate(180%)` | `blur(22px) saturate(180%)` | Applied via `backdrop-filter` and `-webkit-backdrop-filter` |
| `--shadow-sm` | `0 1px 2px rgba(14, 20, 43, 0.04), 0 0 0 1px rgba(14, 20, 43, 0.02)` | `0 1px 3px rgba(0, 0, 0, 0.50)` | Subtle cards |
| `--shadow-md` | `0 8px 24px rgba(14, 20, 43, 0.07), 0 1px 3px rgba(14, 20, 43, 0.04)` | `0 10px 28px rgba(0, 0, 0, 0.65)` | Floating panels |
| `--shadow-lg` | `0 20px 48px rgba(14, 20, 43, 0.12), 0 4px 12px rgba(14, 20, 43, 0.05)` | `0 22px 54px rgba(0, 0, 0, 0.80)` | Popovers & modals |

## 3. Concentric Radii & Spacing

### Spacing Scale (`theme.css`)
`--space-1: 4px`, `--space-2: 8px`, `--space-3: 12px`, `--space-4: 16px`, `--space-5: 20px`, `--space-6: 24px`, `--space-8: 32px`.

### Radius Ladder (Concentric Geometry)
- `--radius-xs: 6px` — keyboard badges (`.kbd`), micro-chips
- `--radius-sm: 10px` — inputs, printer rows, brand tile (`36×36`), secondary buttons
- `--radius-md: 14px` — standard cards (Odoo popover inner boxes & stat cards use `12px` = `20px` outer − `8px` concentric inset)
- `--radius-lg: 18px` — primary panels
- `--radius-xl: 21px` — top-level sheets/containers (Odoo `.o_native_print_dropdown` uses `20px`)
- `--radius-pill: 999px` — status pills (`.np-mode-pill`, `.np-inuse-pill`)

## 4. Typography Ladder & RTL Rules

- **Sans stack (`--font-sans`):** `-apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", "Thmanyah Sans", "Open Sans", system-ui, sans-serif`
- **Arabic RTL (`[dir="rtl"] body`):** `"Thmanyah Sans", -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif` (weights 300, 400, 500, 700, 900 loaded via `@font-face` with `font-display: swap`), `line-height: var(--leading-relaxed)` (`1.65`), and `letter-spacing: 0` on headings, titles, and buttons so Arabic ligatures never break.
- **Monospace (`--font-mono`):** `"SF Mono", Menlo, "Cascadia Code", ui-monospace, monospace`
- **Type scale:**
  - `--text-xs: 11.5px`
  - `--text-sm: 13px`
  - `--text-base: 14.5px` (body default)
  - `--text-md: 16px`
  - `--text-lg: 20px`
  - `--text-xl: 26px`
- **Line heights:** `--leading-tight: 1.22`, `--leading-body: 1.48`, `--leading-relaxed: 1.65`.
- **Numeric alignment:** `.tabular-nums` (`font-variant-numeric: tabular-nums`) on job counts, queue stats, and version numbers.

## 5. Motion, Reduced Transparency & WCAG 2.2 AA

- **Motion tokens:** `--dur-fast: 140ms` (Odoo: `--np-dur-fast: 140ms`), `--dur-med: 240ms`, `--ease-out` & `--ease-spring` (Odoo: `--np-ease-out`): `cubic-bezier(0.22, 1, 0.36, 1)`.
- **`prefers-reduced-motion: reduce`:** Forces `animation-duration: 0.01ms`, `animation-iteration-count: 1`, `transition-duration: 0.01ms`, and `scroll-behavior: auto` globally in `theme.css`.
- **`prefers-reduced-transparency: reduce`:**
  - Desktop app swaps `--glass-bg` / `--glass-bg-strong` to solid `#fafafa` / `#ffffff` (Light) and `#0f0f0f` (Dark), and sets `--glass-blur: none`.
  - Odoo disables `backdrop-filter` / `-webkit-backdrop-filter` and falls back to `var(--np-card)` on `.o_native_print_dropdown` and `.o_notification_manager .o_notification.o_native_print_notification`.
- **WCAG 2.2 AA Contrast & Focus:**
  - All typography tokens (`--text-primary`, `--text-secondary`, `--text-muted`) and semantic status foreground/background pairs meet **>= 4.5:1** contrast (WCAG 1.4.3).
  - Control borders (`--border-strong`) meet **>= 3:1** non-text contrast against surface backgrounds (WCAG 1.4.11).
  - Keyboard focus indicators (`:focus-visible`) render a `2px solid` outline with `2px` offset (`--brand-text` in desktop, `--np-brand` + `--np-focus-ring` in Odoo).
