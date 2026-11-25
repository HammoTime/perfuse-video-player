use crate::mpv::{self, Mpv};
use perfuse::media::{Item, MediaSource, PlaybackState, Progress, Source};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub const CACHE_SECONDS: u32 = 300;

pub enum Event {
    Playing {
        seconds: f64,
        duration: f64,
        buffered: f64,
        paused: bool,
        volume: f64,
    },
    Warning(String),
    Finished(Result<(), String>),
}

enum Control {
    Stop,
    Pause,
    Seek(f64),
    Volume(f64),
    Audio,
    Subtitles,
    Speed(f64),
    Chapter(i32),
}

pub struct Player {
    control: Sender<Control>,
    pub events: Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Player {
    #[cfg(test)]
    pub fn test_stub() -> Self {
        Self {
            control: mpsc::channel().0,
            events: mpsc::channel().1,
            worker: None,
        }
    }

    // A window ID is required for product playback; None is a headless integration test.
    pub fn start(
        window: Option<u64>,
        source: Source,
        item: Item,
        version: usize,
        resume: bool,
    ) -> Self {
        let (control, receiver) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = play(
                window,
                source.as_ref(),
                &item,
                version,
                resume,
                &receiver,
                &tx,
            );
            crate::runtime::cleanup();
            let _ = tx.send(Event::Finished(result));
        });
        Self {
            control,
            events,
            worker: Some(worker),
        }
    }
    pub fn stop(&self) {
        let _ = self.control.send(Control::Stop);
    }
    pub fn pause(&self) {
        let _ = self.control.send(Control::Pause);
    }
    pub fn seek(&self, seconds: f64) {
        let _ = self.control.send(Control::Seek(seconds.max(0.0)));
    }
    pub fn volume(&self, value: f64) {
        let _ = self.control.send(Control::Volume(value.clamp(0.0, 100.0)));
    }
    pub fn audio(&self) {
        let _ = self.control.send(Control::Audio);
    }
    pub fn subtitles(&self) {
        let _ = self.control.send(Control::Subtitles);
    }
    pub fn speed(&self, value: f64) {
        let _ = self.control.send(Control::Speed(value.clamp(0.5, 2.0)));
    }
    pub fn chapter(&self, direction: i32) {
        let _ = self.control.send(Control::Chapter(direction.signum()));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn configure(mpv: &Mpv, window: Option<u64>) -> Result<(), String> {
    let cache_seconds = CACHE_SECONDS.to_string();
    for (name, value) in [
        ("config", "no"),
        ("terminal", "no"),
        ("idle", "yes"),
        ("hwdec", "auto-safe"),
        ("tls-verify", "yes"),
        ("network-timeout", "15"),
        ("stream-lavf-o", "max_redirects=0"),
        ("access-references", "no"),
        ("ytdl", "no"),
        ("input-default-bindings", "no"),
        ("osc", "no"),
        ("osd-level", "0"),
        ("cache", "yes"),
        ("cache-secs", cache_seconds.as_str()),
        ("demuxer-readahead-secs", cache_seconds.as_str()),
        // Disk packets avoid the small RAM byte cap truncating UHD read-ahead to ~20s.
        // This cap now bounds packet metadata; temporary packet files are deleted on close.
        ("cache-on-disk", "yes"),
        ("demuxer-max-bytes", "128MiB"),
        ("demuxer-max-back-bytes", "16MiB"),
        ("demuxer-hysteresis-secs", "0"),
    ] {
        mpv.option(name, value)?;
    }
    if let Some(window) = window {
        mpv.option("wid", &window.to_string())?;
        mpv.option("vo", "gpu-next")?;
        mpv.option("gpu-api", "d3d11")?;
    } else {
        mpv.option("vo", "null")?;
        mpv.option("ao", "null")?;
    }
    mpv.initialize()
}

fn play(
    window: Option<u64>,
    source: &dyn MediaSource,
    item: &Item,
    version: usize,
    resume: bool,
    control: &Receiver<Control>,
    events: &Sender<Event>,
) -> Result<(), String> {
    let version = item
        .versions
        .get(version)
        .ok_or("No playable version is available.")?;
    let playback = source.playback(item, version)?;
    let mpv = Mpv::new()?;
    configure(&mpv, window)?;
    // Append each header without comma-list parsing or putting tokens in the URL.
    mpv.set("http-header-fields", "")?;
    for header in &playback.headers {
        mpv.command(&["change-list", "http-header-fields", "append", header])?;
    }
    let start = if resume {
        item.resume_ms.unwrap_or(0) as f64 / 1000.0
    } else {
        0.0
    };
    mpv.command(&[
        "loadfile",
        playback.url.as_str(),
        "replace",
        "-1",
        &format!("start={start}"),
    ])?;

    let mut seconds = start;
    let mut duration = item.duration_ms.unwrap_or(0) as f64 / 1000.0;
    let mut paused = false;
    let mut loaded = false;
    let mut last_report = Instant::now();
    let mut last_ui = Instant::now();
    let started = Instant::now();
    let mut result = Ok(());
    let mut warned = false;
    let report = |state, seconds, duration| {
        source.report(
            item,
            version,
            Progress {
                state,
                position_ms: milliseconds(seconds),
                duration_ms: milliseconds(duration),
            },
        )
    };
    'playback: loop {
        while let Ok(command) = control.try_recv() {
            let request = match command {
                Control::Stop => {
                    let _ = mpv.command(&["stop"]);
                    break 'playback;
                }
                Control::Pause => mpv.command(&["cycle", "pause"]),
                Control::Seek(value) => {
                    mpv.command(&["seek", &value.to_string(), "absolute+exact"])
                }
                Control::Volume(value) => mpv.set("volume", &value.to_string()),
                Control::Audio => mpv.command(&["cycle", "aid"]),
                Control::Subtitles => mpv.command(&["cycle", "sid"]),
                Control::Speed(value) => mpv.set("speed", &value.to_string()),
                Control::Chapter(direction) => {
                    mpv.command(&["add", "chapter", &direction.to_string()])
                }
            };
            if request.is_err() {
                let _ = events.send(Event::Warning(
                    "The playback control could not be applied.".into(),
                ));
            }
        }
        match mpv.event(0.1) {
            mpv::Event::Loaded => loaded = true,
            mpv::Event::Ended { eof, failed } => {
                if eof {
                    seconds = duration;
                }
                if failed {
                    result = Err("The embedded engine could not play this file. Check server access and the media format.".into());
                }
                break;
            }
            mpv::Event::Shutdown => break,
            mpv::Event::None => {}
        }
        if loaded && last_ui.elapsed() >= Duration::from_millis(200) {
            seconds = number(&mpv, "time-pos").unwrap_or(seconds);
            duration = number(&mpv, "duration").unwrap_or(duration);
            paused = mpv.get("pause").as_deref() == Some("yes");
            let _ = events.send(Event::Playing {
                seconds,
                duration,
                paused,
                buffered: number(&mpv, "demuxer-cache-duration").unwrap_or(0.0),
                volume: number(&mpv, "volume").unwrap_or(100.0),
            });
            last_ui = Instant::now();
        }
        if !loaded && started.elapsed() > Duration::from_secs(45) {
            result = Err("The media did not finish loading within 45 seconds.".into());
            break;
        }
        if loaded && last_report.elapsed() >= Duration::from_secs(10) {
            let state = if paused {
                PlaybackState::Paused
            } else {
                PlaybackState::Playing
            };
            if report(state, seconds, duration).is_err() && !warned {
                let _ = events.send(Event::Warning(
                    "Playback continues, but progress could not be saved to Plex.".into(),
                ));
                warned = true;
            }
            last_report = Instant::now();
        }
    }
    drop(mpv);
    if loaded && report(PlaybackState::Stopped, seconds, duration).is_err() {
        let _ = events.send(Event::Warning(
            "Playback stopped, but the final progress update could not reach Plex.".into(),
        ));
    }
    result
}

fn number(mpv: &Mpv, name: &str) -> Option<f64> {
    mpv.get(name)?.parse::<f64>().ok().filter(|n| n.is_finite())
}
fn milliseconds(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1000.0) as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn time_is_bounded() {
        assert_eq!(milliseconds(1.234), 1234);
        assert_eq!(milliseconds(f64::NAN), 0);
        assert_eq!(milliseconds(-1.0), 0);
    }

    #[test]
    #[ignore = "requires native libmpv and tools/demo_server.py"]
    fn embedded_playback_and_cache() {
        let engine = Mpv::new().unwrap();
        configure(&engine, None).unwrap();
        assert_eq!(
            number(&engine, "options/cache-secs"),
            Some(CACHE_SECONDS as f64)
        );
        assert_eq!(engine.get("options/cache-on-disk").as_deref(), Some("yes"));
        drop(engine);
        use perfuse::plugins::{Credentials, plex::PLUGIN};
        let source = (PLUGIN.open)(
            &Credentials {
                server: "http://127.0.0.1:32499".into(),
                token: "demo".into(),
                client_id: "perfuse-embedded-test".into(),
            },
            false,
        )
        .unwrap();
        let item = source.item("200").unwrap();
        let player = Player::start(None, source.clone(), item, 0, false);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut cache_ready = false;
        loop {
            assert!(
                Instant::now() < deadline,
                "media or five-minute cache did not become ready"
            );
            match player
                .events
                .recv_timeout(Duration::from_secs(10))
                .expect("no engine events")
            {
                Event::Playing {
                    seconds, buffered, ..
                } => {
                    cache_ready |= buffered >= 299.0;
                    if seconds >= 2.0 && cache_ready {
                        player.stop();
                    }
                }
                Event::Finished(result) => {
                    result.unwrap();
                    break;
                }
                Event::Warning(message) => panic!("{message}"),
            }
        }
        assert!(
            cache_ready,
            "five minutes of media was not actually buffered"
        );
        let resumed = source.item("200").unwrap();
        let saved = resumed.resume_ms.unwrap() as f64 / 1000.0;
        assert!(saved >= 2.0);
        let player = Player::start(None, source.clone(), resumed, 0, true);
        loop {
            assert!(Instant::now() < deadline, "resume timed out");
            match player.events.recv_timeout(Duration::from_secs(10)).unwrap() {
                Event::Playing { seconds, .. } if seconds > saved + 1.0 => break,
                Event::Finished(result) => panic!("resume ended early: {result:?}"),
                Event::Warning(message) => panic!("{message}"),
                _ => {}
            }
        }
        drop(player);
        assert!(source.item("200").unwrap().resume_ms.unwrap() as f64 > saved * 1000.0);
    }
}
