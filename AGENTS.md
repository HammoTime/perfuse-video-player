# Perfuse repository guide

## Where the UI design lives

The checked-in multi-server prototype is the design reference for first run, Settings → Servers and the add-server prompts. Read the relevant screen in the prototype and handoff before changing the native UI.

- [Prototype entry point](docs/design/multi-server/index.html) and [handoff](docs/design/multi-server/handoff/HANDOFF.md). Sections 9 and 10 cover first run and Settings → Servers.
- [Prompt markup and transitions](docs/design/multi-server/js/app.js). Look for `flowHTML`, `flowHead`, `kindOption` and `openAddDialog` for the exact copy, order of controls, navigation and focus.
- [Visual specifications](docs/design/multi-server/css/app.css). The “Add-server flow (onboarding + dialog)” section defines card sizes, spacing, fields, the modal and responsive layouts. The glyphs used in the native app are in [assets/icons](assets/icons).
- The shared flow starts with a source chooser and opens one provider's form at a time. The approved implementation scope is Plex and Jellyfin. The prototype also illustrates Audiobookshelf for future work.
- [Native implementation](src/connection.rs). `App::onboarding` hosts the flow in a 720-point card with the wordmark. `App::add_server_dialog` hosts it in a 640-point modal with a close button. Both call `App::connection_fields`. The source tiles stack at viewport widths of 1000 points and below. The modal measures each step against the viewport so taller forms can grow and shorter windows can scroll.
- [Offscreen checks](src/ui_tests.rs). `ui_tests::login_snapshots` renders both entry points. Compare `login-desktop` and `login-jellyfin` for first run, and `add-server-types`, `add-server-plex-manual` and `add-server-jellyfin` for Settings. The ordinary tests also exercise keyboard focus, Back, Escape, form bounds and completion.

### Hover panels, confirmations and buttons

- Use the connection-status hover panel as the visual reference for hover panels, menus and confirmation pop-ups. `popover_frame()` supplies the charcoal panel, a 1-point divider border, 14-point corners, 16-point padding and the soft shadow. `App::new` applies the same defaults to egui tooltips and menus. Keep the larger add-server dialog dimensions from the prototype. The status behavior and Home library list share `HoverPopover` in `src/servers.rs` for delayed hover, focus, click-to-pin and dismissal.
- Give confirmations a clear title, a wrapping explanation in muted text, and a distinct footer. Server removal uses a neutral Cancel and a red Remove. Cancel, Escape and clicking outside dismiss the dialog.
- Adjacent action buttons must share width and height, including when one is disabled. Use `Pill::pair` to size both to the wider label and taller control. Groups of library toggles use a common measured size and wrap as a group. Check the actual control bounds at the minimum window size.
- Sidebar server headings keep the 24-point expand and sort controls 4 points apart and reserve space for the fully expanded floating scrollbar. Home server cards reserve a 28-point library row even before libraries load. Keep all chips that fit on one line. When three or more would wrap, show two chips and an expand button that opens the server's ordered list of visible libraries.

## Product and user decisions

- Perfuse is a small, open-source **native Windows desktop client**, written in Rust, for Plex and Jellyfin movie and TV libraries.
- Deliver a runnable native Windows `.exe`.
- Keep the feature set and architecture simple. Both compiled-in source plugins share the Rust media interface, native library UI and embedded player.
- Playback must stay **inside Perfuse**, using native libmpv/FFmpeg. Do not restore the previous external `mpv.exe` process, separate player window, or JSON IPC implementation.
- The user requested **at least five minutes of read-ahead**. The current configuration targets 300 seconds with a disk-backed packet cache. Preserve it unless explicitly changing caching behavior.
- The user explicitly asked to stop spending time testing the five-minute cache and leave practical playback/cache evaluation to them. Do not run the optional cache integration check or add cache benchmarks by default. Run focused checks when needed for an actual reported fault.
- The user can inspect the UI. Do not make routine progress depend on UI automation or repeated confirmation requests.
- Broad codec coverage is a goal. HDR, Dolby Vision, Atmos/DTS:X passthrough, and real-server/device compatibility still need platform-specific evaluation.

## Source map

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | eframe/egui native app, library navigation, search, artwork textures, details/version picker, embedded video layout and controls |
| `src/connection.rs` | Shared source chooser, Plex and Jellyfin forms, validation, sign-in workers, Plex server selection and connection confirmation |
| `src/ui_tests.rs` | Headless egui_kittest tests of the real app loop: login interaction, bounds and offscreen image comparisons |
| `src/lib.rs`, `src/media.rs` | Shared media source interface and library, item, version and playback types |
| `src/plugins/mod.rs`, `src/http.rs` | Source registry, typed server credentials, origin validation, redirect policy and bounded HTTP decoding |
| `src/plugins/plex/mod.rs` | Plex client implementation, API response mapping, pagination, artwork, URL and header validation, and playback timeline updates |
| `src/plugins/plex/auth.rs` | Plex browser PIN sign-in, cancellable polling and validated account server discovery |
| `src/plugins/jellyfin/mod.rs` | Jellyfin views, item queries and mapping, image references, direct file selection and playback reports |
| `src/plugins/jellyfin/auth.rs`, `src/plugins/jellyfin/tests.rs` | Password and Quick Connect exchanges and synthetic HTTP protocol tests |
| `src/servers.rs` | Source-aware server groups, aggregate Home, connection checks and server management |
| `src/session.rs` | Windows Credential Manager session storage, restore and removal; no plaintext token files |
| `src/player.rs` | Playback worker, controls/events, libmpv configuration, buffering, resume, progress reporting and shutdown |
| `src/icons.rs` | Bundled `assets/` SVG icons (embedded in the exe, `currentColor` rendered white then tinted); rasterised app window icon and the large taskbar icon from the exe resource |
| `assets/` | Icon, brand and button-state SVGs; `preview.html` maps each icon to its UI use. `Pill` and `IconButton` in `src/main.rs` implement the `assets/buttons` state sheets |
| `src/cache.rs` | Persistent artwork cache (`%LOCALAPPDATA%\Perfuse\Artwork`): encoded server bytes keyed by an FNV hash of server origin + artwork path (no token), trimmed oldest-first above 512 MiB at startup. Cached art is sent before network fetches; new art shows a pulsing placeholder then fades in over 1s |
| `src/prefs.rs` | Non-sensitive UI preferences (`%LOCALAPPDATA%\Perfuse\preferences.json`): per-server sidebar library sort mode and custom order keyed by an FNV hash of server origin (no tokens or raw URLs); skipped under `cfg!(test)` |
| `build.rs` | Rasterises `assets/brand` into a multi-size .ico and embeds it as resource 1 via `embed-resource` (windres for GNU, rc.exe for MSVC) on Windows targets |
| `src/mpv.rs` | Small dynamically loaded binding to libmpv's stable C client API, using `libloading` |
| `src/runtime.rs` | Background Windows x64 libmpv/FFmpeg installation, actual component versions, manual update checks, SHA256 verification, selective extraction and versioned per-user activation |
| `src/video.rs` | Win32 child video surface inside the app window |
| `tools/build-windows.ps1` | Native Windows build, optional ordinary tests, delivery-folder copy and local toolchain setup |
| `tools/demo_server.py` | Python standard-library synthetic Plex fixture: movies, TV hierarchy, pagination, generated artwork/audio, in-memory progress |
| `.github/workflows/ci.yml` | Formatting, strict Clippy, ordinary tests and offscreen snapshots on Windows |
| `README.md` | Public feature overview, screenshots and installer link |
| `CONTRIBUTING.md` | Build, tests, release steps and playback library licences |

Rust edition is 2024; the declared minimum Rust version is 1.88. The UI uses eframe 0.33 with Glow and AccessKit. Native playback uses a separately packaged DLL loaded in the same process. There is no web frontend or JavaScript build step.

## Tracing a source from sign-in to playback

- **Registration.** `plugins::BUILTIN` feeds the source picker, plugin ID lookup and Windows credential enumeration. Each plugin supplies its kind, ID, name, default origin and `open` function. `ServerKind::plugin` dispatches a saved session to the same implementation. The shared `MediaSource` interface is the boundary used by navigation and playback.
- **Connect.** Choosing a source starts a fresh credential draft. Plex offers browser sign-in and a manual address/token disclosure. Browser discovery in `plugins::plex::auth` feeds a checklist of account servers. Jellyfin accepts an address and username, an optional password, or Quick Connect with the address alone. `App::connect` signs in on a worker, opens the plugin with `session::open` and fetches its libraries. `Reply::Connected` adds the verified server and shows its libraries. Back and Close invalidate pending replies. Done returns to Settings, and Continue to Perfuse opens Home after first run.
- **Jellyfin authentication.** A password, including an empty one for a passwordless account, goes in the JSON body of `POST /Users/AuthenticateByName`. Quick Connect starts at `POST /QuickConnect/Initiate`. The UI displays the six-digit code, and the worker checks `GET /QuickConnect/Connect` every five seconds using the one-time challenge secret. Approval exchanges that secret at `POST /Users/AuthenticateWithQuickConnect`. Both methods yield a user access token. Subsequent API and media requests use a sensitive `Authorization: MediaBrowser …` header with the same device ID. User-scoped endpoints resolve the user from that token.
- **Restore and select.** A saved session carries `kind`, `server`, `token` and `client_id`. The serde default for a missing `kind` is Plex. Windows enumerates `Perfuse/Plex/<origin>` and `Perfuse/Jellyfin/<origin>` and also reads the older `Perfuse/PlexSession` blob. `App::restore` reconnects each through its own plugin. `Server::check` refreshes libraries and Continue Watching. `App::activate` binds navigation to one source and discards the previous source's page, detail, artwork and pending results. IDs are local to that source. Jellyfin preferences have a distinct hash input to preserve existing Plex library ordering and visibility.
- **Browse.** `App::load_page` dispatches `Browse` requests. Jellyfin lists the user's movie and TV views through `/UserViews`, searches and pages `/Items` with `parentId`, `searchTerm`, `startIndex` and `limit`, and follows series and season IDs for children. Server Home uses creation date for Recently Added, `/UserItems/Resume` for Continue Watching and premiere date for Recently Released. Home item IDs are matched to libraries in batches for visibility filtering. Last-played timestamps are converted to Unix seconds for ordering alongside Plex.
- **Play.** Opening a playable Jellyfin item fetches `/Items/{id}` for its versions, track metadata and user data. The selected media source ID goes to `/Videos/{id}/stream?static=true&mediaSourceId=…`. The plugin constructs this URL on the connected origin and passes the auth header separately to libmpv. Artwork references contain a validated item ID, image type and image tag. Both artwork and playback use locally constructed routes. Multi-part titles use the shared `Stream::Multipart` rejection path.
- **Verify.** Run `cargo test --locked --lib plugins::jellyfin` for the loopback protocol fixtures. `cargo test --locked` also exercises legacy session decoding, provider switching, form privacy and Quick Connect cancellation in the real egui app loop. The Windows `ui_tests::login_snapshots` suite includes the Jellyfin form.

For endpoint or authentication changes, consult the upstream server controllers and SDK guide. [Jellyfin API controllers](https://github.com/jellyfin/jellyfin/tree/v10.11.0/Jellyfin.Api/Controllers) · [Jellyfin authentication guide](https://kotlin-sdk.jellyfin.org/guide/authentication.html)

## Build and Windows delivery

Run from the repository root in PowerShell:

```powershell
.\tools\build-windows.ps1
# Include ordinary tests when appropriate:
.\tools\build-windows.ps1 -RunTests
# Faster development build:
.\tools\build-windows.ps1 -DebugBuild
```

- The runnable artifact is **`target\windows\perfuse.exe`**. The build output before packaging is `target\release\perfuse.exe`, or `target\debug\perfuse.exe` for a development build.
- Keep the accompanying runtime files beside the delivered executable when bundling libmpv. The build script copies the Perfuse licence to `LICENSE.txt`.
- `tools/build-windows.ps1` copies `.tools\libmpv\libmpv-2.dll` when available. Windows first launch shows licence links and saves acceptance per user in `%LOCALAPPDATA%\Perfuse\playback-license.accepted`. Declining exits. An explicit download confirmation is required before installing or updating the shinchiro portable libmpv/FFmpeg runtime into `%LOCALAPPDATA%\Perfuse\Runtime`. Perfuse verifies the GitHub release asset SHA256 digests, extracts the playback and graphics DLLs with Windows `tar.exe`, retains licence and source information and validates the C API before publishing the main DLL last. Existing bundled or installed runtimes are reused. `PERFUSE_LIBMPV` takes precedence and disables managed installation and updates.
- `mpv.exe`, `mpv.com`, `MPV_PATH`, named-pipe IPC, and the old external-player launch settings are obsolete. Neither executable is needed for playback.
- Settings' **3rd party libraries** section reads the runtime's libmpv, FFmpeg, libass, libplacebo and optional subrandr versions, plus the companion Direct3D compiler's file version. Check for Updates refreshes inventory and queries upstream; Update is disabled until a different/newer release is found. Upgrades stage and validate the checked release under `%LOCALAPPDATA%\Perfuse\Runtime\<archive SHA256>`, then atomically replace `current.txt`. Resolution is explicit `PERFUSE_LIBMPV`, activated version, bundled DLL, then legacy per-user DLL. Updates do not replace loaded/bundled DLLs. Old versions remain for other running instances; failed downloads never activate a partial runtime. An explicit override disables managed updates.
- Close the project app cleanly before replacing its executable or loaded DLL. Preserve final playback progress through normal shutdown. Do not indiscriminately terminate other media players or unrelated processes.
- The script first uses an installed Windows `cargo.exe`. Standard MSVC Rust requires the Visual Studio C++ build tools.
- This workspace also has an isolated GNU Rust toolchain under `.tools\cargo` and `.tools\rustup`, plus a portable `llvm-mingw-*` compiler. These are local, ignored artifacts, not guaranteed in a fresh checkout. No machine-wide PATH change was made.
- The local GNU/Clang combination needs **`-C link-self-contained=yes`** for Rust's matching MinGW runtime libraries, including host build scripts. Without it, linking fails on `-lgcc`/`-lgcc_eh`. Copy the environment setup from the build script for ad-hoc native Cargo commands; setting PATH alone is insufficient.
- Windows PowerShell 5 can treat redirected native stderr as error records. The build script converts output to strings and checks `$LASTEXITCODE`; preserve that behavior and do not classify normal Cargo stderr progress as a build failure.
- Release tags pushed to trigger `.github/workflows/release.yml` **must be lightweight Git tags** (`git tag vX.Y.Z`, not annotated `git tag -a`). The `publish` job verifies the tag commit via GitHub's `repos/{owner}/{repo}/commits/tags/$TAG` endpoint, which returns HTTP 422 for annotated tag objects. Also bump `Cargo.toml` / `Cargo.lock` and regenerate `tests/snapshots/login-libraries.png` (which displays `Perfuse <version>` in About) before tagging so snapshot CI matches the release version.

The bundled development library is from shinchiro's `20261111` x86_64 development package, mpv revision `bcde6b93cc`. Its archive SHA256 is `273c079b11ea5575822f972756f4c9a71149a5c10130353abeb8586738938452`. Prefer authoritative upstream packages and verify checksums when replacing binaries. Perfuse's application source is AGPL-3.0-only. The playback libraries retain their own licences. [Upstream release](https://github.com/shinchiro/mpv-winbuild-cmake/releases/tag/20261111)

## Playback invariants

- `Player::start` creates a worker that owns the libmpv handle and all calls into it. Controls travel to that worker over Rust channels; playback state travels back to the UI.
- FFI symbols must match `mpv/client.h`'s C ABI. Resolve symbols before allocating the handle, keep the DLL loaded until after `mpv_terminate_destroy`, and free returned strings with `mpv_free`. Maintain explicit safety comments around unsafe code.
- `video::Surface` is created, positioned and destroyed on the UI thread. It uses the eframe HWND as its parent, DPI-scaled egui bounds, and libmpv `wid` embedding with `vo=gpu-next`, `gpu-api=d3d11`.
- Video fills the available app area at all times. Controls use transparent egui Areas in an owned transparent immediate viewport above the native video child, without reserving video space. Keep the root viewport's `with_transparent(false)` and `App::clear_color` opaque black: an opaque clear color alone does not prevent native video child pixels from revealing the desktop through a transparent root window. The default glutin config requests 8 alpha bits even with native root transparency disabled; only the controls viewport enables Windows/DWM transparency. eframe clears immediate viewports separately to transparent RGBA. `Surface::prepare_overlay` validates the overlay process, assigns the root window as owner, tracks the video bounds, and mirrors the parent window's always-on-top (`WS_EX_TOPMOST`) state; do not make it globally always-on-top on its own or turn video into a separate window.
- eframe 0.33/glutin-winit can discard a secondary window's transparency request when the Windows GL driver reports no support (upstream egui issue #7543/fix #8423). `enable_overlay_alpha` restores winit's DWM alpha-composition setup explicitly before assigning ownership/showing the overlay. Preserve this workaround until the dependency contains the upstream fix; alpha-zero painting alone does not fix the resulting black window. Check the native DWM call as well as headless egui paint/layout tests.
- Fullscreen controls fade out over 375ms after two seconds idle and fade in over 300ms after one second of sustained movement (movement gaps over 350ms reset the streak). Keyboard/button/wheel input reveals them immediately. Native cursor sampling covers movement over libmpv's child window. Clicking the video can give libmpv's child window keyboard focus; `Surface::reclaim_keyboard` returns it to the app each frame so Space/arrow/F11 shortcuts keep working. Windowed controls stay visible. Headless app-loop checks cover layout, transparency and opacity; the hidden Win32 ownership test covers overlay bounds/ownership, not actual GL/DWM composition, which remains for the user to inspect.
- **Stop/join the player before destroying the video surface.** Current field order and completion handling preserve this lifetime relationship. App exit must finish the worker and final progress update.
- User playback requires the embedded surface. `Player::start(None, ...)` selects null audio/video outputs for headless integration checks; it is not a product playback fallback.
- Preserve hardware decoding (`auto-safe`), TLS verification and disabled user mpv configuration. There are no inherited external player scripts or user settings.
- `CACHE_SECONDS` in `src/player.rs` is the duration setting. Current cache configuration is `cache=yes`, `cache-secs=300`, `demuxer-readahead-secs=300`, `cache-on-disk=yes`, `demuxer-max-bytes=128MiB`, `demuxer-max-back-bytes=16MiB`, and `demuxer-hysteresis-secs=0`.
- With disk caching, the byte caps apply to packet metadata rather than encoded payload. Reverting to a small RAM packet cap can truncate UHD read-ahead to roughly 20 seconds. Temporary cache files can grow during viewing and are deleted at playback close.
- Start playback when data is ready and fill the cache in the background. The UI displays actual `demuxer-cache-duration`; network speed and the remaining file duration can limit it. Do not imply five minutes is instantly available.
- Plex uses `viewOffset` and its timeline endpoint. Jellyfin uses `UserData.PlaybackPositionTicks` (10,000 ticks per millisecond). The player reports a successful file load once, the current playing or paused position every ten seconds, and the final position on shutdown. Jellyfin posts these to `/Sessions/Playing`, `/Sessions/Playing/Progress` and `/Sessions/Playing/Stopped`. Plex retains its periodic and stop timeline updates. Each server applies its own watched-state policy.

## Security, state and current scope

- Remember me defaults on for Windows. Successful connections save the session in the current user's Credential Manager with local-computer persistence. Removing a server deletes its saved credential. Disabling Remember me removes the stored entry while keeping the live session. Passwords and Quick Connect secrets stay transient. `PLEX_URL` and `PLEX_TOKEN` apply only to Plex restore and its manual form. `PERFUSE_LIBMPV` optionally points to a development DLL.
- Never log tokens, passwords, Quick Connect secrets, authenticated URLs, raw request/response bodies, or libmpv diagnostic strings that may contain credentials. Keep authenticated clients and credential types free of `Debug` implementations.
- Send the token in an HTTP header and pass it directly to the in-process library. Do not put it into URLs, command-line arguments, config files or temporary files.
- Keep same-origin media/artwork validation, redirect rejection, identifier validation, TLS checks and bounded JSON/image decoding. Servers currently must be HTTP(S) origins without credentials, query strings or reverse-proxy path prefixes.
- Existing features include multiple connected Plex and Jellyfin servers, aggregate and per-server Home, movie and TV libraries, paginated title search, shows/seasons/episodes, artwork/backdrops, synopsis/version selection, direct playback, resume/progress, buffered seeking, speed/chapter controls and grouped settings. The native UI uses black and charcoal panels with blue accents. The user explicitly wants to validate UI changes themselves. Leave live visual and real-server sign-in validation to them.
- Not implemented: saved playback preferences, watchlists, collections, account profile switching, external Plex subtitle fetching, transcoding fallback, offline downloads, automatic next episode, multipart playback, disc/menu navigation or DRM. Multipart media is rejected explicitly rather than silently playing only part one. Do not add fake data or inactive buttons for unsupported parts of the design.

## Validation and evidence

For an installed/configured Rust toolchain, use the checks appropriate to the change:

```text
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

- Ordinary tests cover timestamp bounds and Plex and Jellyfin HTTP/model/trust-boundary behavior. They require neither real credentials nor libmpv.
- Authentication tests cover encoded browser URLs, server-resource trust boundaries and saved-session validation. The ignored `credential_manager_roundtrip` test uses a uniquely named synthetic credential and removes it; run it explicitly only when validating native credential storage. The sandbox cannot write Credential Manager, so that check needs a normal interactive Windows context. Do not use a real account for automated sign-in checks.
- The user requested reliable UI checks without desktop control. `src/ui_tests.rs` runs the real eframe update loop headlessly with synthetic fixtures and skips credential restore. Normal tests check login bounds, keyboard entry, password semantics, scrolling, cancel/error handling, download consent and server selection. Follow the Windows snapshot commands in `CONTRIBUTING.md` with `WGPU_BACKEND=dx12`. When forwarding Cargo arguments through a PowerShell script, quote the literal `'--'` before `--ignored` or `-D warnings`. CI runs the snapshot suites on `windows-2022` and uploads `.artifacts/ui` plus diffs. Reviewed baselines in `tests/snapshots` are intentional tracked test fixtures. Do not update them automatically in CI or substitute isolated component tests for the actual app layout. These tests do not verify native video embedding or real authentication.
- `tools/demo_server.py` serves synthetic Plex data on loopback port 32499. Its dummy credential is documented in CONTRIBUTING.md. It provides a 480-second audio fixture for the optional long-buffer check. Progress exists only in server memory.
- The ignored `embedded_playback_and_cache` test requires the fixture and native libmpv. For development, set `PERFUSE_LIBMPV` to the full DLL path. Do not run it routinely given the user's cache-testing preference above.
- Before that preference was expressed, native Windows formatting, strict Clippy, three ordinary tests, and the optional in-process buffering/resume/progress/shutdown check passed. The updated native release was built. Embedded GPU output and practical real-media/cache evaluation remain with the user; do not describe them as independently verified.
- Local embedded playback evidence is in `.artifacts\embedded-build.log` and `.artifacts\embedded-validation.log`.
- `target`, `.tools`, `.artifacts`, `.env` and logs are ignored. Do not commit generated binaries, downloaded toolchains, cache data or credentials. Keep durable instructions in this file and README rather than relying on temporary paths, process IDs or old tool sessions.
- Documentation-only edits do not need application tests. Once relevant checks pass, continue toward delivery instead of repeating broad validation without a new reason.

## Working context

- Discover tools by exact namespace/name. List matching names first; load only selected definitions. Do not search whole descriptions with broad fragments or print a complete tool catalog.
- Request focused file ranges, search results and JSON fields. Process large datasets inside tools and return the relevant findings, counts, provenance, errors and completeness limits. Save bulky evidence as an artifact and cite its path.
- Aim for about 2,000 tokens per routine tool result; use more when needed for correctness. Do not repeatedly print unchanged documentation, logs, schemas or page snapshots. This budget does not restrict requested final deliverables.
- Follow browser tool documentation and use its supported incremental or targeted observations. After a failed operation, inspect the relevant state and correct the cause. Stop repeating an unchanged failing action; report a concrete blocker after a bounded recovery attempt.
- Treat truncated results as incomplete. Retrieve relevant omitted portions before drawing conclusions; preserve source data and complete copyable deliverables when requested.
- Read skill/reference material only when applicable. Read required guidance fully, in bounded sections when needed, and retain a brief note of what has already been read.
- During long work, keep a concise local handoff with decisions, changed files, validation and next steps. Reuse it after compaction instead of re-running broad discovery. Keep credentials out of summaries and output.
- For requested delegated work, give each agent a bounded task and ask for concise findings, evidence paths and blockers rather than full transcripts.

## MCP design preference

- Never design an MCP interface as a 1:1 mirror of API operations. Use a compact search-and-execute interface, following Cloudflare's Code Mode pattern: discover schemas on demand, compose operations and filter intermediate results on the server, and return only the relevant output. Keep the initial tool descriptions small; do not move the full API catalogue into a single tool description. Enforce authentication, authorisation and validation for every underlying operation.
