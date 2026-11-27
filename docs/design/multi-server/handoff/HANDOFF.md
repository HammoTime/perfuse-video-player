# Perfuse: multiple servers per instance (design handoff)

This is the design spec for turning Perfuse into a client for several servers at once: Plex, Jellyfin and Audiobookshelf. The prototype is the reference for behaviour. This document covers measurements, copy, states and a suggested data model, so you can build it in the eframe/egui app (`src/main.rs`) without guessing.

> The target is still the **native egui app**. The HTML prototype is only a reference. CSS values below are given next to their `Color32` / egui equivalents.

---

## 1. Files

| Path | What it is |
| --- | --- |
| `index.html` | Clickable prototype entry (open in any browser, 1440×900 recommended; minimum 860×620 like `MIN_BROWSE_SIZE`) |
| `preview.html` | Offline interactive shelf preview with simulated artwork loading |
| `css/app.css` | All tokens (`:root`) and component styles |
| `js/app.js` | Prototype behaviour: navigation, status aggregation, audiobook player, add-server flow, settings |
| `js/data.js` | Sample servers, videos, audiobooks with chapters (mirrors the model in §9) |
| `js/shelves.js` | Responsive shelf sizing, paging and the sample artwork reveal |
| `js/icons.js` | Every glyph used, existing and new, with `isNew` flags |
| `handoff/icons/*.svg` | **24 new icons** in the exact `assets/icons` format, ready to drop in |
| `handoff/icons/preview.html` | Contact sheet for the new icons and where each is used |
| `handoff/tools/export-icons.js` | Regenerates `handoff/icons` from `js/icons.js` (`node handoff/tools/export-icons.js`) |
| `fonts/Ubuntu-Light.ttf` | egui's default font, copied from `epaint_default_fonts` so the prototype matches the app |

### Viewing states in the prototype

Open [the shelf preview](../preview.html) to browse a larger fictional library offline. **Replay loading** on Home deliberately delays the second video, *Beyond the Blue*, while the other visible covers arrive. Resize the window, page away and back, or open a title and return to the same shelf position.

| Keys | State |
| --- | --- |
| `Ctrl+Shift+1` | All 3 servers connected (green) |
| `Ctrl+Shift+2` | Jellyfin "Media" unreachable: **2 (of 3) servers connected** (yellow) |
| `Ctrl+Shift+3` | Nothing reachable: **Cannot connect to servers** (red) |
| `Ctrl+Shift+0` | First run: no servers (landing / onboarding) |
| `?state=2` etc. | Same as above, set from the URL |

The add-server flow simulates outcomes:
- An address containing `offline` returns "Couldn't reach …".
- The password or token `wrong` returns an authentication error.
- Anything else succeeds.
- Re-adding a seed address (for example `http://media.lan:8096`) restores that server's sample content.

---

## 2. Tokens

Existing constants are unchanged. Add only the ones marked **NEW**.

| Token | Value | Rust |
| --- | --- | --- |
| Title bar / root panel | `#000000` | `visuals.panel_fill = BLACK` |
| Sidebar fill | `#141417` | `(20,20,23)` (existing sidebar frame) |
| Main fill | `#0D0D10` | `(13,13,16)` |
| Panel | `#1C1C1E` | `PANEL` |
| Raised / art placeholder | `#222428` | `(34,36,40)` |
| Control / hover / pressed | `#2C2C30` / `#3E3E43` / `#1F3A5C` | `CONTROL`, `CONTROL_HOVER`, `CONTROL_PRESSED` |
| Sidebar selected | `#172A40` | `(23,42,64)` |
| Divider | `#2A2A2E` | **NEW** `DIVIDER` |
| Text / muted | `#EBEBF5` / `#98989F` | `TEXT`, `MUTED` |
| Accent | `#3D9BFF` | `ACCENT` |
| Status green | `#7EE2A0` | `GREEN` |
| Status yellow | `#F2C94C` | **NEW** `WARN` `(242,201,76)`, 11.9:1 on sidebar |
| Status red | `#FF8A80` | **NEW** `STATUS_ERROR` `(255,138,128)`, ≈5.3:1 on sidebar. Keep `ERROR_TEXT` for form errors. |
| Progress track / buffer | gray 65 / gray 110 | existing seek bar values |

Type uses the egui default (Ubuntu Light). Sizes: caption 12, small 13, body 15 (`TextStyle::Body`), lead 16, h3 18, h2 22, h1 30, display 36.
Spacing is on a 4 px grid: 4 / 8 / 12 / 16 / 20 / 24 / 28 / 32 / 40 / 48.
Browse cards, navigation, controls, popovers and panels have square corners. Keep the existing Windows-style pin, minimize, maximize and close buttons in the top-right title bar. Status dots and circular control indicators retain their geometry.

---

## 3. Navigation model

```
Home (all servers)                         ← new
├── Server Home  (per server)              ← today's Home, scoped to one server
│   └── Library  (per server library)      ← today's library grid
│       ├── Video details                  ← unchanged
│       └── Audiobook details              ← new
│           └── Player (full)              ← new; docked bar everywhere else
└── Servers & settings
    └── Servers block                      ← redesigned
First run (no servers) → landing           ← redesigned
```

- Title-bar **Back** keeps working as a history stack and restores scroll position.
- A view model of `View::{Home, Server(id), Library(id, lib), Video(id), Book(id), Player, Settings}` is enough. Every content reference carries its `ServerId`.

---

## 4. Sidebar (244 px, unchanged width)

```
 ┌──────────────────────────────┐
 │ [⌂] Home                     │  nav item 42h, icon 18, text inset 40 (SIDEBAR_TEXT_INSET)
 │                              │
 │ [▶] BASEMENT        [≡] [⌄]  │  server group header 32h: kind icon 16 + NAME (12 px, UPPER, +0.06em, MUTED)
 │   [▦] Movies                 │  libraries as today
 │   [▭] TV Shows               │
 │   [▭] Lectures               │
 │                              │  24 px gap between groups
 │ [△] MEDIA      [▲!]  [≡] [⌄] │  non-online: status glyph after the name (yellow/red)
 │   Offline · last seen 4 min  │  12 px MUTED note, inset 42
 │   [▦] Films        (dimmed)  │  libraries stay clickable → offline empty state
 │ …                            │
 │──────── (scrolls) ───────────│
 │ ● 3 servers connected        │  status button (see §5)
 │ [⚙] Servers & settings       │
 └──────────────────────────────┘
```

- **The label is an icon, not text.** "Plex – Basement" is shown as the Plex glyph plus "BASEMENT". The glyph has an accessible name ("Plex"), and the header has a tooltip: `Plex – Basement`. Long names truncate with an ellipsis, and the full name is in the tooltip and on Server Home.
- Clicking the header opens **Server Home**. The selected state uses the same fill as a selected nav item (`#172A40`), and the glyph turns ACCENT.
- `[≡]` is the existing per-server sort toggle (24 px circle; pressed = `CONTROL_PRESSED` with an `#8CC8FF` glyph). It is still keyed per server in `prefs.rs`.
- `[⌄]` collapses or expands the group (chevron rotates −90° when collapsed, 150 ms). Store the collapsed state in prefs, keyed by server.
- Group order follows the order in Settings → Servers.
- The groups scroll; Home and the footer stay fixed.

---

## 5. Connection status (sidebar footer)

### Aggregation rules

| Condition | Text | Colour | Glyph |
| --- | --- | --- | --- |
| all N online | `N servers connected` (`1 server connected`) | text MUTED, glyph GREEN | `status-connected` (filled dot) |
| some online | `X (of N) servers connected` | WARN (text and glyph), tinted row bg (WARN at 9%) | `status-partial` (filled triangle) |
| none online | `Cannot connect to servers` | STATUS_ERROR, tinted row bg | `status-offline` (ring with slash) |
| any check in flight | `Checking servers…` | MUTED | `refresh` |

The glyph shape changes along with the colour, so status is never conveyed by colour alone.

### Hover popover (340 px wide, panel fill, 16 padding, opens 8 px above the button)

- It opens on **hover** after 250 ms and on **keyboard focus**. Clicking pins it open. It closes on mouse-leave (unless pinned), Escape or an outside click.
- Header: the title (`All servers connected` / `1 server unreachable` / `No servers reachable`) and a one-line summary.
- One row per server: a 32 px kind tile, the name, and the state line. The state line reads `Connected · 12 ms` or `Can't reach server · last seen 4 min ago` (in red), and offline rows get a **Retry** button.
- Footer: `Check again` (re-checks unreachable servers, or all of them if all are online) · `Manage servers` (opens Settings).
- In egui this is an `egui::Area` anchored to the button rect with `Order::Foreground`. Hover uses `response.hovered()` plus a short delay; keep it open while the pointer is over the area itself.

---

## 6. Home (all servers), new

1. **Page head:** `Home` / `Pick up where you left off, across all your servers.` The refresh button checks all servers.
2. **Servers:** a responsive grid of server cards (min 260 px, 16 gap). Each card has:
   - a 48 px kind tile,
   - the name (18 px) and the kind label (12 px MUTED),
   - the state line with its glyph,
   - library chips (28 h, icon and name).
   Clicking a card opens Server Home. An offline card gets a dark-red tint (`#1A1718` fill, `#3A2526` border) and faint chips.
3. **Continue Watching:** all in-progress video across online servers, as 16∶9 cards. Each card adds a **source line** with the kind glyph (13 px) and server name.
4. **Continue Listening:** in-progress audiobooks with square covers. They show the source line, `author · 2 h 45 min left`, and a `Playing` / `Paused` badge on the current book.
   Each row appears only when it has items. "Watching" and "Listening" are split by content type.
5. If a server is unreachable, its items are removed from the rows. A note under the row explains why: `2 items from Media hidden while offline`, with the WARN glyph.
6. If **no** server is reachable, the rows are replaced by the offline empty state: `Can't reach any of your servers`, with **Try again** and **Server settings** buttons.

Rail arrows sit next to each heading, as in v0.3.1.

Size shelves from the usable content width in logical points. Use a 16-point gap and minimum widths of 240 for video, 176 for books and 160 for posters. Evenly distribute the remaining width up to a comfortable maximum. Every full page uses whole columns. The final page keeps the same card widths. Continue Watching gets one row by default. A tall window can show two when the next populated shelf still fits. Compact server cards on short windows to keep playable items within reach.

Show titles, progress and focus immediately. Reserve the artwork rectangle while its image loads. Newly ready previews fade over 83 ms with a 16 ms stagger between simultaneous arrivals, capped at 48 ms. Each image can finish independently. Previously displayed covers return immediately. Use an instant reveal when Windows animation effects are disabled.

Crossfade the outgoing and incoming content panes over 120 ms when opening another screen or going back. Keep the title bar, sidebar and audiobook dock steady. Scrolling, resizing and same-screen updates keep their existing content visible. Switch screens immediately when animation effects are disabled.

## 7. Server Home (today's Home, scoped)

- A breadcrumb above the title: `[glyph] Plex / Basement`.
- Title: the server name. Subtitle: `Plex server · 3 libraries`. The refresh button re-checks this server.
- Video servers: Continue Watching (this server only) and Recently Added (posters), exactly as today.
- Audiobookshelf: Continue Listening and Recently Added (square covers).
- When the server is offline: `Can't reach Media` / `Jellyfin at http://media.lan:8096 isn't responding. It was last seen 4 min ago.` with **Try again** and **Server settings**.

---

## 8. Audiobooks

### Audiobook details

- **Hero (panel, 28 padding):**
  - 200 px square cover.
  - Eyebrow `AUDIOBOOK` (ACCENT, 12 px, uppercase).
  - Title (36 px).
  - `By **Author** · Narrated by **Narrator**`.
  - Meta line: `Tidewater, book 2 · 2023 · 7 h 21 min · 16 chapters`.
  - Description (max 70 characters per line).
- **Actions:**
  - In progress: `Resume` (primary) and `Play from beginning` (secondary).
  - Not started: `Play`.
  - Finished: `Listen again`.
  - Currently playing: `Pause`, plus `Open player`.
  - Next to the actions, a progress line: `Chapter 9 of 16 · 2 h 45 min left`.
- **Two columns** (stacked below 1180 px):
  - **Chapters panel:** each row is 52 px min. The leading column shows a ✓ (GREEN) for played chapters, `now-playing` (ACCENT) for the current one, and the number for upcoming ones. Then the title (ellipsis), and on the current row a 3 px in-chapter progress bar. Duration `mm:ss` / `h:mm:ss` (tabular figures). The current row uses the selected fill; played titles are MUTED. Clicking any row plays from that chapter.
  - **Bookmarks panel:** a bookmark glyph, the note, `5:52:10 · 9. Brackish`, and a delete ✕. Clicking a bookmark jumps to it and plays. Empty state: `No bookmarks yet. Add one while listening to save your place with a note.`
  - **Details panel:** server, library, narrator, release year, length.

### Docked player (88 px, `#17171A`, top divider)

It shows on every browse screen while a book is loaded, and is hidden on the full player.

```
[cover 56] Title            [|◁] [↺30] (▶) [30↻] [▷|]        [☾ 28:41] [1.25×] [☾] [≣] [⌃] [✕]
           Chapter title    0:12:04 ━━━━━━━━━━━────── −0:21:30
```

- The left block (cover and text) opens the full player.
- The transport shows previous chapter, back 30 s, play/pause, forward 30 s and next chapter. Chapter and skip icons are 20 px in 32 px buttons. Play/pause uses a 25 px icon in a 40 px white circle.
- The time row is **chapter-relative**: elapsed in the chapter, the bar, and the time remaining in the chapter.
- Right side, in order:
  - sleep countdown badge (only while a timer is set),
  - speed chip (pressed style when not 1×),
  - sleep-timer menu,
  - Chapters (opens the full player on the Chapters tab),
  - Open player,
  - Stop.
- Below 1000 px the right cluster hides. Speed and sleep stay available in the full player.

### Full player

- The left column fills the available width. Both timelines span that column, with the transport and tools centered beneath them.
  - collapse button and breadcrumb,
  - 240 px cover, with `CHAPTER 9 OF 16` (eyebrow), the chapter title (18 px ACCENT), the book title (30 px) and the byline,
  - **chapter scrubber:** a slider over the current chapter only, labelled `elapsed` and `−remaining`,
  - **book track:** a thin 4 px whole-book bar with a 2 px **tick at each chapter boundary** (the current chapter's tick is ACCENT), labelled `Book 5:52:10 of 7:21:00` and `1 h 29 min left`. At speeds other than 1×, the right label reads `1 h 11 min left at 1.25×`,
  - transport: 27.5 px icons in 44 px buttons and a 35 px play/pause icon in a 64 px white button,
  - tools row (40 h buttons): `Speed 1.25×`, `Sleep · 28:41`, `Add bookmark`.
- Right column (360 px, sticky, panel): a segmented tab control, `Chapters (16)` / `Bookmarks (1)`, with the lists described above.
- **Previous chapter** jumps to the start of the current chapter if more than 3 s have played; otherwise it goes to the previous chapter.
- **Speed menu:** 0.75, 1 (normal), 1.25, 1.5, 1.75, 2, 2.5, 3×.
- **Sleep menu:** Off, 15, 30, 45 or 60 minutes, End of chapter. End of chapter pauses exactly at the chapter boundary and leaves the position at the next chapter's start.
- **Add bookmark** stores the position. The default note is `<chapter> · <time in chapter>`. Adding one switches to the Bookmarks tab and shows a toast.
- **Keyboard:** all controls are focusable. Menus are `menuitemradio` with ↑/↓ navigation and Escape, which returns focus to the trigger.

Playback itself still goes through `Player` / libmpv. Audio-only needs no video surface: don't create `video::Surface` for audiobooks. Chapter data comes from the Audiobookshelf API, and progress and bookmarks sync back to the server, the same way Plex timeline updates work today.

---

## 9. First run (landing), redesigned

This uses the same centred card language as today's login screen (panel `#1C1C1E`, 44 padding, 720 px max).

1. The Perfuse mark and wordmark (40 px ACCENT).
2. `Make yourself at home.` / `Your films, shows and audiobooks, in original quality. Connect a server to get started. You can add more later.`
3. **Three server-type tiles** (3 columns, 1 column below 1000 px). Each tile has a kind glyph, a name and a one-line description:
   - **Plex:** Movies and TV. Sign in with your Plex account.
   - **Jellyfin:** Movies and TV. Use your server address and Jellyfin account.
   - **Audiobookshelf:** Audiobooks and podcasts, with chapters and bookmarks.
4. **The flow per type** is shared with the Settings dialog (§10):
   - **Plex:**
     1. `Sign in with Plex` (browser PIN flow, as today).
     2. The waiting state, with Cancel and `Open the sign-in page again`.
     3. **Choose Plex servers:** a checklist of the servers the account can see. It shows `Your server` / `Shared with you`, the library count, and `Already added` (disabled).
     4. `Add N servers`.

     There is also a `Connect manually with an address and token` disclosure (address and token).
   - **Jellyfin:**
     - Fields: server address, username and password. The password is optional: `Leave empty if your account has no password.`
     - **Quick Connect** is offered as an alternative. It shows a 6-digit code and waits for approval from a signed-in device.
   - **Audiobookshelf:** server address, username and password (all required).
   - All forms include `Remember me on this computer` (default on; Credential Manager, per server).
5. **Validation:**
   - Fields validate on blur, and on submit.
   - Errors appear under the field, with ERROR_TEXT and a red border.
   - If there are two or more errors, a summary banner links to each field.
   - Rules: the address is required; it must start with `http://` or `https://`; it must not duplicate an existing server (`This server is already added as "Media".`).
   - Connection failures show a banner: `Couldn't reach media.lan:8096`, or `Username or password is incorrect` / `That token was rejected`.
   - The primary button shows a spinner and `Connecting…` while busy.
6. **Success:**
   - `Basement is connected`, with an editable **Name in Perfuse** field (defaults to the server's own name or the first label of the host name).
   - The found libraries: `Found 3 libraries: Movies, TV Shows, Lectures`.
   - **Continue to Perfuse** / **Add another server**. Adding another returns to the type tiles, with a `Connected so far` list and a persistent **Continue to Perfuse** button.

## 10. Settings → Servers, redesigned

- **Section head:** `Servers`, the server count, and an **Add server** primary button. That button opens the same flow in a modal `<dialog>` (640 px); in egui, a modal `Window`. Close (✕), Escape, and Done all return focus to *Add server*.
- **One row per server** (panel). The columns are:
  - drag handle (28 px),
  - kind tile (44 px),
  - **name** (16 px) over `Plex · https://192.168.1.20:32400` (12 px MUTED, ellipsis),
  - state (`Connected · 12 ms` / `Offline` / `Checking…`, with its glyph),
  - actions: `Reconnect` (offline only), Rename ✎, expand ⌄.
- **Reorder:** drag a row, or focus its handle and use ↑/↓. The order drives both the sidebar groups and the Home server grid. Announce `Basement moved to position 2 of 3`.
- **Rename** happens inline: a text field where Enter saves and Escape cancels. Saving on blur is allowed; an empty name keeps the old one.
- **Expanded detail:**
  - Type, Address, Signed in as, Status.
  - **Library visibility switches** (`Libraries shown in the sidebar and on Home`). A hidden library disappears from the sidebar, the rows and Server Home.
  - `Check connection` and `Sign out and remove` (danger button).
- **Remove** asks for confirmation: `Remove Basement?` / `Perfuse will sign out of this Plex server and forget its saved sign-in. Your progress stays on the server.` with Cancel / Remove (danger). Removing the last server returns to the first-run landing.

---

## 11. Suggested model changes

```rust
pub type ServerId = String;            // stable, e.g. FNV hash of kind + origin (no token)

pub enum ServerKind { Plex, Jellyfin, Audiobookshelf }

pub struct ServerConfig {
    pub id: ServerId,
    pub kind: ServerKind,
    pub name: String,                  // user-editable display name
    pub address: String,
    pub account: String,               // display only; secrets live in Credential Manager
    pub order: u32,
}

pub enum ServerStatus { Online { latency_ms: u32 }, Checking, Offline { last_seen: Option<SystemTime> } }

pub enum LibraryKind { Movies, Shows, Audiobooks, Podcasts }

pub struct Chapter { pub index: u32, pub title: String, pub start: f64, pub duration: f64 }
pub struct Bookmark { pub id: String, pub at: f64, pub note: String }

pub struct Audiobook {
    pub key: ItemKey,                  // (ServerId, item id)
    pub title: String, pub author: String, pub narrator: Option<String>,
    pub series: Option<(String, f32)>, pub year: Option<u16>,
    pub chapters: Vec<Chapter>, pub position: f64, pub finished: bool,
    pub bookmarks: Vec<Bookmark>,
}
```

- Every media source plugin (Plex today; Jellyfin and Audiobookshelf next) implements the shared interface in `src/media.rs`. The app holds `Vec<(ServerConfig, Box<dyn MediaSource>, ServerStatus)>`.
- `session.rs`: one Credential Manager entry per `ServerId`.
- `prefs.rs`: add server order, each group's collapsed flag and hidden library IDs, all keyed by `ServerId`. The existing per-server sort keys already use an origin hash.
- `cache.rs` artwork keys already include the server origin, so no change is needed.
- Continue rows merge per-server results sorted by last-viewed time. Fetch servers concurrently, and let an offline server simply contribute nothing; the hidden-items note uses the server's last known count.
- Health check: a lightweight ping per server (Plex `/identity`, Jellyfin `/System/Info/Public`, Audiobookshelf `/ping`). Run it at startup, every 60 s and on Retry. Update the footer from the aggregate.

## 12. New icons (`handoff/icons`, copy into `assets/icons`)

`server-plex`, `server-jellyfin`, `server-audiobookshelf`, `audiobooks`, `podcasts`, `headphones`, `back-30`, `forward-30`, `chapters`, `sleep-timer`, `bookmark`, `bookmark-add`, `player-expand`, `player-collapse`, `now-playing`, `add`, `check`, `chevron-down`, `drag-handle`, `rename`, `quick-connect`, `status-partial`, `status-offline`, `more` (spare overflow glyph, not used in the prototype).

The three server glyphs are **simplified monochrome marks** drawn to the 1.5 px stroke family. They are not the official brand logos. If you want exact marks, swap in each project's official assets and check their trademark guidelines. Register each new file in `src/icons.rs` and add it to `assets/preview.html`.

## 13. Assumptions and open points

- Video playback UI is unchanged. In the prototype, `Resume` on a video only shows a toast.
- Audiobookshelf podcasts are listed as a library type and an icon, but no podcast screens are designed.
- Sample audiobooks, authors and covers are fictional placeholders, drawn in the fixture's art style.
- Latency values, the "last seen" times and the Quick Connect code are illustrative.
- Not designed yet: offline downloads, per-server user switching (Plex Home users) and Jellyfin SyncPlay.
