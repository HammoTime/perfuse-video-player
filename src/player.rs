use crate::mpv::{self, Mpv};
use perfuse::media::{
    AudioTrack, Item, MediaSource, Playback, PlaybackState, Progress, Source, Stream,
};
use reqwest::header::{HeaderName, HeaderValue};
use std::{
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub const CACHE_SECONDS: u32 = 300;
/// An end of file this far before the expected end is treated as a dropped stream.
const EARLY_END_SECONDS: f64 = 10.0;

/// libmpv writes read-ahead packets here. With `config=no` it has no default cache folder
/// and would keep read-ahead in RAM, where demuxer-max-bytes caps it well below 300 seconds.
pub fn cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Perfuse")
        .join("Playback cache")
}

/// Removes packet files a crashed session left behind. Files a running player holds stay.
pub fn trim_cache() {
    if cfg!(test) {
        return;
    }
    let Ok(entries) = fs::read_dir(cache_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[derive(Clone, Copy)]
enum Output {
    Window(u64),
    Audio,
    Null,
}

#[derive(Clone, Copy)]
pub struct StartOptions {
    pub version: usize,
    pub resume: bool,
    pub volume: f64,
}

pub enum Event {
    Playing {
        seconds: f64,
        duration: f64,
        buffered: f64,
        paused: bool,
        #[cfg(test)]
        volume: f64,
    },
    Warning(String),
    BookCompleted,
    ChapterSleepExpired(f64),
    Finished(Result<(), String>),
}

enum Control {
    Stop,
    Pause,
    SetPaused(bool),
    Seek(f64),
    Volume(f64),
    Audio,
    Subtitles,
    Speed(f64),
    Chapter(i32),
    SleepAfterChapter(bool),
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

    // A window ID embeds video. Audiobooks use the normal audio output without a video child.
    pub fn start(window: Option<u64>, source: Source, item: Item, options: StartOptions) -> Self {
        let (control, receiver) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let worker = thread::spawn(move || {
            let output = if item.is_book() {
                Output::Audio
            } else {
                window.map_or(Output::Null, Output::Window)
            };
            // A panic must still end the session in the UI. Unwinding destroys libmpv first.
            let result = catch_unwind(AssertUnwindSafe(|| {
                play(output, source.as_ref(), &item, options, &receiver, &tx)
            }))
            .unwrap_or_else(|_| Err("Playback stopped unexpectedly. Please try again.".into()));
            crate::runtime::cleanup();
            let _ = tx.send(Event::Finished(result));
        });
        Self {
            control,
            events,
            worker: Some(worker),
        }
    }

    /// Asks the worker to save progress and stop without waiting for it. Keep any video
    /// surface alive until the returned handle reports that the worker finished.
    pub fn retire(mut self) -> Retired {
        self.stop();
        Retired(self.worker.take())
    }
    pub fn stop(&self) {
        let _ = self.control.send(Control::Stop);
    }
    pub fn pause(&self) {
        let _ = self.control.send(Control::Pause);
    }
    pub fn set_paused(&self, paused: bool) {
        let _ = self.control.send(Control::SetPaused(paused));
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
        let _ = self.control.send(Control::Speed(value.clamp(0.5, 3.0)));
    }
    pub fn chapter(&self, direction: i32) {
        let _ = self.control.send(Control::Chapter(direction.signum()));
    }
    pub fn sleep_after_chapter(&self, enabled: bool) {
        let _ = self.control.send(Control::SleepAfterChapter(enabled));
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

/// A player that was asked to stop. Its worker may still be saving the final position.
pub struct Retired(Option<thread::JoinHandle<()>>);

impl Retired {
    pub fn is_finished(&self) -> bool {
        self.0.as_ref().is_none_or(thread::JoinHandle::is_finished)
    }

    /// Waits for the worker until `deadline`. Returns false if it is still running.
    pub fn wait_until(&self, deadline: Instant) -> bool {
        while !self.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
        true
    }

    /// Stops tracking a worker that outlived its deadline. It ends with the process.
    pub fn detach(mut self) {
        self.0.take();
    }
}

impl Drop for Retired {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            let _ = worker.join();
        }
    }
}

fn configure(mpv: &Mpv, output: Output) -> Result<(), String> {
    let cache_seconds = CACHE_SECONDS.to_string();
    let cache_dir = cache_dir();
    fs::create_dir_all(&cache_dir)
        .map_err(|_| "Could not create the playback cache folder.".to_owned())?;
    mpv.option("demuxer-cache-dir", &cache_dir.to_string_lossy())?;
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
    match output {
        Output::Window(window) => {
            mpv.option("wid", &window.to_string())?;
            mpv.option("vo", "gpu-next")?;
            mpv.option("gpu-api", "d3d11")?;
        }
        Output::Audio => {
            mpv.option("vid", "no")?;
            mpv.option("vo", "null")?;
        }
        Output::Null => {
            mpv.option("vo", "null")?;
            mpv.option("ao", "null")?;
        }
    }
    mpv.initialize()
}

// A replacement can leave END_FILE and FILE_LOADED events for the previous entry queued.
// Match them to START_FILE before updating the book's position or advancing a track.
struct PlayingFile {
    index: usize,
    offset: f64,
    entry: i64,
    headers: Vec<(HeaderName, HeaderValue)>,
    ready: bool,
    opening: Instant,
    retried_at: Option<f64>,
}

impl PlayingFile {
    fn open(
        mpv: &Mpv,
        playback: Playback,
        index: usize,
        offset: f64,
        position: f64,
    ) -> Result<Self, String> {
        mpv.set("http-header-fields", "")?;
        // One list entry per header: -append never splits a value at its commas.
        for header in playback.header_lines() {
            mpv.command(&["change-list", "http-header-fields", "append", &header])?;
        }
        mpv.command(&[
            "loadfile",
            playback.url.as_str(),
            "replace",
            "-1",
            &format!("start={}", (position - offset).max(0.0)),
        ])?;
        let entry = mpv
            .get("playlist/0/id")
            .and_then(|id| id.parse().ok())
            .ok_or("The playback engine could not select the file.")?;
        Ok(Self {
            index,
            offset,
            entry,
            headers: playback.headers,
            ready: false,
            opening: Instant::now(),
            retried_at: None,
        })
    }
}

fn track_at(tracks: &[AudioTrack], seconds: f64) -> usize {
    let position = (seconds * 1000.0).round() as u64;
    tracks
        .partition_point(|track| track.start_ms <= position)
        .saturating_sub(1)
}

/// The last position has already been saved. Leave the book loaded in the UI
/// and keep its speed and volume while the engine has no active file.
fn wait_for_replay(
    mpv: &Mpv,
    duration: f64,
    control: &Receiver<Control>,
    events: &Sender<Event>,
) -> Option<(f64, bool)> {
    let mut position = 0.0;
    let mut chapter_sleep = false;
    let _ = events.send(Event::BookCompleted);
    while let Ok(command) = control.recv() {
        match command {
            Control::Stop => break,
            Control::Pause | Control::SetPaused(false) => {
                return Some((position, chapter_sleep));
            }
            Control::SleepAfterChapter(enabled) => chapter_sleep = enabled,
            Control::Seek(value) => position = value.min(duration),
            Control::Volume(value) => {
                let _ = mpv.set("volume", &value.to_string());
            }
            Control::Speed(value) => {
                let _ = mpv.set("speed", &value.to_string());
            }
            _ => continue,
        }
        let _ = events.send(Event::Playing {
            seconds: position,
            duration,
            buffered: 0.0,
            paused: true,
            #[cfg(test)]
            volume: number(mpv, "volume").unwrap(),
        });
    }
    None
}

fn play(
    output: Output,
    source: &dyn MediaSource,
    item: &Item,
    options: StartOptions,
    control: &Receiver<Control>,
    events: &Sender<Event>,
) -> Result<(), String> {
    let version = item
        .versions
        .get(options.version)
        .ok_or("No playable version is available.")?;
    let mpv = Mpv::new()?;
    configure(&mpv, output)?;
    mpv.set("volume", &options.volume.clamp(0.0, 100.0).to_string())?;
    let tracks = match &version.stream {
        Stream::Audiobook(tracks) if tracks.is_empty() => {
            return Err("This book has no audio tracks.".into());
        }
        Stream::Audiobook(tracks) => tracks.as_slice(),
        _ => &[],
    };
    // Fetched for every open, so a rotated bearer reaches libmpv on the next file.
    let playback_for = |index: usize| source.playback(item, version, index).map_err(String::from);
    let offset = |index: usize| {
        tracks
            .get(index)
            .map_or(0.0, |t| t.start_ms as f64 / 1000.0)
    };
    let start = if options.resume {
        item.resume_ms.unwrap_or(0) as f64 / 1000.0
    } else {
        0.0
    };
    let index = track_at(tracks, start);
    let mut file = PlayingFile::open(&mpv, playback_for(index)?, index, offset(index), start)?;
    let mut event_entry = None;
    let seek = |file: &mut PlayingFile, target: f64| -> Result<(), String> {
        let index = track_at(tracks, target);
        // Seeking within the open file needs no request. If its bearer expired meanwhile,
        // the dropped or failed read is reopened below with a renewed header.
        if tracks.is_empty() || (index == file.index && file.ready) {
            return mpv.command(&[
                "seek",
                &(target - file.offset).max(0.0).to_string(),
                "absolute+exact",
            ]);
        }
        *file = PlayingFile::open(&mpv, playback_for(index)?, index, offset(index), target)?;
        Ok(())
    };
    let mut seconds = start;
    let chapters = item.book.chapters.as_slice();
    let chapter_at =
        |position: f64| chapters.partition_point(|c| c.end_ms <= milliseconds(position));
    let mut sleep_chapter = None;
    let mut duration = item.duration_ms.unwrap_or(0) as f64 / 1000.0;
    let mut paused = false;
    let mut loaded = false;
    let mut last_report = Instant::now();
    let mut last_ui = Instant::now();
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
                Control::SetPaused(paused) => mpv.set("pause", if paused { "yes" } else { "no" }),
                Control::Seek(value) => {
                    let target = if duration > 0.0 {
                        value.min(duration)
                    } else {
                        value
                    };
                    seek(&mut file, target).map(|()| {
                        seconds = target;
                        if sleep_chapter.is_some() {
                            sleep_chapter = Some(chapter_at(target));
                        }
                    })
                }
                Control::SleepAfterChapter(enabled) => {
                    sleep_chapter = enabled.then(|| chapter_at(seconds));
                    Ok(())
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
            mpv::Event::StartFile(id) => event_entry = Some(id),
            mpv::Event::Loaded if event_entry == Some(file.entry) => {
                file.ready = true;
                if !loaded {
                    loaded = true;
                    if report(PlaybackState::Started, seconds, duration).is_err() {
                        let _ = events.send(Event::Warning(
                            "Could not report the start of playback to the server.".into(),
                        ));
                    }
                }
            }
            mpv::Event::Ended { eof, failed } if event_entry == Some(file.entry) => {
                let next = file.index + 1;
                let fresh = file
                    .retried_at
                    .is_none_or(|last| (seconds - last).abs() >= 5.0);
                // FFmpeg can reopen a byte range on a long book after its bearer has expired.
                // Retry once at this position, and only when the server supplies a new bearer.
                if failed
                    && !tracks.is_empty()
                    && fresh
                    && let Ok(playback) = playback_for(file.index)
                    && playback.headers != file.headers
                    && let Ok(mut opened) =
                        PlayingFile::open(&mpv, playback, file.index, file.offset, seconds)
                {
                    opened.retried_at = Some(seconds);
                    file = opened;
                    continue;
                }
                // libmpv also reports a dropped connection as the end of the file. Reopen
                // once at this position when the file ends well before its expected end.
                // A file that ends again right away has genuinely ended, so a duration
                // estimate that runs long costs one reopen, never an error.
                let expected_end = if next < tracks.len() {
                    offset(next)
                } else {
                    duration
                };
                if eof && !failed && fresh && expected_end - seconds > EARLY_END_SECONDS {
                    match playback_for(file.index).and_then(|playback| {
                        PlayingFile::open(&mpv, playback, file.index, file.offset, seconds)
                    }) {
                        Ok(mut opened) => {
                            opened.retried_at = Some(seconds);
                            file = opened;
                            continue;
                        }
                        Err(_) => {
                            result = Err("The connection to the server dropped during playback. Your position was saved.".into());
                            break;
                        }
                    }
                }
                if eof && !failed && next < tracks.len() {
                    seconds = offset(next);
                    match playback_for(next)
                        .and_then(|p| PlayingFile::open(&mpv, p, next, seconds, seconds))
                    {
                        Ok(opened) => {
                            file = opened;
                            continue;
                        }
                        Err(error) => {
                            result = Err(error);
                            break;
                        }
                    }
                }
                if eof && !failed {
                    seconds = duration;
                    if !tracks.is_empty() {
                        if report(PlaybackState::Stopped, seconds, duration).is_err() {
                            let _ = events.send(Event::Warning(
                                "The book finished, but its final position could not be saved."
                                    .into(),
                            ));
                        }
                        let Some((restart, chapter_sleep)) =
                            wait_for_replay(&mpv, duration, control, events)
                        else {
                            return Ok(());
                        };
                        let index = track_at(tracks, restart);
                        // The finished position is already saved, so a failed restart
                        // reports nothing further.
                        loaded = false;
                        let reopened = mpv.set("pause", "no").and_then(|()| {
                            PlayingFile::open(
                                &mpv,
                                playback_for(index)?,
                                index,
                                offset(index),
                                restart,
                            )
                        });
                        file = match reopened {
                            Ok(file) => file,
                            Err(error) => {
                                result = Err(error);
                                break;
                            }
                        };
                        seconds = restart;
                        sleep_chapter = chapter_sleep.then(|| chapter_at(restart));
                        warned = false;
                        last_report = Instant::now();
                        continue;
                    }
                }
                if failed {
                    result = Err("The embedded engine could not play this file. Check server access and the media format.".into());
                }
                break;
            }
            mpv::Event::Shutdown => break,
            _ => {}
        }
        if file.ready && last_ui.elapsed() >= Duration::from_millis(200) {
            seconds = number(&mpv, "time-pos").map_or(seconds, |pos| pos + file.offset);
            if tracks.is_empty() {
                duration = number(&mpv, "duration").unwrap_or(duration);
            }
            paused = mpv.get("pause").as_deref() == Some("yes");
            if let Some(index) = sleep_chapter
                && let Some(chapter) = chapters.get(index)
                && chapter.end_ms < milliseconds(duration)
                && !paused
                && milliseconds(seconds) >= chapter.end_ms
                && mpv.get("seeking").as_deref() != Some("yes")
            {
                let end = chapters
                    .get(index + 1)
                    .map_or(chapter.end_ms, |next| next.start_ms) as f64
                    / 1000.0;
                sleep_chapter = None;
                // A failed seek leaves the book paused where it is. It must never end the
                // session, which would skip the final progress report.
                if mpv.set("pause", "yes").is_ok() {
                    paused = true;
                    if seek(&mut file, end).is_ok() {
                        seconds = end;
                    } else {
                        let _ = events.send(Event::Warning(
                            "Paused, but the end of the chapter could not be reached.".into(),
                        ));
                    }
                    let _ = events.send(Event::ChapterSleepExpired(seconds));
                }
            }
            let _ = events.send(Event::Playing {
                seconds,
                duration,
                paused,
                buffered: number(&mpv, "demuxer-cache-duration").unwrap_or(0.0),
                #[cfg(test)]
                volume: number(&mpv, "volume").unwrap(),
            });
            last_ui = Instant::now();
        }
        if !file.ready && file.opening.elapsed() > Duration::from_secs(45) {
            result = Err("The media did not finish loading within 45 seconds.".into());
            break;
        }
        if file.ready && last_report.elapsed() >= Duration::from_secs(10) {
            let state = if paused {
                PlaybackState::Paused
            } else {
                PlaybackState::Playing
            };
            if report(state, seconds, duration).is_err() && !warned {
                let _ = events.send(Event::Warning(
                    "Playback continues, but progress could not be saved to the server.".into(),
                ));
                warned = true;
            }
            last_report = Instant::now();
        }
    }
    drop(mpv);
    if loaded && report(PlaybackState::Stopped, seconds, duration).is_err() {
        let _ = events.send(Event::Warning(
            "Playback stopped, but the final progress update could not reach the server.".into(),
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
mod book_tests;

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
        configure(&engine, Output::Null).unwrap();
        assert_eq!(
            number(&engine, "options/cache-secs"),
            Some(CACHE_SECONDS as f64)
        );
        assert_eq!(engine.get("options/cache-on-disk").as_deref(), Some("yes"));
        drop(engine);
        use perfuse::plugins::{Credentials, plex::PLUGIN};
        let source = (PLUGIN.open)(
            &Credentials {
                kind: perfuse::plugins::ServerKind::Plex,
                server: "http://127.0.0.1:32499".into(),
                token: "demo".into(),
                client_id: "perfuse-embedded-test".into(),
            },
            perfuse::plugins::OpenOptions::default(),
        )
        .unwrap();
        let item = source.item("200").unwrap();
        let options = StartOptions {
            version: 0,
            resume: false,
            volume: 100.0,
        };
        let player = Player::start(None, source.clone(), item, options);
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
                Event::BookCompleted | Event::ChapterSleepExpired(_) => {
                    panic!("unexpected audiobook event for a Plex title")
                }
            }
        }
        assert!(
            cache_ready,
            "five minutes of media was not actually buffered"
        );
        let resumed = source.item("200").unwrap();
        let saved = resumed.resume_ms.unwrap() as f64 / 1000.0;
        assert!(saved >= 2.0);
        let player = Player::start(
            None,
            source.clone(),
            resumed,
            StartOptions {
                resume: true,
                ..options
            },
        );
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
