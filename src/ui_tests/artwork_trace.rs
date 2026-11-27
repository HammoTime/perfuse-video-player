use super::*;
use perfuse::media::{Capabilities, MediaSource, Playback, Progress, SourceResult};
use std::{
    collections::HashSet,
    sync::{Condvar, Mutex},
    thread::ThreadId,
};

const ITEMS: usize = 48;
const SLOW: &str = "/trace/0";
const NEIGHBOR: &str = "/trace/1";
const IMAGE_SIZE: [usize; 2] = [192, 108];

#[derive(Default)]
struct TraceSource {
    calls: Mutex<Vec<(String, ThreadId)>>,
    release: (Mutex<u8>, Condvar),
}

impl TraceSource {
    fn record(&self, operation: &str) {
        self.calls
            .lock()
            .unwrap()
            .push((operation.to_owned(), thread::current().id()));
    }

    fn count(&self, operation: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == operation)
            .count()
    }

    fn release_through(&self, stage: u8) {
        *self.release.0.lock().unwrap() = stage;
        self.release.1.notify_all();
    }
}

impl MediaSource for TraceSource {
    fn address(&self) -> &str {
        "http://artwork-trace.invalid"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn libraries(&self) -> SourceResult<Vec<Library>> {
        self.record("libraries");
        Ok(vec![])
    }
    fn browse(&self, _: &Browse, _: &str, offset: usize) -> SourceResult<Page> {
        self.record("browse");
        let refreshed = self.count("browse") > 1;
        let items = (0..ITEMS)
            .map(|index| Item {
                id: index.to_string(),
                title: if refreshed {
                    format!("Updated trace {index}")
                } else {
                    format!("Trace {index}")
                },
                kind: Kind::Movie,
                last_viewed_at: Some((ITEMS - index) as u64),
                thumb: Some(format!("/trace/{index}")),
                ..Item::default()
            })
            .collect();
        Ok(Page::new(items, offset, Some(ITEMS)))
    }
    fn item(&self, _: &str) -> SourceResult<Item> {
        unreachable!("the test only browses")
    }
    fn cached_artwork(&self, _: &str) -> Option<image::RgbaImage> {
        self.record("cache");
        None
    }
    fn artwork(&self, path: &str) -> SourceResult<image::RgbaImage> {
        self.record(path);
        {
            let stage = if path == SLOW { 2 } else { 1 };
            let (gate, released) = &self.release;
            let (ready, _) = released
                .wait_timeout_while(gate.lock().unwrap(), Duration::from_secs(10), |ready| {
                    *ready < stage
                })
                .unwrap();
            assert!(
                *ready >= stage,
                "the UI failed to advance while artwork was in flight"
            );
        }
        Ok(image::RgbaImage::from_pixel(
            IMAGE_SIZE[0] as u32,
            IMAGE_SIZE[1] as u32,
            image::Rgba([80, 120, 160, 255]),
        ))
    }
    fn playback(&self, _: &Item, _: &Version, _: usize) -> SourceResult<Playback> {
        unreachable!("the test only browses")
    }
    fn report(&self, _: &Item, _: &Version, _: Progress) -> SourceResult<()> {
        unreachable!("the test only browses")
    }
}

#[derive(Default)]
struct FrameTrace {
    durations: Vec<Duration>,
    peak_uploads: usize,
}

impl FrameTrace {
    fn until(&mut self, ui: &mut Harness<'_, App>, ready: impl Fn(&Harness<'_, App>) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let start = Instant::now();
            ui.step();
            self.durations.push(start.elapsed());
            // Count only this fixture's preview textures, excluding fonts and UI icons.
            let uploads = ui
                .output()
                .textures_delta
                .set
                .iter()
                .filter(|(_, delta)| delta.image.size() == IMAGE_SIZE)
                .count();
            self.peak_uploads = self.peak_uploads.max(uploads);
            if ready(ui) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for an artwork frame"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
fn combined_home_loads_neighbors_while_one_cover_is_blocked_and_preserves_them_on_return() {
    let mut ui = app_harness([1920.0, 900.0], 1.0 / 60.0);
    let source = Arc::new(TraceSource::default());
    let app = ui.state_mut();
    app.source = Some(source.clone());
    attach_server(app);
    app.overview = true;
    ui.run_steps(3);
    let mut trace = FrameTrace::default();
    ui.get_by_label("Refresh servers").click();
    trace.until(&mut ui, |_| source.count("cache") >= 3);
    assert!(ui.query_by_label("Open Trace 0").is_some());
    // Let the first group complete together, keeping one visible image blocked.
    source.release_through(1);
    trace.until(&mut ui, |ui| {
        source.count(SLOW) == 1 && ui.state().servers[0].artwork.contains(NEIGHBOR)
    });
    assert!(!ui.state().servers[0].artwork.contains(SLOW));
    assert_eq!(
        source.count("/trace/47"),
        0,
        "distant pages were fetched eagerly"
    );
    source.release_through(2);
    trace.until(&mut ui, |ui| {
        let art = &ui.state().servers[0].artwork;
        art.contains(SLOW) && art.get(NEIGHBOR).is_some_and(|(_, opacity)| opacity == 1.0)
    });
    let original = ui.state().servers[0].artwork.get(NEIGHBOR).unwrap().0.id();
    ui.get_by_label("Scroll Continue Watching forward").click();
    trace.until(&mut ui, |ui| {
        ui.state().carousel_offsets["overview_continue"] > 200.0
    });
    ui.get_by_label("Scroll Continue Watching back").click();
    trace.until(&mut ui, |ui| {
        ui.state().carousel_offsets["overview_continue"] < 1.0
    });
    ui.get_by_label("Refresh servers").click();
    trace.until(&mut ui, |ui| {
        ui.query_by_label("Open Updated trace 1").is_some()
    });
    for _ in 0..10 {
        trace.until(&mut ui, |_| true);
        let (texture, opacity) = ui.state().servers[0].artwork.get(NEIGHBOR).unwrap();
        assert_eq!((texture.id(), opacity), (original, 1.0));
    }
    assert_eq!(source.count(NEIGHBOR), 1);
    assert!(
        (1..=4).contains(&trace.peak_uploads),
        "peak uploads {}",
        trace.peak_uploads
    );
    let calls = source.calls.lock().unwrap();
    let workers: HashSet<_> = calls.iter().map(|(_, id)| *id).collect();
    assert!(!workers.contains(&thread::current().id()));
    trace.durations.sort_unstable();
    let frames = trace.durations.len();
    eprintln!(
        "Home artwork trace: {frames} harness frames, median {:?}, p95 {:?}, max {:?}; {} worker threads, {} preview uploads in busiest frame",
        trace.durations[frames / 2],
        trace.durations[(frames - 1) * 95 / 100],
        trace.durations[frames - 1],
        workers.len(),
        trace.peak_uploads
    );
}
