//! One resident artwork cache per source. Painting describes the current viewport;
//! after layout, the scheduler services visible slots before their neighbours.
use crate::{Source, egui, prefs, tasks};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

const MAX_JOBS: usize = 4;
const MAX_UPLOADS: usize = 4;
const UPLOAD_BUDGET: Duration = Duration::from_millis(2);
const RESIDENT_BYTES: usize = 96 * 1024 * 1024;
const MAX_RESIDENT: usize = 192;
const FADE: f64 = 0.083;
const RETRY_AFTER: f64 = 45.0;

#[derive(Clone, Default)]
pub(super) struct Artwork(Rc<RefCell<Cache>>);

#[derive(Default)]
struct Cache {
    ready: HashMap<String, Preview>,
    bytes: usize,
    evicted: VecDeque<String>,
    failed_until: HashMap<String, f64>,
    wanted: Vec<Slot>,
    now: f64,
}

struct Slot {
    path: String,
    visible: bool,
    behind: bool,
    pos: egui::Pos2,
}

struct Preview {
    texture: egui::TextureHandle,
    reveal: f64,
    last_used: f64,
}

impl Artwork {
    #[cfg(test)]
    pub(super) fn insert(&self, path: String, texture: egui::TextureHandle) {
        self.store(path, texture, f64::NEG_INFINITY);
    }

    #[cfg(test)]
    pub(super) fn contains(&self, path: &str) -> bool {
        self.0.borrow().ready.contains_key(path)
    }

    fn store(&self, path: String, texture: egui::TextureHandle, reveal: f64) {
        let mut cache = self.0.borrow_mut();
        if cache.ready.contains_key(&path) {
            return;
        }
        let last_used = cache.now;
        cache.failed_until.remove(&path);
        // Remember recent evictions even when artwork isn't persisted to disk.
        let reveal = if cache.evicted.contains(&path) {
            f64::NEG_INFINITY
        } else {
            reveal
        };
        cache.bytes += texture.byte_size();
        cache.ready.insert(
            path,
            Preview {
                texture,
                reveal,
                last_used,
            },
        );
    }

    pub(super) fn get(&self, path: &str) -> Option<(egui::TextureHandle, f32)> {
        let cache = self.0.borrow();
        let entry = cache.ready.get(path)?;
        let opacity = ((cache.now - entry.reveal) / FADE).clamp(0.0, 1.0) as f32;
        Some((entry.texture.clone(), opacity))
    }

    pub(super) fn failed(&self, path: &str) -> bool {
        self.0.borrow().failed_until.contains_key(path)
    }

    pub(super) fn observe(&self, ui: &egui::Ui, rect: egui::Rect, path: &str) {
        let clip = ui.clip_rect();
        if !ui.is_visible() || !clip.expand2(rect.size()).intersects(rect) {
            return;
        }
        let visible = clip.intersects(rect);
        let behind = !visible
            && ui
                .stack()
                .iter()
                .any(|frame| frame.tags().contains("prefetch-behind"));
        let now = ui.input(|i| i.time);
        let mut cache = self.0.borrow_mut();
        cache.now = now;
        if let Some(entry) = cache.ready.get_mut(path) {
            entry.last_used = now;
        }
        if let Some(slot) = cache.wanted.iter_mut().find(|slot| slot.path == path) {
            if visible && !slot.visible {
                slot.visible = true;
                slot.pos = rect.min;
            }
            slot.behind &= behind;
        } else {
            cache.wanted.push(Slot {
                path: path.to_owned(),
                visible,
                behind,
                pos: rect.min,
            });
        }
    }

    fn trim(&self, protected: &[Slot]) {
        let mut cache = self.0.borrow_mut();
        if cache.bytes <= RESIDENT_BYTES && cache.ready.len() <= MAX_RESIDENT {
            return;
        }
        let mut oldest = cache
            .ready
            .iter()
            .filter(|(path, _)| !protected.iter().any(|slot| slot.path == **path))
            .map(|(path, entry)| (path.clone(), entry.last_used))
            .collect::<Vec<_>>();
        oldest.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (path, _) in oldest {
            if cache.bytes <= RESIDENT_BYTES && cache.ready.len() <= MAX_RESIDENT {
                break;
            }
            if let Some(entry) = cache.ready.remove(&path) {
                cache.bytes -= entry.texture.byte_size();
                if cache.evicted.len() == MAX_RESIDENT * 4 {
                    cache.evicted.pop_front();
                }
                cache.evicted.push_back(path);
            }
        }
    }
}

struct Interest<'a> {
    art: &'a Artwork,
    source: &'a Source,
    paths: Vec<Slot>,
}

struct Flight {
    art: Artwork,
    path: String,
    rx: Receiver<Option<(egui::ColorImage, bool)>>,
}

impl Flight {
    fn matches(&self, view: &Interest<'_>, slot: &Slot) -> bool {
        self.path == slot.path && Rc::ptr_eq(&self.art.0, &view.art.0)
    }
}

#[derive(Default)]
pub(super) struct Scheduler {
    flights: Vec<Flight>,
    motion: Option<(bool, f64)>,
}

impl Scheduler {
    pub(super) fn frame<'a>(
        &mut self,
        ctx: &egui::Context,
        sources: impl Iterator<Item = (&'a Artwork, &'a Source)>,
    ) {
        let now = ctx.input(|i| i.time);
        let views: Vec<_> = sources
            .filter_map(|(art, source)| {
                // The book dock, active view and overview may share a cache.
                let paths = std::mem::take(&mut art.0.borrow_mut().wanted);
                (!paths.is_empty()).then_some(Interest { art, source, paths })
            })
            .collect();

        if ctx.will_discard() || (self.flights.is_empty() && views.is_empty()) {
            return;
        }
        let (mut animate, checked) = self.motion.unwrap_or((true, f64::NEG_INFINITY));
        if now - checked >= 1.0 {
            animate = prefs::client_animations_enabled();
            self.motion = Some((animate, now));
        }
        let mut ordered: Vec<_> = views
            .iter()
            .flat_map(|view| view.paths.iter().map(move |slot| (view, slot)))
            .collect();
        ordered.sort_by(|(_, a), (_, b)| {
            b.visible
                .cmp(&a.visible)
                .then_with(|| a.behind.cmp(&b.behind))
                .then_with(|| a.pos.y.total_cmp(&b.pos.y))
                .then_with(|| a.pos.x.total_cmp(&b.pos.x))
        });
        let uploads = self.collect_ready(ctx, &ordered, animate, now);
        for view in &views {
            let mut cache = view.art.0.borrow_mut();
            cache.failed_until.retain(|path, until| {
                *until + RETRY_AFTER > now || view.paths.iter().any(|slot| &slot.path == path)
            });
            for slot in &view.paths {
                if slot.visible
                    && let Some(entry) = cache.ready.get_mut(&slot.path)
                {
                    if !animate {
                        entry.reveal = f64::NEG_INFINITY;
                    } else if now - entry.reveal < FADE {
                        ctx.request_repaint();
                    }
                }
            }
            drop(cache);
            if uploads > 0 {
                view.art.trim(&view.paths);
            }
        }
        for (view, slot) in ordered {
            // Reserve a worker for newly visible cards while prefetching.
            if self.flights.len() >= MAX_JOBS - usize::from(!slot.visible) {
                break;
            }
            let cache = view.art.0.borrow();
            if cache.ready.contains_key(&slot.path)
                || cache
                    .failed_until
                    .get(&slot.path)
                    .is_some_and(|until| *until > now)
                || self.flights.iter().any(|job| job.matches(view, slot))
            {
                continue;
            }
            self.launch(ctx, view, &slot.path);
        }
    }

    fn collect_ready(
        &mut self,
        ctx: &egui::Context,
        ordered: &[(&Interest<'_>, &Slot)],
        animate: bool,
        now: f64,
    ) -> usize {
        let rank = |job: &Flight| {
            ordered
                .iter()
                .position(|(view, slot)| job.matches(view, slot))
        };
        self.flights
            .sort_by_key(|job| rank(job).unwrap_or(usize::MAX));
        let start = Instant::now();
        let mut uploads = 0;
        let mut reveals: usize = 0;
        let mut i = 0;
        while i < self.flights.len() {
            if uploads > 0 && (uploads == MAX_UPLOADS || start.elapsed() >= UPLOAD_BUDGET) {
                ctx.request_repaint();
                break;
            }
            let result = match self.flights[i].rx.try_recv() {
                Ok(image) => image,
                Err(TryRecvError::Disconnected) => None,
                Err(TryRecvError::Empty) => {
                    i += 1;
                    continue;
                }
            };
            let job = self.flights.remove(i);
            let Some(index) = rank(&job) else {
                // The request completed after this source or slot left the
                // viewport and its prefetch margin. Leave its bytes on disk.
                continue;
            };
            if let Some((image, cached)) = result {
                let visible = ordered[index].1.visible;
                let reveal = if animate && visible && !cached {
                    let start = reveal_start(now, reveals);
                    reveals += 1;
                    start
                } else {
                    f64::NEG_INFINITY
                };
                let texture = ctx.load_texture(&job.path, image, egui::TextureOptions::LINEAR);
                job.art.store(job.path, texture, reveal);
                uploads += 1;
            } else {
                job.art
                    .0
                    .borrow_mut()
                    .failed_until
                    .insert(job.path, now + RETRY_AFTER);
            }
            ctx.request_repaint();
        }
        uploads
    }

    fn launch(&mut self, ctx: &egui::Context, view: &Interest<'_>, path: &str) {
        let (tx, rx) = mpsc::channel();
        self.flights.push(Flight {
            art: view.art.clone(),
            path: path.to_owned(),
            rx,
        });
        let (source, path, ctx) = (view.source.clone(), path.to_owned(), ctx.clone());
        tasks::spawn(move || {
            let image = source
                .cached_artwork(&path)
                .map(|image| (image, true))
                .or_else(|| source.artwork(&path).ok().map(|image| (image, false)));
            let image = image.map(|(rgba, cached)| {
                let size = [rgba.width() as usize, rgba.height() as usize];
                (
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                    cached,
                )
            });
            let _ = tx.send(image);
            ctx.request_repaint();
        });
    }
}

fn reveal_start(now: f64, batch_index: usize) -> f64 {
    now + batch_index.min(3) as f64 * 0.016
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_finishes_once_and_warm_art_stays_opaque() {
        let ctx = egui::Context::default();
        let art = Artwork::default();
        let texture = ctx.load_texture(
            "art",
            egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
            Default::default(),
        );
        let id = texture.id();
        let last = reveal_start(1.0, 99);
        assert_eq!(last, 1.048);
        art.store("same".into(), texture.clone(), last);
        art.0.borrow_mut().now = last + FADE / 2.0;
        assert!((art.get("same").unwrap().1 - 0.5).abs() < 0.001);
        art.0.borrow_mut().now = 2.0;
        art.store("same".into(), texture.clone(), 2.0);
        let (resident, opacity) = art.get("same").unwrap();
        assert_eq!(resident.id(), id);
        assert_eq!(opacity, 1.0);
        art.insert("cached".into(), texture);
        assert_eq!(art.get("cached").unwrap().1, 1.0);
        assert!(Artwork::default().get("same").is_none());
    }

    #[test]
    fn landscape_card_keeps_warm_poster_under_the_backdrop_until_opaque() {
        let ctx = egui::Context::default();
        let art = Artwork::default();
        let movie = crate::Item {
            kind: crate::Kind::Movie,
            thumb: Some("poster".into()),
            art: Some("backdrop".into()),
            ..Default::default()
        };
        let paint = |time: f64| {
            let output = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        crate::cover_view(ui, &movie, &art, egui::vec2(120.0, 68.0), false);
                    });
                },
            );
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Rect(rect) => {
                        rect.brush.map(|b| (b.fill_texture_id, rect.fill.a()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(paint(0.0).is_empty());
        assert_eq!(art.0.borrow().wanted.len(), 1);
        assert_eq!(art.0.borrow().wanted[0].path, "backdrop");
        let image = egui::ColorImage::filled([1, 1], egui::Color32::WHITE);
        let poster = ctx.load_texture("poster", image.clone(), Default::default());
        art.insert("poster".into(), poster.clone());
        assert_eq!(paint(1.0), [(poster.id(), 255)]);
        let backdrop = ctx.load_texture("backdrop", image, Default::default());
        art.store("backdrop".into(), backdrop.clone(), 2.0);
        let layers = paint(2.0 + FADE / 2.0);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0], (poster.id(), 255));
        assert_eq!(layers[1].0, backdrop.id());
        assert!((126..=129).contains(&layers[1].1));
        assert_eq!(paint(3.0), [(backdrop.id(), 255)]);
    }

    #[test]
    fn eviction_preserves_visible_previews_and_remembers_revisited_art() {
        let art = Artwork::default();
        let texture = egui::Context::default().load_texture(
            "tiny",
            egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
            Default::default(),
        );
        for index in 0..=MAX_RESIDENT {
            art.0.borrow_mut().now = index as f64;
            art.insert(index.to_string(), texture.clone());
        }
        art.trim(&[Slot {
            path: "0".into(),
            visible: true,
            behind: false,
            pos: egui::Pos2::ZERO,
        }]);
        assert!(art.contains("0"));
        assert!(!art.contains("1"));
        let now = art.0.borrow().now;
        art.store("1".into(), texture, now);
        assert_eq!(art.get("1").unwrap().1, 1.0);
    }
}
