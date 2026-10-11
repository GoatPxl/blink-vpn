# Design handoff — GoatPxl direction D ("C made easy")

Status: **proposed**. Nothing in this folder is wired into the build yet. This document is the
spec for porting the D-row artboards from the Claude Design canvas *GoatPxl VPN Redesign* into
`src/`. Renders of every screen are in the owner's `screenshotsv1/design-D*.png`; the canvas holds
the editable source.

Everything below respects `AGENTS.md`. Where the design needs a backend change, §6 says exactly
what and why — none of it touches the provisioning contract (§2), the privilege model (§4), or
provider neutrality (§3).

---

## 1. Principles the UI must keep

1. **One primary action per screen**, and it is always the next step in the journey. Connect and
   Power off never share a row with equal weight.
2. **Hide the plumbing, keep the receipt.** Cipher, handshake, routes live under Shield. Cost is
   visible on Home at all times — it is the product's argument.
3. **Never wait silently.** The 2-minute build gets the game; the 15-second handshake gets the
   HOLD ON screen; everything that happens while the window is hidden is a native notification
   with one action.
4. **Honest verbs.** A different city is a *new server*, so the UI says **Move**, not "switch".
   Power off *deletes* at the provider; the copy says so.

## 2. Tokens and type

`tokens.css` beside this file is the full set. Migration from the current
`src/app/globals.css`:

| Today (`globals.css`) | Becomes | Note |
|---|---|---|
| `--surface-container-lowest` #0b0e15 | `--ink` #0b0b0a | warm black, not blue-black |
| `--surface` / `--surface-container*` | `--panel`, `--panel-2`, `--ink-2` | three fills, not five |
| `--outline-variant` rgba | `--line` #24241f, **2px solid** | borders are opaque and visible |
| `--on-surface` | `--bone` #f2f0e6 | |
| `--on-surface-variant` | `--stone` #a29f92 | 4.6:1 on ink — passes |
| `--primary` #abc7ff (blue) | `--lime` #d4ff3a | |
| `--secondary` #4edea3 (green) | *removed* | lime carries "secure" |
| `--tertiary` #ffb95f | `--ember` #ff8a3d | in-progress only |
| `--danger` #ff6b6b | `--alert` #ff4d6d | |
| `--shadow-elevated` soft blur | `--shadow-hard*` hard offsets | no blur anywhere |
| `rounded-*` utilities | **none** — radius is 0 | search `rounded-` and delete |

**Fonts.** Three Google Fonts, all OFL: Silkscreen 400/700, Space Grotesk 300–700 (variable),
JetBrains Mono 100–800 (variable). Bundle them under `public/fonts/` and load with `@font-face`
in `globals.css`; **do not** load from Google at runtime — the app must render identically with
the kill switch holding the default route and no DNS.

**Silkscreen rule.** Display only: headlines ≥ 32px, the 12px uppercase tags, 10px nav labels,
and button labels. Never body copy, never below 10px. Body is Space Grotesk; anything a user
might copy (IP, id, price, timer) is JetBrains Mono.

**Icons.** Pixel glyphs on a 10×10 grid, drawn as inline `<svg shape-rendering="crispEdges">`
with `<rect>`s, `fill="currentColor"`. The set in the artboards: home, map (globe), shield, setup
(sliders), power, phone, tray. Add new ones on the same grid; no icon fonts, no emoji.

## 3. Shell

- Titlebar 36px, custom (Tauri `decorations: false`), label `GOATPXL VPN` in Silkscreen 12px
  stone; min/max/close buttons 46×36. Close = hide to tray when the setting is on (it is on by
  default; `settings-view.tsx` already has the toggle).
- Left rail 96px, 2px right border. Logo (40×48 goat) then four items:
  `HOME · MAP · SHIELD · SETUP`. Active item: lime fill, ink glyph. The rail is `flex-wrap`-safe
  and stacks above content under 560px, but the window minimum is 1360×840 so this rarely matters.
- Content area: `padding: 28px 36px 36px`, panels in a column with 20px gap. The Home hero panel
  is `flex: 1` so it fills the window at any size.
- **Rail colour follows state**: logo and active item are lime when secure, ember while
  building/holding, stone-2 when off, alert on failure. Same four colours drive the tray icon.

`AppView` in `src/components/dashboard/shared.tsx` becomes `"home" | "map" | "shield" | "setup"`.
`VIEW_ROUTE` maps them to `/dashboard`, `/dashboard/map`, `/dashboard/shield`,
`/dashboard/settings`. The `nodes`, `health` and `activity` routes redirect into `map` and
`shield` respectively so old links keep working.

## 4. Screens

Each entry: artboard → file it replaces → what it shows → commands it calls.

### D1 Home · secure → `dashboard-overview.tsx` (connected branch)
- Hero panel, lime border: tag `■ SECURE SINCE 21:40`; `SECURE` display-xl lime; city line
  28px with a `change ↓` link → Map with the Move sheet pre-opened; **Disconnect** 200×200 square,
  alert outline and hard alert shadow, top right.
- Bottom strip of three: **This session** (mono 36px, from `TunnelPhase` secure-since),
  **Server cost so far** (mono 36px, see §6.2) with `· $0.006/hr` beside it, **Checks** `4 / 4
  passing →` linking to Shield.
- Two quiet cards below: phones (`listNodePeers`, names + spare-slot count, "Show codes" →
  Shield › Phones or the existing QR dialog) and **Done for the day?** with a ghost Power off.
- Nothing else. Throughput, cipher, handshake, QR images all leave this screen.

### D2 Home · off → `dashboard-overview.tsx` (idle branch) + `dashboard-empty-state.tsx`
- `OFF` display-xl stone-2; subtitle "Last time: San Francisco, US · 2 h 14 m · $0.01".
- One 300×200 lime **CONNECT** button labelled with the last location, `~2 min · $4/mo`.
  It runs the full chain: provision → auto-connect (§6.3).
- Row of four tiles: three recent/favourite locations (code, city, `visited 3× · $4/mo`) and a
  dashed **Cheapest near me** tile. Each tile = Move sheet with that region preselected.
- Tray tip card at the bottom.
- First run (no provider credential) does not show this screen; it shows D8.

### D3 Map → new `map-view.tsx`, replaces `nodes-view.tsx`
- Header `MAP`, filter chips `All · Cheapest · Visited · Favourites · List view`. List view is
  the current `nodes-view.tsx` fleet table, restyled; it is not a separate route.
- Dot-matrix world: a fixed 72×36 land mask rendered as a CSS grid of 3px-gapped squares
  (`--panel-2` land, `--ink-2` sea). Ship the mask as a static string; no tiles, no network.
  Max width 960px, centred, pins positioned in % of the mask box.
- Pins: lime with a lime ring = your running node; ember = favourite; bone = any region from a
  provider with a **verified** credential; stone-2 = region from a provider with no credential
  (click → "Add a key for Amazon EC2" → Setup). Pin data comes from `listProviderRegions` per
  verified provider, plus lat/long (§6.1).
- Below: **Quick build** (same as last time · cheapest near me · favourite) and **Selected**
  panel (cheapest size, "also on" other providers, last visit) with **Move here** / **Build
  here** (label depends on whether a node is running) and ☆ Favourite.

### D4 Move sheet → new `move-sheet.tsx` (modal over Map or Home)
- Title `SAN FRANCISCO → FRANKFURT`, price block, "ready in ~2 min".
- Left: three numbered steps with estimates — Build (~2:00), Connect (~0:15, "one Windows
  prompt"), Power off old (~0:05). Right: size radio (cheapest preselected, one step up offered)
  and an opt-in checkbox **Keep San Francisco running too (doubles the bill)**, default off.
- Footer note "Phones will need to scan Frankfurt's new codes." Primary **Move to Frankfurt →**.
- When no node is running the sheet is titled `BUILD FRANKFURT` and step 3 is omitted.

### D5 Build → `connect-progress.tsx` provisioning branch, replaces the corner toast
- Status strip (ember border): `BUILDING FRANKFURT` display-m, one line "Creating the droplet at
  DigitalOcean · you still have internet · connects by itself when ready", mono timer
  `0:47 of ~2:00`, 40-block progress bar. **The strip is always visible above the game.**
- Game panel: `GOAT INVADERS` with score / hi / lives, 48×24 playfield, `← → SPACE`, `ESC
  hides`. Footer: "Optional. Runs locally, no network. The build keeps going if you close this."
  with Hide game / Cancel build. Spec for the game itself: §6.5.
- Provisioning steps and the live log move to Shield › Log; they are not on this screen.

### D6 Hold On · compact → `connect-progress.tsx` connect branch (repeat)
- Full-panel ember state: tag, `HOLD ON` 160px, mono countdown 72px, one sentence "Internet
  pauses for ~15 s while the tunnel shakes hands. Don't disconnect.", 30-block bar, link "Why
  does this happen? →".
- Driven by `getTunnelPhase` (service up, inbound octets 0). Flips to D1 the moment inbound
  octets > 0 (AGENTS.md §4). No game here — ever.
- **First-time variant** (C7 on the canvas): same panel plus the "What's happening" step list
  and the "If it takes longer than 45 s" card with a Roll back button that enables at 0:45. Show
  C7 until the user has completed one successful connect, then D6.

### D7 Shield → new `shield-view.tsx`, replaces `health-view.tsx` + `activity-view.tsx`
- Header `SHIELD`, right-aligned `4 / 4 PASSING` with a lime square.
- Tabs (real `role="tablist"`): **Status · Checks · Tunnel config · Log**. Log tab label shows
  the failed count (`Log · 2 failed`).
- Status: four tiles (Kill switch ARMED, Exit IP, Handshake, Cipher) from `getHealthPanelData`,
  then throughput bars (60 s, lime down / bone up) and the checks list with Run again / Copy bug
  report.
- Checks = current Connectivity Check cards. Tunnel config = the rendered `.conf` with Copy.
  Log = current Activity history with its filters, plus Report a problem and Where things are.
- **Phones** section at the bottom of Status: the device slots and QR dialog from
  `device-slots.tsx`, unchanged in behaviour.

### D8 First run → `onboarding-wizard.tsx`
- Step indicator `✓ 2 3 · key → build → connect` — three steps, not four. Step 1 (WireGuard
  readiness) is auto-checked and only becomes a screen if it fails.
- Step 2: six tiles — DigitalOcean (`easiest`), Linode, Amazon EC2 (`advanced`), Google
  (`advanced`), OpenStack (`experimental`), and a dashed **No cloud yet** tile linking to
  DigitalOcean signup. Then a numbered mini-guide: **Open token page ↗** (deep link per provider,
  §6.6), "tick Droplets · read & write", **Show me (20 s)** link, token field with a **Paste**
  button (reads clipboard once, on click, never automatically). Validation via
  `validateProviderToken` on blur/paste; the tile turns lime when verified.
- Step 3 panel on the right fills in as soon as a key verifies: suggested region code 52px lime,
  city, "Nearest to your time zone. Cheapest size." (§6.4), price, **Build it & connect →**,
  "Pick another city" → Map. Clicking Build goes straight to D5.

### D9 Toasts, tray, notifications → `dashboard-shell.tsx` toast + new `tray.rs`
- **In-app toast** (bottom-right, 520px, bone border, hard lime shadow, 14px padding): icon,
  title 15px/600, body 12.5px stone, optional action button 36px. Variants shown: power-off undo
  (alert shadow, 6-segment countdown bar, `Undo · 4`), "Frankfurt is ready — connecting now",
  "Key for Linode stopped working" with **Fix key**.
- **Power off = one click + 6-second undo.** No typed confirmation. `destroyVpnNode` is called
  when the toast expires; Undo cancels before the call. Replaces the Danger Zone card and C10.
- **Tray menu** two variants (secure / off), items in §6.7. Double-click = connect to last.
- **Windows notifications** (only while the window is hidden, one per event, one action):
  tunnel dropped (Reconnect / Disconnect), no reply for 2 min (Reconnect / Ignore), server ready
  (none), provider key stopped working (Fix key), node on for 12 h (Power off / Keep it). Never
  for routine success.

## 5. Components

| Component | Spec |
|---|---|
| Panel | `--panel` fill, 2px `--line` border, 0 radius, padding 18–28px. State panels tint: ember border + `--ember-ink` fill; alert border + `--alert-ink`; lime border on the secure hero. |
| Button, primary | Silkscreen 14px uppercase, 48px high, lime fill, ink text, `--shadow-hard`. One per screen. |
| Button, ghost | 48px, 2px bone border, bone text, transparent. |
| Button, quiet | ghost with `--line-3` border and stone text. For Cancel, Re-check, "···". |
| Button, destructive | ghost with alert border/text; the big square version adds `--shadow-hard-alert`. |
| Giant action | 200×200 (Disconnect) or 300×200 (Connect) square, icon over label, sub-label Silkscreen 11px at 75%. |
| Tag | Silkscreen 12px uppercase stone, `■ ` prefix when it names a state, coloured to match. |
| Tile | panel 16–20px padding, column: code (Silkscreen 22–26px lime), name (15px 600), meta (12.5px stone). Hover border lime. Dashed border = "add" affordance. |
| Row | 10–14px vertical padding, 2px `--line-2` bottom border, last row borderless. |
| Input | 44–52px, `--ink` fill, 2px `--line-3` border, mono 14–15px, focus border lime. Label = Tag. |
| Toggle | 56×28, 2px border, square knob 24px. On = lime fill/ink knob. |
| Progress | CSS grid of N equal blocks, 3–4px gap, 14–26px tall: done = state colour, current = `*-dim`, pending = `*-track`. Never a thin bar. |
| Tabs | 48px, Silkscreen 13px, bottom-borderless, selected = panel fill + lime border/text. |
| Dialog | panel, bone border, `--shadow-hard-lg`, max-width 680–760, header / body / footer separated by 2px lines. Backdrop: the page at 28% opacity + grayscale(.4). |
| Dot map | see D3. |
| Chart | bars only (lime/bone pairs), 2px baseline. No lines, no gradients, no axes beyond min/max labels in mono 11.5px. |

Accessibility as drawn: all controls are real `<button>`/`<a>`/`<input>`; icon-only buttons
carry `aria-label`; state colours always pair with a word (`SECURE`, `ARMED`, `FAILED`), never
colour alone; stone on ink is 4.6:1; stone-2 is used only ≥ 20px or for non-essential text.

## 6. Backend and plumbing the design needs

None of these change the provisioning contract. Each is small and provider-neutral.

### 6.1 Region geography gains lat/long
`ProviderRegion` gets `latitude: Option<f32>, longitude: Option<f32>`. Fill them in each
adapter's `*_regions.rs` table (they are curated already; add two columns). Unknown slugs stay
`None` and are listed but not pinned. `listProviderRegions` already returns the struct, so the
TS type in `tauri.ts` gains the two optional fields in the same change (AGENTS.md §8).

### 6.2 Cost so far
`created_at` is on `vpn_nodes`; hourly price is in the stored plan snapshot. Cost so far =
`(now − created_at) × hourly`, computed in the frontend from `listVpnNodes` and ticked every
second. Label it "server cost so far" and keep the "estimate — your provider's invoice is the
truth" footnote in Shield › Status, not on Home.

### 6.3 Auto-connect after provision
New orchestrator step, behind a setting `auto_connect_after_provision` (default **on**, toggle in
Setup › Tunnel defaults): when `createVpnNode` returns ready, call `connectTunnel` for it. This
is one UAC prompt, same as today; the only thing removed is the click in between. The C8
pre-prompt shows before the first ever connect; afterwards only if the user un-ticks "don't
explain this again".

### 6.4 Suggested first region
Pure frontend: map `Intl.DateTimeFormat().resolvedOptions().timeZone` to a continent and pick
the verified provider's cheapest region on it (fallback: the provider's first US region). No
network call, no IP geolocation. Size = cheapest in the catalogue.

### 6.5 Goat Invaders
A single React component, ~300 lines, canvas-less (DOM grid, 48×24, 2px gap, or a `<canvas>` if
perf needs it). Local-only: no network, high score in `app_settings`. Keyboard: arrows/space,
Esc hides. Pauses when the window loses focus. It mounts only inside the D5 provisioning branch
and unmounts when the orchestrator reaches "ready" — the status strip and progress bar are
rendered *outside* it so a crash in the game cannot hide the build state.

### 6.6 Provider deep links and scope hints
Add to each adapter's credential-field declaration (already used by the onboarding form):
`token_page_url`, `scope_hint` ("Droplets: read & write"), and `guide_gif` (bundled asset). DO
and Linode first; AWS/GCE say "advanced" and link to the IAM/service-account page.

### 6.7 Tray
`tauri-plugin-tray` (or the built-in `tray-icon` feature in Tauri 2). Four icon states (lime /
ember / stone-2 / alert, 16px pixel goat). Menu, secure: `Secure · <city> · <timer>` (disabled),
Disconnect, Change location ▸ (recent 3 + "Open map…"), `Power off server ($x so far)`, Open
GoatPxl, Quit (keeps tunnel up). Menu, off: `Off · nothing billing`, `Connect · <last>`,
Connect elsewhere ▸, Open GoatPxl, Quit. Double-click = Connect · last. All actions route
through the same commands the window uses; the tray never calls the provider directly.

### 6.8 Notifications
`tauri-plugin-notification`. Emit only when the main window is hidden. Events and actions as in
D9. Each notification carries a `kind` from the existing structured error model (AGENTS.md §7) so
the action button maps to a `next_action`, not to message text.

### 6.9 Move orchestration
New command `moveVpnNode { from_node_id, region, size, keep_old: bool }` that sequences existing
commands: `createVpnNode` → wait ready → `connectTunnel` (new) → wait `TunnelPhase` secure →
`disconnectTunnel` (old) is unnecessary because the new service replaced it → `destroyVpnNode`
(old) unless `keep_old`. If any step fails before the new tunnel is secure, the old node is left
untouched and the new one is destroyed, and the user sees C12 with `kind = move.failed`.

### 6.10 Undo-able power off
Frontend only: `destroyVpnNode` is scheduled 6 s after the click; the toast shows the countdown
and an Undo that clears the timer. If the window closes during the countdown the destroy still
fires (the timer lives in the shell, not the view).

## 7. Order of work

1. Tokens + fonts + shell (rail, titlebar, four views routing, tray-colour state). Everything
   else restyles inside this.
2. Home (D1/D2) — the hero, cost ticker (§6.2), giant buttons.
3. Shield (D7) — fold Health + Activity into tabs; nothing new, just moved.
4. Build + Hold On (D5/D6/C7) — replace the corner toast with the full panels; auto-connect
   (§6.3). Game last within this step; ship without it if needed.
5. Map + Move (D3/D4) — lat/long (§6.1), dot mask, Move command (§6.9).
6. Onboarding (D8) — deep links (§6.6), suggested region (§6.4).
7. Tray + notifications + undo toast (D9, §6.7, §6.8, §6.10).

Verify each step at 1360×840 and maximized, against a release build (`npm run tauri build`),
because the console-window and tray behaviours differ in dev (AGENTS.md §4).
