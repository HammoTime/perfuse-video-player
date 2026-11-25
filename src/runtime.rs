//! Portable playback runtime setup. No administrator access or machine-wide changes.
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};

pub const LICENSE_LINKS: &[(&str, &str)] = &[
    (
        "libmpv (GPL)",
        "https://github.com/mpv-player/mpv/blob/master/Copyright",
    ),
    ("FFmpeg (GPL build)", "https://ffmpeg.org/legal.html"),
    (
        "libass (ISC)",
        "https://github.com/libass/libass/blob/master/COPYING",
    ),
    (
        "libplacebo (LGPL)",
        "https://github.com/haasn/libplacebo/blob/master/LICENSE",
    ),
    (
        "subrandr (MPL-2.0)",
        "https://github.com/afishhh/subrandr/blob/master/LICENSE",
    ),
    (
        "Direct3D shader compiler (Microsoft)",
        "https://github.com/microsoft/win32metadata/blob/main/licenses/sdk_license.txt",
    ),
    (
        "Upstream build and dependencies",
        "https://github.com/shinchiro/mpv-winbuild-cmake",
    ),
];

pub enum Update {
    #[cfg_attr(not(windows), allow(dead_code))]
    Status(String),
    Libraries(Installed),
    #[cfg_attr(not(windows), allow(dead_code))]
    Available(Option<Available>),
    Finished(Result<(), String>),
}

#[derive(Clone)]
pub struct LibraryInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Default)]
pub struct Installed {
    pub libraries: Vec<LibraryInfo>,
    #[cfg(windows)]
    archive_digest: Option<String>,
    #[cfg(windows)]
    mpv_version: String,
    #[cfg(windows)]
    build_date: Option<u32>,
}

#[derive(Clone)]
pub struct Available {
    pub version: String,
    #[cfg(windows)]
    bundle: (windows::Asset, windows::Asset),
}

#[cfg(test)]
pub fn test_available() -> Available {
    #[cfg(windows)]
    {
        windows::test_available()
    }
    #[cfg(not(windows))]
    {
        Available {
            version: "20261111".into(),
        }
    }
}

#[cfg(windows)]
fn user_directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("Perfuse"))
}

#[cfg(windows)]
fn directory() -> Option<PathBuf> {
    user_directory().map(|dir| dir.join("Runtime"))
}

pub fn license_accepted() -> bool {
    #[cfg(windows)]
    {
        user_directory().is_some_and(|dir| windows::license_accepted(&dir))
    }
    #[cfg(not(windows))]
    true
}

pub fn accept_license() -> Result<(), String> {
    #[cfg(windows)]
    {
        let dir = user_directory().ok_or("Windows could not locate your local app data folder.")?;
        windows::accept_license(&dir)
    }
    #[cfg(not(windows))]
    Ok(())
}

pub fn library_path() -> PathBuf {
    // An explicit development override always wins, including when it is invalid.
    if let Some(path) = std::env::var_os("PERFUSE_LIBMPV") {
        return path.into();
    }
    #[cfg(windows)]
    {
        let packaged = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("libmpv-2.dll")));
        let root = directory();
        root.as_deref()
            .and_then(windows::active_library)
            .unwrap_or_else(|| choose_library(packaged, root.map(|dir| dir.join("libmpv-2.dll"))))
    }
    #[cfg(not(windows))]
    PathBuf::from("libmpv.so.2")
}

#[cfg(windows)]
fn choose_library(packaged: Option<PathBuf>, installed: Option<PathBuf>) -> PathBuf {
    packaged
        .filter(|path| path.is_file())
        .or(installed)
        .unwrap_or_else(|| PathBuf::from("libmpv-2.dll"))
}

pub fn needed() -> bool {
    cfg!(windows) && std::env::var_os("PERFUSE_LIBMPV").is_none() && !library_path().is_file()
}

pub fn start(ctx: egui::Context) -> Receiver<Update> {
    spawn(ctx, |emit| {
        #[cfg(windows)]
        {
            let dir = directory().ok_or("Windows could not locate your local app data folder.")?;
            let installed = windows::install(&dir, None, &|message| emit(Update::Status(message)))?;
            emit(Update::Libraries(installed));
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = emit;
            Err("Automatic playback setup requires Windows.".into())
        }
    })
}

pub fn inspect(ctx: egui::Context) -> Receiver<Update> {
    spawn(ctx, |emit| {
        #[cfg(windows)]
        {
            cleanup();
            emit(Update::Libraries(windows::inventory(&library_path())?));
        }
        #[cfg(not(windows))]
        emit(Update::Libraries(Installed::default()));
        Ok(())
    })
}

pub fn check(ctx: egui::Context) -> Receiver<Update> {
    spawn(ctx, move |emit| {
        #[cfg(windows)]
        {
            cleanup();
            let installed = windows::inventory(&library_path())?;
            emit(Update::Libraries(installed.clone()));
            let latest = windows::latest(&windows::client()?)?;
            let available = windows::is_newer(&latest, &installed);
            emit(Update::Available(available.then_some(latest)));
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = emit;
            Err("Runtime updates require Windows.".into())
        }
    })
}

pub fn upgrade(ctx: egui::Context, available: Available) -> Receiver<Update> {
    spawn(ctx, move |emit| {
        if std::env::var_os("PERFUSE_LIBMPV").is_some() {
            return Err("Remove PERFUSE_LIBMPV to manage playback updates in Perfuse.".into());
        }
        #[cfg(windows)]
        {
            let root = directory().ok_or("Windows could not locate your local app data folder.")?;
            let installed = windows::install(&root, Some(&available), &|message| {
                emit(Update::Status(message))
            })?;
            emit(Update::Libraries(installed));
            emit(Update::Available(None));
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (emit, available);
            Err("Runtime updates require Windows.".into())
        }
    })
}

pub fn cleanup() {
    #[cfg(windows)]
    if std::env::var_os("PERFUSE_LIBMPV").is_none()
        && let Some(root) = directory()
        && let Ok(_operation) = windows::exclusive(&root.join(".manage"))
    {
        windows::cleanup(&root);
    }
}

#[cfg(windows)]
pub fn lease(path: &std::path::Path) -> Result<Option<std::fs::File>, String> {
    let Some(root) = directory() else {
        return Ok(None);
    };
    let Some(parent) = path.parent() else {
        return Ok(None);
    };
    if parent == root || parent.parent() == Some(root.as_path()) {
        windows::shared(&parent.join(".lease")).map(Some)
    } else {
        Ok(None)
    }
}

fn spawn(
    ctx: egui::Context,
    task: impl FnOnce(&dyn Fn(Update)) -> Result<(), String> + Send + 'static,
) -> Receiver<Update> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let emit = |update| {
            let _ = tx.send(update);
            ctx.request_repaint();
        };
        emit(Update::Finished(task(&emit)));
    });
    rx
}

#[cfg(windows)]
pub(crate) mod windows {
    use super::{Available, Installed, LibraryInfo};
    use reqwest::{Url, blocking::Client, redirect::Policy};
    use ring::digest::{Context, SHA256};
    use serde::Deserialize;
    use std::{
        fs,
        io::{Read, Write},
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            process::CommandExt,
        },
        path::{Path, PathBuf},
        process::Command,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    const RELEASE: &str =
        "https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest";
    const DOWNLOAD: &str = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/";
    const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
    const MAX_TEXT: u64 = 1024 * 1024;
    const ACCEPTED_LICENSE: &str = "playback-license.accepted";

    pub(super) fn license_accepted(dir: &Path) -> bool {
        dir.join(ACCEPTED_LICENSE).is_file()
    }

    pub(super) fn accept_license(dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir)
            .and_then(|()| {
                fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(dir.join(ACCEPTED_LICENSE))
            })
            .map(|_| ())
            .map_err(|_| {
                "Could not save licence acceptance. Check folder permissions and retry.".into()
            })
    }

    pub(super) fn exclusive(path: &Path) -> std::io::Result<fs::File> {
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path)
    }

    pub(super) fn shared(path: &Path) -> Result<fs::File, String> {
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(path)
            .map_err(|_| "The playback runtime just changed. Please retry.".into())
    }

    pub(super) fn cleanup(root: &Path) {
        let Some(active) = active_library(root) else {
            return;
        };
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if active.parent() != Some(dir.as_path())
                && entry.file_name().to_str().is_some_and(valid_key)
                && entry
                    .metadata()
                    .is_ok_and(|meta| meta.is_dir() && meta.file_attributes() & 0x400 == 0)
            {
                remove_runtime(&dir, false);
            }
        }
        // The first managed installer used unversioned files directly in Runtime.
        if root.join("THIRD-PARTY.txt").is_file() {
            remove_runtime(root, true);
        }
    }

    fn remove_runtime(dir: &Path, legacy: bool) {
        let lease = dir.join(".lease");
        let Ok(unused) = exclusive(&lease) else {
            return;
        };
        match fs::remove_file(dir.join("libmpv-2.dll")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return,
        }
        for name in ["d3dcompiler_43.dll", "LICENSE-mpv.txt", "THIRD-PARTY.txt"] {
            let _ = fs::remove_file(dir.join(name));
        }
        drop(unused);
        if !legacy {
            let _ = fs::remove_file(lease);
            let _ = fs::remove_dir(dir);
        }
    }

    fn valid_key(key: &str) -> bool {
        key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
    }

    pub(super) fn active_library(root: &Path) -> Option<PathBuf> {
        let mut key = String::new();
        fs::File::open(root.join("current.txt"))
            .ok()?
            .take(65)
            .read_to_string(&mut key)
            .ok()?;
        if !valid_key(&key) {
            return None;
        }
        let path = root.join(key).join("libmpv-2.dll");
        path.is_file().then_some(path)
    }

    pub(super) fn bundle_key(available: &Available) -> Result<String, String> {
        available.bundle.0.validate(DOWNLOAD)?;
        Ok(available.bundle.0.digest.as_deref().unwrap_or_default()[7..].to_owned())
    }

    pub(super) fn activate(root: &Path, key: &str) -> Result<(), String> {
        if !valid_key(key) || !root.join(key).join("libmpv-2.dll").is_file() {
            return Err("The playback update is incomplete.".into());
        }
        let pending = root.join(format!("current-{}.tmp", std::process::id()));
        fs::write(&pending, key)
            .and_then(|()| fs::rename(&pending, root.join("current.txt")))
            .map_err(|_| {
                "Could not activate the playback update. The previous runtime is still selected."
                    .into()
            })
    }

    pub(super) fn inventory(path: &Path) -> Result<Installed, String> {
        if !path.is_file() {
            return Ok(Installed::default());
        }
        let mpv = crate::mpv::Mpv::from_path(path)?;
        for (name, value) in [
            ("config", "no"),
            ("load-scripts", "no"),
            ("terminal", "no"),
            ("vo", "null"),
            ("ao", "null"),
        ] {
            mpv.option(name, value)?;
        }
        mpv.initialize()?;
        let mut installed = Installed::default();
        for (name, property) in [
            ("libmpv", "mpv-version"),
            ("FFmpeg", "ffmpeg-version"),
            ("libass", "libass-version"),
            ("libplacebo", "libplacebo-version"),
            ("subrandr", "subrandr-version"),
        ] {
            if let Some(mut version) = mpv.get(property).filter(|v| !v.is_empty()) {
                if property == "mpv-version" {
                    installed.mpv_version = version.clone();
                }
                if property == "libass-version"
                    && let Ok(value) = version.parse::<u32>()
                {
                    version = format!(
                        "{}.{:x}.{:x}",
                        value >> 28,
                        (value >> 20) & 0xff,
                        (value >> 12) & 0xff
                    );
                }
                installed.libraries.push(LibraryInfo {
                    name: name.into(),
                    version,
                });
            }
        }
        if let Some(parent) = path.parent() {
            let graphics = parent.join("d3dcompiler_43.dll");
            if graphics.is_file() {
                installed.libraries.push(LibraryInfo {
                    name: "Direct3D shader compiler".into(),
                    version: file_version(&graphics)
                        .unwrap_or_else(|| "Version unavailable".into()),
                });
            }
            if let Ok(file) = fs::File::open(parent.join("THIRD-PARTY.txt")) {
                let mut provenance = String::new();
                if file.take(MAX_TEXT).read_to_string(&mut provenance).is_ok() {
                    installed.build_date = build_date(&provenance);
                    installed.archive_digest = provenance
                        .lines()
                        .map(str::trim)
                        .map(|line| line.strip_prefix("sha256:").unwrap_or(line))
                        .find(|line| {
                            line.len() == 64 && line.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                        .map(|hex| format!("sha256:{}", hex.to_ascii_lowercase()));
                }
            }
        }
        Ok(installed)
    }

    fn file_version(path: &Path) -> Option<String> {
        use std::{ffi::c_void, os::windows::ffi::OsStrExt};
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
        };
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: the path/subblock are NUL-terminated, the resource buffer stays live
        // through the query, and the returned record is size-checked and read unaligned.
        unsafe {
            let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
            if size == 0 || size > 16 * 1024 * 1024 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
                return None;
            }
            let mut value: *mut c_void = std::ptr::null_mut();
            let mut length = 0;
            if VerQueryValueW(
                data.as_ptr().cast(),
                windows_sys::core::w!("\\"),
                &mut value,
                &mut length,
            ) == 0
                || value.is_null()
                || (length as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
            {
                return None;
            }
            let info = value.cast::<VS_FIXEDFILEINFO>().read_unaligned();
            if info.dwSignature != 0xfeef04bd {
                return None;
            }
            Some(format!(
                "{}.{}.{}.{}",
                info.dwFileVersionMS >> 16,
                info.dwFileVersionMS & 0xffff,
                info.dwFileVersionLS >> 16,
                info.dwFileVersionLS & 0xffff
            ))
        }
    }

    pub(super) fn latest(client: &Client) -> Result<Available, String> {
        let release = serde_json::from_slice(&text(client, RELEASE)?)
            .map_err(|_| "Could not read the latest playback release.")?;
        let bundle = assets(release)?;
        let version = bundle
            .0
            .name
            .trim_start_matches("mpv-dev-x86_64-")
            .trim_end_matches(".7z")
            .to_owned();
        Ok(Available { version, bundle })
    }

    pub(super) fn is_newer(latest: &Available, installed: &Installed) -> bool {
        if installed
            .build_date
            .zip(build_date(&latest.bundle.0.name))
            .is_some_and(|(current, next)| current > next)
        {
            return false;
        }
        if let Some(digest) = &installed.archive_digest {
            return Some(digest) != latest.bundle.0.digest.as_ref();
        }
        let revision = installed
            .mpv_version
            .rsplit_once("-g")
            .map(|(_, rev)| rev)
            .filter(|rev| rev.len() >= 7 && rev.bytes().all(|b| b.is_ascii_hexdigit()));
        let latest_revision = latest
            .bundle
            .0
            .name
            .strip_suffix(".7z")
            .and_then(|name| name.rsplit_once("-git-"))
            .map(|(_, rev)| rev);
        !revision
            .zip(latest_revision)
            .is_some_and(|(current, next)| next.starts_with(current) || current.starts_with(next))
    }

    fn build_date(text: &str) -> Option<u32> {
        text.split_once("mpv-dev-x86_64-")?.1.get(..8)?.parse().ok()
    }

    #[cfg(test)]
    pub(super) fn test_available() -> Available {
        let library = Asset {
            name: "mpv-dev-x86_64-20261111-git-bcde6b93cc.7z".into(),
            size: 3,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
            browser_download_url: format!(
                "{DOWNLOAD}20261111/mpv-dev-x86_64-20261111-git-bcde6b93cc.7z"
            ),
        };
        let mut graphics = library.clone();
        graphics.name = graphics.name.replacen("mpv-dev-", "mpv-", 1);
        graphics.browser_download_url = graphics
            .browser_download_url
            .replacen("mpv-dev-", "mpv-", 1);
        Available {
            version: "20261111".into(),
            bundle: (library, graphics),
        }
    }

    #[derive(Deserialize)]
    struct Release {
        assets: Vec<Asset>,
    }

    #[derive(Clone, Deserialize)]
    pub(crate) struct Asset {
        pub(crate) name: String,
        size: u64,
        digest: Option<String>,
        browser_download_url: String,
    }

    impl Asset {
        fn checksum(&self) -> Result<[u8; 32], String> {
            let hex = self
                .digest
                .as_deref()
                .and_then(|value| value.strip_prefix("sha256:"))
                .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or("The download has no valid SHA256 checksum.")?;
            let mut bytes = [0; 32];
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                    .map_err(|_| "Invalid download checksum.")?;
            }
            Ok(bytes)
        }

        pub(crate) fn validate(&self, prefix: &str) -> Result<(), String> {
            let url =
                Url::parse(&self.browser_download_url).map_err(|_| "Invalid download address.")?;
            if self.size == 0
                || self.size > MAX_ARCHIVE
                || !url.as_str().starts_with(prefix)
                || !self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path_segments().and_then(|mut parts| parts.next_back())
                    != Some(self.name.as_str())
            {
                return Err("Invalid playback download metadata.".into());
            }
            self.checksum().map(|_| ())
        }
    }

    fn assets(release: Release) -> Result<(Asset, Asset), String> {
        if !cfg!(target_arch = "x86_64") {
            return Err("Automatic playback setup currently supports Windows x64.".into());
        }
        let library = release
            .assets
            .iter()
            .find(|asset| {
                asset
                    .name
                    .strip_prefix("mpv-dev-x86_64-")
                    .is_some_and(|suffix| {
                        suffix.starts_with(|c: char| c.is_ascii_digit()) && suffix.ends_with(".7z")
                    })
            })
            .ok_or("The latest release has no portable Windows x64 playback libraries.")?
            .clone();
        let graphics_name = library.name.replacen("mpv-dev-", "mpv-", 1);
        let graphics = release
            .assets
            .into_iter()
            .find(|asset| asset.name == graphics_name)
            .ok_or("The latest release has no matching graphics runtime.")?;
        library.validate(DOWNLOAD)?;
        graphics.validate(DOWNLOAD)?;
        Ok((library, graphics))
    }

    pub(crate) fn client() -> Result<Client, String> {
        Client::builder()
            .user_agent("Perfuse-updater")
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(600))
            .redirect(Policy::custom(|attempt| {
                let url = attempt.url();
                if attempt.previous().len() < 5
                    && url.scheme() == "https"
                    && matches!(
                        url.host_str(),
                        Some(
                            "github.com"
                                | "release-assets.githubusercontent.com"
                                | "objects.githubusercontent.com"
                        )
                    )
                {
                    attempt.follow()
                } else {
                    attempt.error("Unexpected download redirect")
                }
            }))
            .build()
            .map_err(|_| "Could not initialize the downloader.".into())
    }

    fn text(client: &Client, url: &str) -> Result<Vec<u8>, String> {
        let response = client
            .get(url)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|_| {
                "Could not fetch playback release information. Check your connection and retry."
                    .to_owned()
            })?;
        let mut bytes = Vec::new();
        response
            .take(MAX_TEXT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read playback release information.")?;
        if bytes.len() as u64 > MAX_TEXT {
            return Err("Playback release information is too large.".into());
        }
        Ok(bytes)
    }

    pub(crate) fn download(
        client: &Client,
        asset: &Asset,
        path: &Path,
        label: &str,
        report: &dyn Fn(String),
    ) -> Result<(), String> {
        let response = client
            .get(&asset.browser_download_url)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|_| {
                "Could not download the update. Check your connection and retry.".to_owned()
            })?;
        save_download(response, asset, path, label, report)
    }

    fn save_download(
        mut source: impl Read,
        asset: &Asset,
        path: &Path,
        label: &str,
        report: &dyn Fn(String),
    ) -> Result<(), String> {
        let expected = asset.checksum()?;
        let mut file = fs::File::create(path).map_err(|_| "Could not save the download.")?;
        let mut hash = Context::new(&SHA256);
        let mut total = 0;
        let mut percent = 101;
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|_| "Download was interrupted. Please retry.")?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > asset.size {
                return Err("The download is larger than expected.".into());
            }
            file.write_all(&buffer[..count])
                .map_err(|_| "Could not save the download. Check free disk space.")?;
            hash.update(&buffer[..count]);
            let progress = total * 100 / asset.size;
            if progress != percent {
                percent = progress;
                report(format!("{label} ({percent}%)"));
            }
        }
        if total != asset.size || hash.finish().as_ref() != expected {
            return Err("Download failed its SHA256 check. Please retry.".into());
        }
        Ok(())
    }

    fn extract(archive: &Path, destination: &Path, member: &str) -> Result<(), String> {
        let system =
            std::env::var_os("SystemRoot").ok_or("Could not locate Windows archive tools.")?;
        // Only the named DLL is extracted; archive paths, scripts and executables are ignored.
        let output = Command::new(Path::new(&system).join("System32/tar.exe"))
            .args(["-xf"]).arg(archive).arg("-C").arg(destination).arg("--").arg(member)
            .creation_flags(0x08000000).output()
            .map_err(|_| "Could not run Windows tar.exe. Automatic setup requires Windows 10 1803 or newer.")?;
        if !output.status.success() || !destination.join(member).is_file() {
            return Err(
                "Could not extract the playback runtime. Check free disk space and retry.".into(),
            );
        }
        Ok(())
    }

    fn install_staged(
        dir: &Path,
        stage: &Path,
        selected: &Available,
        report: &dyn Fn(String),
    ) -> Result<(), String> {
        let client = client()?;
        let (library, graphics) = &selected.bundle;
        for (asset, member, label) in [
            (library, "libmpv-2.dll", "Downloading playback libraries"),
            (
                graphics,
                "d3dcompiler_43.dll",
                "Downloading graphics support",
            ),
        ] {
            let archive = stage.join(&asset.name);
            download(&client, asset, &archive, label, report)?;
            report("Extracting playback libraries…".into());
            extract(&archive, stage, member)?;
        }
        let revision = library
            .name
            .strip_suffix(".7z")
            .and_then(|name| name.rsplit_once("-git-"))
            .map(|(_, rev)| rev)
            .filter(|rev| {
                (7..=40).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_hexdigit())
            })
            .ok_or("The playback release has no source revision.")?;
        report("Checking playback libraries and license information…".into());
        let copyright = text(
            &client,
            &format!("https://raw.githubusercontent.com/mpv-player/mpv/{revision}/Copyright"),
        )?;
        fs::write(stage.join("LICENSE-mpv.txt"), copyright)
            .map_err(|_| "Could not save playback license information.")?;
        let mut provenance = format!(
            "Portable libmpv / FFmpeg runtime for Perfuse\n\n{}\n{}\n\n{}\n{}\n\nmpv source for this build:\nhttps://github.com/mpv-player/mpv/tree/{revision}\n\nLicenses and source:\n",
            library.browser_download_url,
            library.digest.as_deref().unwrap_or_default(),
            graphics.browser_download_url,
            graphics.digest.as_deref().unwrap_or_default()
        );
        for (name, url) in super::LICENSE_LINKS {
            provenance.push_str(&format!("{name}: {url}\n"));
        }
        fs::write(stage.join("THIRD-PARTY.txt"), provenance)
            .map_err(|_| "Could not save playback source information.")?;
        // Resolve the real C API before publishing the runtime. The temporary handle and DLL
        // are dropped on this worker before the DLL is moved or used by playback.
        drop(crate::mpv::Mpv::from_path(&stage.join("libmpv-2.dll"))?);
        fs::create_dir_all(dir).map_err(|_| "Could not create the playback runtime folder.")?;
        if dir.join("libmpv-2.dll").is_file() {
            return Ok(());
        }
        for member in [
            "d3dcompiler_43.dll",
            "LICENSE-mpv.txt",
            "THIRD-PARTY.txt",
            "libmpv-2.dll",
        ] {
            // The main DLL is published last, so interrupted setup remains retryable.
            fs::rename(stage.join(member), dir.join(member)).map_err(|_| "Could not install the playback libraries. Close other Perfuse instances and retry.")?;
        }
        Ok(())
    }

    pub(super) fn install(
        root: &Path,
        available: Option<&Available>,
        report: &dyn Fn(String),
    ) -> Result<Installed, String> {
        let parent = root.parent().ok_or("Invalid playback runtime folder.")?;
        fs::create_dir_all(root).map_err(|_| "Could not create the playback runtime folder.")?;
        let _operation = exclusive(&root.join(".manage")).map_err(
            |_| "Another Perfuse instance is managing the playback libraries. Please retry.",
        )?;
        let current = active_library(root);
        let selected = match available {
            Some(available) => {
                if let Some(path) = &current
                    && let Ok(installed) = inventory(path)
                    && !is_newer(available, &installed)
                {
                    cleanup(root);
                    return Ok(installed);
                }
                available.clone()
            }
            None => {
                if let Some(path) = current {
                    let installed = inventory(&path)?;
                    cleanup(root);
                    return Ok(installed);
                }
                report("Checking the latest playback libraries…".into());
                latest(&client()?)?
            }
        };
        let key = bundle_key(&selected)?;
        let dir = root.join(&key);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let stage = parent.join(format!(".runtime-install-{}-{stamp}", std::process::id()));
        fs::create_dir(&stage).map_err(|_| "Could not prepare the playback download folder.")?;
        let result = install_staged(&dir, &stage, &selected, report);
        let _ = fs::remove_dir_all(&stage);
        result?;
        let installed = inventory(&dir.join("libmpv-2.dll"))?;
        activate(root, &key)?;
        cleanup(root);
        Ok(installed)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn licence_acceptance_is_remembered() {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir()
                .join(format!("perfuse-licence-{}-{stamp}", std::process::id()));
            assert!(!license_accepted(&dir));
            accept_license(&dir).unwrap();
            assert!(license_accepted(&dir));
            accept_license(&dir).unwrap();
            fs::remove_file(dir.join(ACCEPTED_LICENSE)).unwrap();
            fs::remove_dir(dir).unwrap();
        }

        fn asset(name: &str) -> Asset {
            Asset {
                name: name.into(),
                size: 3,
                digest: Some(
                    "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                        .into(),
                ),
                browser_download_url: format!("{DOWNLOAD}20261111/{name}"),
            }
        }

        #[test]
        fn release_selection_and_download_trust_boundaries() {
            let name = "mpv-dev-x86_64-20261111-git-bcde6b93cc.7z";
            let library = asset(name);
            let graphics = asset(&name.replacen("mpv-dev-", "mpv-", 1));
            let (selected, _) = assets(Release {
                assets: vec![
                    asset("mpv-dev-x86_64-v3-20261111-git-bcde6b93cc.7z"),
                    library.clone(),
                    graphics.clone(),
                ],
            })
            .unwrap();
            assert_eq!(selected.name, name);
            assert_eq!(
                library.checksum().unwrap().as_slice(),
                ring::digest::digest(&SHA256, b"abc").as_ref()
            );
            assert!(
                assets(Release {
                    assets: vec![library.clone()]
                })
                .is_err()
            );
            for address in [
                "http://github.com/evil",
                "https://example.com/payload.7z",
                "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261111/other.7z",
            ] {
                let mut invalid = library.clone();
                invalid.browser_download_url = address.into();
                assert!(invalid.validate(DOWNLOAD).is_err());
            }
            for digest in [
                None,
                Some("sha256:no".into()),
                Some(format!("sha256:{}", "é".repeat(32))),
            ] {
                let mut invalid = library.clone();
                invalid.digest = digest;
                assert!(invalid.validate(DOWNLOAD).is_err());
            }
            let mut oversized = library;
            oversized.size = MAX_ARCHIVE + 1;
            assert!(oversized.validate(DOWNLOAD).is_err());
        }

        #[test]
        fn download_rejects_corrupt_truncated_and_oversized_data() {
            let path =
                std::env::temp_dir().join(format!("perfuse-download-{}.part", std::process::id()));
            let asset = asset("test.7z");
            assert!(save_download(&b"abc"[..], &asset, &path, "Download", &|_| {}).is_ok());
            for bytes in [&b"abd"[..], &b"ab"[..], &b"abcd"[..]] {
                assert!(save_download(bytes, &asset, &path, "Download", &|_| {}).is_err());
            }
            fs::remove_file(path).unwrap();
        }

        #[test]
        #[ignore = "downloads the latest upstream runtime; no playback or cache evaluation"]
        fn portable_runtime_install() {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "perfuse-runtime-test-{}-{stamp}",
                std::process::id()
            ));
            let dir = root.join("Runtime");
            let result = (|| -> Result<(), String> {
                let latest = latest(&client()?)?;
                let installed = install(&dir, Some(&latest), &|_| {})?;
                for library in &installed.libraries {
                    println!("{}: {}", library.name, library.version);
                }
                assert!(
                    installed
                        .libraries
                        .iter()
                        .any(|library| library.name == "FFmpeg")
                );
                assert!(!is_newer(&latest, &installed));
                let key = bundle_key(&latest)?;
                let versioned = dir.join(&key);
                let selected = inventory(&versioned.join("libmpv-2.dll"))?;
                assert!(!is_newer(&latest, &selected));
                assert_eq!(active_library(&dir), Some(versioned.join("libmpv-2.dll")));
                assert!(versioned.join("LICENSE-mpv.txt").is_file());
                assert!(versioned.join("THIRD-PARTY.txt").is_file());
                assert!(versioned.join("d3dcompiler_43.dll").is_file());
                drop(crate::mpv::Mpv::from_path(&versioned.join("libmpv-2.dll")).unwrap());
                Ok(())
            })();
            let _ = fs::remove_dir_all(&root);
            result.unwrap();
        }

        #[test]
        fn update_comparison_and_atomic_selection() {
            let latest = test_available();
            let mut installed = Installed {
                archive_digest: latest.bundle.0.digest.clone(),
                ..Default::default()
            };
            assert!(!is_newer(&latest, &installed));
            installed.archive_digest = Some(format!("sha256:{}", "b".repeat(64)));
            assert!(is_newer(&latest, &installed));
            installed.build_date = Some(20261112);
            assert!(!is_newer(&latest, &installed));
            installed.build_date = None;
            installed.archive_digest = None;
            installed.mpv_version = "v0.42.0-38-gbcde6b93c".into();
            assert!(!is_newer(&latest, &installed));
            installed.mpv_version = "v0.41.0-3-g012345678".into();
            assert!(is_newer(&latest, &installed));

            let root =
                std::env::temp_dir().join(format!("perfuse-selection-{}", std::process::id()));
            let first = "a".repeat(64);
            let next = "b".repeat(64);
            for key in [&first, &next] {
                fs::create_dir_all(root.join(key)).unwrap();
                fs::write(root.join(key).join("libmpv-2.dll"), b"synthetic").unwrap();
            }
            activate(&root, &first).unwrap();
            assert_eq!(
                active_library(&root),
                Some(root.join(&first).join("libmpv-2.dll"))
            );
            assert!(activate(&root, "../invalid").is_err());
            assert_eq!(
                active_library(&root),
                Some(root.join(&first).join("libmpv-2.dll"))
            );
            activate(&root, &next).unwrap();
            assert_eq!(
                active_library(&root),
                Some(root.join(&next).join("libmpv-2.dll"))
            );
            let lease = shared(&root.join(&first).join(".lease")).unwrap();
            cleanup(&root);
            assert!(root.join(&first).join("libmpv-2.dll").is_file());
            drop(lease);
            cleanup(&root);
            assert!(!root.join(&first).exists());
            assert!(root.join(&next).join("libmpv-2.dll").is_file());
            fs::write(root.join("current.txt"), "../invalid").unwrap();
            assert!(active_library(&root).is_none());
            fs::remove_dir_all(root).unwrap();
        }
    }
}
