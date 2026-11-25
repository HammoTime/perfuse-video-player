# Contributing to Perfuse

Perfuse is a Rust desktop app for Windows x64 with an egui interface and in-process libmpv playback.

## Build on Windows

Install Rust 1.88 or newer and the Visual Studio C++ build tools for the standard MSVC target. Run the build script from the repository root. It also supports the project-local GNU toolchain under `.tools` when available. [Install Rust](https://www.rust-lang.org/tools/install)

```powershell
.\tools\build-windows.ps1
.\tools\build-windows.ps1 -RunTests
.\tools\build-windows.ps1 -DebugBuild
```

The script copies the executable and license to `target\windows`. Close Perfuse before replacing a running executable. To build and launch directly with Cargo, use `cargo run --locked --release`.

The Windows installer contains the Perfuse executable and licence. On first launch, Perfuse shows the licence links and asks for acceptance even if the playback libraries are already installed. **Decline and exit** closes Perfuse. Acceptance is stored per Windows user in `%LOCALAPPDATA%\Perfuse\playback-license.accepted`. Any missing playback libraries are downloaded only after an explicit confirmation.

With permission, Perfuse downloads the Windows x64 libmpv and graphics packages, verifies their release SHA-256 digests and extracts the DLLs with Windows' built-in `tar.exe` (Windows 10 version 1803 or newer). It keeps the libraries and their source and license notices in `%LOCALAPPDATA%\Perfuse\Runtime`. [Upstream Windows builds](https://github.com/shinchiro/mpv-winbuild-cmake/releases)

For offline playback, place `libmpv-2.dll` and its companion files beside the executable. Set `PERFUSE_LIBMPV` to the absolute DLL path to use a development build and disable managed playback library installation and updates. [mpv installation guide](https://mpv.io/installation/)

**Settings → 3rd party libraries** lists the installed components and their versions. Use **Check for Updates** to see if a new runtime is available, then review the licence links and confirm any download. Perfuse keeps the current runtime until it has verified and activated the new one.

## Run the checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Ordinary tests use synthetic fixtures for Plex requests and the app UI. The UI harness runs the eframe update loop without opening a window or reading saved credentials.

### Screenshot baselines

On Windows, run the offscreen snapshot suites with DirectX 12:

```powershell
$env:WGPU_BACKEND = 'dx12'
cargo test --locked ui_tests::login_snapshots -- --ignored
cargo test --locked ui_tests::library_snapshots -- --ignored
```

The suites cover login, download permission, settings and library screens across window sizes and display scales. View the rendered images in `.artifacts/ui/` and inspect any `.new.png` or `.diff.png` files in `tests/snapshots/`. After reviewing an intentional change, set `UPDATE_SNAPSHOTS=true`, rerun the affected suites and unset the variable. Commit the reviewed PNGs. Windows CI compares those baselines and uploads the rendered images on success and failure.

### Local Plex fixture

```sh
python3 tools/demo_server.py
```

In **Manual server connection**, connect to `http://127.0.0.1:32499` with the token `demo`. The fixture provides synthetic movies, TV episodes, artwork, a generated audio file and in-memory playback progress. With the fixture running and libmpv available, the optional `cargo test --locked embedded_playback_and_cache -- --ignored` checks in-process playback and buffering.

For a development connection to your own server, `PLEX_URL` and `PLEX_TOKEN` override the remembered values or prefill the manual form. Use an HTTP(S) server origin with no path prefix. Perfuse stores remembered Plex credentials in Windows Credential Manager. Use **Sign out & forget this device** to remove the saved credentials.

The screenshot tests exercise egui layout and interaction. Use a Windows app build for testing native video output, hardware decoding and actual Plex sign-in.

### Media source plugins

The Perfuse library exposes `media::MediaSource` and shared types for libraries, items, versions and playback progress. The compiled-in Plex plugin in `src/plugins/plex/` translates Plex responses into those types and supplies authenticated direct-play requests. Plugin registration lives in `src/plugins/mod.rs`.

## Package a release

Build the release executable first, then use Inno Setup 6.7.3 with the version from `Cargo.toml`. The installer is written to `dist`.

```powershell
ISCC.exe /DPerfuseVersion=0.3.1 installer/perfuse.iss
```

The **Release Perfuse** workflow builds the x64 MSVC executable and publishes the installer to GitHub Releases. Push a supported version tag to release that exact version, or run the workflow on a branch and select **normal**, **alpha** or **beta**. Leave the commit field blank to release the branch tip. Stable releases must point to a commit on `main` or `master`.

Manual runs pick the next patch after the manifest version or the highest stable tag. Alpha and beta sequences are independent. Stable clients receive stable updates. Beta clients also receive betas, and alpha clients receive both preview families. The app checks public GitHub releases and verifies the installer digest before running an update. [Release workflow](.github/workflows/release.yml)

## Licensing

Perfuse is licensed under the GNU Affero General Public License version 3 only. Copyright © 2026 Perfuse contributors. [Perfuse license](LICENSE)

The managed playback libraries have their own terms:

- libmpv uses GPL in the upstream Windows build. [mpv copyright and licensing](https://github.com/mpv-player/mpv/blob/master/Copyright)
- The upstream FFmpeg build enables GPL components. [FFmpeg licensing](https://ffmpeg.org/legal.html)
- libass uses the ISC licence. [libass licence](https://github.com/libass/libass/blob/master/COPYING)
- libplacebo uses the LGPL. [libplacebo licence](https://github.com/haasn/libplacebo/blob/master/LICENSE)
- subrandr uses MPL 2.0. [subrandr licence](https://github.com/afishhh/subrandr/blob/master/LICENSE)
- The Direct3D shader compiler is a Microsoft component. [Microsoft Windows SDK terms](https://github.com/microsoft/win32metadata/blob/main/licenses/sdk_license.txt)

See the upstream build recipes for the additional dependencies compiled into the runtime. [Build recipes and dependency list](https://github.com/shinchiro/mpv-winbuild-cmake)
