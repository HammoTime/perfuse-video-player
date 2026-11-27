//! Perfuse release checks and the handoff to the Windows installer.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(windows)]
use crate::runtime::windows::{Asset, client, download};
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
#[cfg(windows)]
use std::{
    fs,
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const REPOSITORY: &str = "HammoTime/perfuse-video-player";

#[derive(Clone)]
pub struct Available {
    pub version: String,
    #[cfg(windows)]
    installer: Asset,
}

impl Available {
    /// The GitHub release page for manual downloads when this copy cannot update itself.
    pub fn release_page(&self) -> String {
        format!(
            "https://github.com/{REPOSITORY}/releases/tag/v{}",
            self.version
        )
    }
}

/// The per-user installation that client updates replace and relaunch.
#[cfg(windows)]
fn installed_location() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|local| PathBuf::from(local).join("Programs/Perfuse/perfuse.exe"))
}

/// True only when this executable is the per-user installation. Development builds and
/// portable copies would otherwise update and relaunch a different executable.
pub fn self_install_supported() -> bool {
    #[cfg(windows)]
    {
        let canonical = |path: PathBuf| fs::canonicalize(path).ok();
        let installed = installed_location().and_then(canonical);
        installed.is_some() && std::env::current_exe().ok().and_then(canonical) == installed
    }
    #[cfg(not(windows))]
    false
}

enum Event {
    Checked(Result<Option<Available>, String>),
    #[cfg(windows)]
    Progress(String),
    #[cfg(windows)]
    Launched(Result<(), String>),
}

#[derive(Default)]
pub struct Updates {
    pub available: Option<Available>,
    pub status: Option<Result<String, String>>,
    pub downloading: bool,
    job: Option<Receiver<Event>>,
}

impl Updates {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn spawn(&mut self, ctx: &egui::Context, task: impl FnOnce(&dyn Fn(Event)) + Send + 'static) {
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        std::thread::spawn(move || {
            task(&|event| {
                let _ = tx.send(event);
                ctx.request_repaint();
            });
        });
    }

    pub fn check(&mut self, ctx: &egui::Context) {
        self.status = Some(Ok("Checking for Perfuse updates…".into()));
        self.spawn(ctx, |emit| {
            #[cfg(windows)]
            let result = latest();
            #[cfg(not(windows))]
            let result = Err("Perfuse updates are available on Windows x64.".into());
            emit(Event::Checked(result));
        });
    }

    pub fn install(&mut self, ctx: &egui::Context, available: Available) {
        #[cfg(windows)]
        {
            self.downloading = true;
            self.status = Some(Ok("Preparing the Perfuse update…".into()));
            self.spawn(ctx, move |emit| {
                emit(Event::Launched(install(&available, &|message| {
                    emit(Event::Progress(message));
                })));
            });
        }
        #[cfg(not(windows))]
        let _ = (ctx, available);
    }

    pub fn poll(&mut self) -> bool {
        while let Some(event) = self.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match event {
                Event::Checked(result) => {
                    self.job = None;
                    self.status = Some(result.map(|available| {
                        self.available = available;
                        match &self.available {
                            Some(update) => format!("Perfuse {} is available.", update.version),
                            None => "Perfuse is up to date.".into(),
                        }
                    }));
                }
                #[cfg(windows)]
                Event::Progress(message) => self.status = Some(Ok(message)),
                #[cfg(windows)]
                Event::Launched(result) => {
                    self.job = None;
                    self.downloading = false;
                    match result {
                        Ok(()) => {
                            self.available = None;
                            self.status = Some(Ok("Restarting Perfuse…".into()));
                            return true;
                        }
                        Err(error) => self.status = Some(Err(error)),
                    }
                }
            }
        }
        false
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Alpha,
    Beta,
    Stable,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version([u64; 3], Family, u64);

#[cfg(any(windows, test))]
impl Version {
    fn parse(version: &str) -> Option<Self> {
        fn number(text: &str) -> Option<u64> {
            if text.bytes().all(|byte| byte.is_ascii_digit())
                && (text.len() <= 1 || !text.starts_with('0'))
            {
                text.parse().ok()
            } else {
                None
            }
        }
        let (core, preview) = version
            .split_once('-')
            .map_or((version, None), |(core, preview)| (core, Some(preview)));
        let mut parts = core.split('.');
        let core = [
            number(parts.next()?)?,
            number(parts.next()?)?,
            number(parts.next()?)?,
        ];
        if parts.next().is_some() {
            return None;
        }
        let (family, sequence) = if let Some(preview) = preview {
            let (family, sequence) = preview.split_once('.')?;
            let family = match family {
                "alpha" => Family::Alpha,
                "beta" => Family::Beta,
                _ => return None,
            };
            let sequence = number(sequence)?;
            (family, (sequence > 0).then_some(sequence)?)
        } else {
            (Family::Stable, 0)
        };
        Some(Self(core, family, sequence))
    }

    fn accepts(self, next: Self) -> bool {
        next > self && next.1 >= self.1
    }
}

#[cfg(windows)]
fn latest() -> Result<Option<Available>, String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        draft: bool,
        prerelease: bool,
        assets: Vec<Asset>,
    }

    let current = Version::parse(VERSION).ok_or("This Perfuse build has no release version.")?;
    // ponytail: scan the 100 most recent releases; paginate if a channel regularly goes missing.
    let response = client()?
        .get(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100"
        ))
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .map_err(|_| "Could not check for Perfuse updates. Check your connection and retry.")?;
    if !response.status().is_success() {
        return Err("GitHub could not provide the Perfuse release list. Try again later.".into());
    }
    let mut bytes = Vec::new();
    const LIMIT: u64 = 8 * 1024 * 1024;
    response
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the Perfuse release list.")?;
    if bytes.len() as u64 > LIMIT {
        return Err("The Perfuse release list is too large.".into());
    }
    let releases: Vec<Release> =
        serde_json::from_slice(&bytes).map_err(|_| "Could not read the Perfuse release list.")?;
    let found = releases
        .into_iter()
        .filter_map(|release| {
            let version = release.tag_name.strip_prefix('v')?;
            let next = Version::parse(version)?;
            if release.draft
                || release.prerelease != (next.1 != Family::Stable)
                || !current.accepts(next)
            {
                return None;
            }
            let name = format!("Perfuse-{}-windows-x64-setup.exe", release.tag_name);
            let installer = release
                .assets
                .into_iter()
                .find(|asset| asset.name == name)?;
            let prefix = format!(
                "https://github.com/{REPOSITORY}/releases/download/{}/",
                release.tag_name
            );
            Some((next, version.to_owned(), installer, prefix))
        })
        .max_by_key(|(next, ..)| *next);
    found
        .map(|(_, version, installer, prefix)| {
            installer.validate(&prefix)?;
            Ok(Available { version, installer })
        })
        .transpose()
}

#[cfg(windows)]
fn install(available: &Available, report: &dyn Fn(String)) -> Result<(), String> {
    if !self_install_supported() {
        return Err("This copy of Perfuse runs outside its installed folder, so it cannot update itself. Download the update from the release page instead.".into());
    }
    // The script checks the installer against this digest again just before running it.
    let digest = available.installer.sha256_hex()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("perfuse-update-{}-{now}", std::process::id()));
    let system = std::env::var_os("SystemRoot").ok_or("Could not locate Windows PowerShell.")?;
    let installed = installed_location().ok_or("Could not locate your local app data folder.")?;
    let current = std::env::current_exe().map_err(|_| "Could not locate Perfuse.")?;
    fs::create_dir(&dir).map_err(|_| "Could not create a temporary folder for the update.")?;
    let installer = dir.join(&available.installer.name);
    let result = (|| {
        download(
            &client()?,
            &available.installer,
            &installer,
            "Downloading Perfuse",
            report,
        )?;
        Command::new(Path::new(&system).join("System32/WindowsPowerShell/v1.0/powershell.exe"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("../installer/finish-update.ps1"),
            ])
            .env("PERFUSE_UPDATE_PID", std::process::id().to_string())
            .env("PERFUSE_UPDATE_SETUP", &installer)
            .env("PERFUSE_UPDATE_SHA256", &digest)
            .env("PERFUSE_UPDATE_APP", &installed)
            .env("PERFUSE_UPDATE_PREVIOUS", &current)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "Could not start the Perfuse installer.")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&installer);
        let _ = fs::remove_dir(&dir);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_channels_compare_versions_numerically() {
        let version = |text| Version::parse(text).unwrap();
        let alpha = version("0.1.10-alpha.2");
        assert!(alpha.accepts(version("0.1.10-alpha.10")));
        assert!(alpha.accepts(version("0.1.10-beta.1")));
        assert!(alpha.accepts(version("0.1.10")));
        assert!(!alpha.accepts(version("0.1.9")));
        assert!(!version("0.1.10-beta.1").accepts(version("0.1.11-alpha.1")));
        assert!(!version("0.1.10").accepts(version("0.1.11-beta.1")));
        assert!(Version::parse("0.1.11-beta.01").is_none());
    }
}
