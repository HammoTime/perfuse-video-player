# Perfuse

Perfuse is a native Windows x64 client written in Rust and egui. Plex, Jellyfin and Audiobookshelf share the `MediaSource` interface, library UI and in-process libmpv/FFmpeg player.

## UI changes

Open the [rendered prototype](docs/design/multi-server/index.html) before editing a screen it covers. Exercise the relevant navigation, focus, menus and responsive states. Compare a [native snapshot](src/ui_tests.rs) at the same state and width, including the minimum window size. Follow the [handoff](docs/design/multi-server/handoff/HANDOFF.md) for layout and copy. Later product decisions take precedence, including shared volume controls in both audiobook players.

## Playback and credentials

- Keep playback embedded. The worker owns libmpv. On shutdown, flush progress and join the worker before destroying the Win32 video surface.
- Preserve the 300-second disk-backed read-ahead. Skip the optional cache integration test in routine checks. The user handles practical playback, live UI and real-server sign-in validation.
- Store remembered tokens in Windows Credential Manager. Keep passwords and challenge secrets transient. Send tokens only in headers and keep secrets out of logs, URLs, arguments and files. Preserve TLS verification, same-origin checks, redirect rejection and bounded decoding.
- Require user consent and verify release digests before installing or updating the managed playback runtime.

## Build and verify

- Build with `.\tools\build-windows.ps1` and deliver `target\windows\perfuse.exe`. Close the app cleanly before replacing it.
- For code changes, run the relevant formatting, Clippy, test and snapshot checks in [CONTRIBUTING.md](CONTRIBUTING.md#run-the-checks). Documentation-only edits need no app tests.
- Before pushing a release tag, update `Cargo.toml`, `Cargo.lock` and the version-bearing `login-libraries` snapshot. Use a lightweight tag.

## Tool use

- Discover tools by exact name. Request focused ranges and fields, filter large results before returning them, and retrieve relevant truncated content. Keep routine output near 2,000 tokens. Save bulky evidence locally.
- Inspect browser state after failures. Keep long-task handoffs and delegated findings brief and credential-free.
- Design MCP tools around search and execution with on-demand schemas and server-side filtering. Authenticate, authorize and validate every underlying operation.
