use super::*;
use perfuse::media::{Book, Bookmark, Chapter};

mod transport;
mod volume;

#[derive(Default)]
pub(crate) struct BookPlayer {
    pub completed: bool,
    loaded: Option<LoadedBook>,
    pub full: bool,
    pub start_at_ms: Option<u64>,
    show_bookmarks: bool,
    sleep: Option<Sleep>,
    scrub_range: Option<(f64, f64)>,
    pending: Option<PendingBookmark>,
    toast: Option<(String, f64)>,
}

struct LoadedBook {
    item: Arc<Item>,
    source: Source,
    server_name: String,
    covers: Artwork,
    cover_rx: Option<Receiver<egui::ColorImage>>,
}

#[derive(Clone, Copy)]
enum Sleep {
    Minutes {
        minutes: u64,
        remaining: Duration,
        sampled: Instant,
    },
    Chapter,
}

impl Sleep {
    fn minutes(minutes: u64) -> Self {
        Self::Minutes {
            minutes,
            remaining: Duration::from_secs(minutes * 60),
            sampled: Instant::now(),
        }
    }

    fn expired(&mut self, now: Instant, paused: bool) -> bool {
        match self {
            Self::Minutes {
                remaining, sampled, ..
            } => {
                if !paused {
                    *remaining = remaining.saturating_sub(now.saturating_duration_since(*sampled));
                }
                *sampled = now;
                remaining.is_zero()
            }
            // The worker follows the selected chapter across file changes and seeks.
            Self::Chapter => false,
        }
    }
}

#[cfg(test)]
#[test]
fn minute_timer_counts_listening_time_across_a_pause() {
    let start = Instant::now();
    let mut timer = Sleep::Minutes {
        minutes: 15,
        remaining: Duration::from_secs(900),
        sampled: start,
    };
    for (seconds, paused, expired) in [
        (300, false, false),
        (1200, true, false),
        (1799, false, false),
        (1800, false, true),
    ] {
        assert_eq!(
            timer.expired(start + Duration::from_secs(seconds), paused),
            expired
        );
    }
}

enum BookmarkEdit {
    Add(Bookmark),
    Remove(f64),
}

struct PendingBookmark {
    source: Source,
    id: String,
    reply: Receiver<Result<BookmarkEdit, String>>,
}

impl BookPlayer {
    pub fn is_loaded(&self) -> bool {
        self.loaded.is_some()
    }

    pub fn uses_source(&self, source: &Source) -> bool {
        self.loaded
            .as_ref()
            .is_some_and(|b| Arc::ptr_eq(&b.source, source))
    }

    pub fn is_current(&self, source: &Source, id: &str) -> bool {
        self.loaded
            .as_ref()
            .is_some_and(|b| b.item.id == id && Arc::ptr_eq(&b.source, source))
    }
}

fn chapter_index(chapters: &[Chapter], seconds: f64) -> Option<usize> {
    let position_ms = (seconds * 1000.0).round() as u64;
    chapters
        .partition_point(|c| c.start_ms <= position_ms)
        .checked_sub(1)
        .filter(|&i| position_ms < chapters[i].end_ms)
}

fn byline(book: &Book) -> String {
    [
        (!book.author.is_empty()).then(|| format!("By {}", book.author)),
        (!book.narrator.is_empty()).then(|| format!("Narrated by {}", book.narrator)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

pub(super) fn book_duration(seconds: f64) -> String {
    let minutes = (seconds.max(0.0) / 60.0).round() as u64;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// Contents of the shared inline breadcrumb on book details and the full player.
pub(super) fn book_crumb(ui: &mut egui::Ui, server: &str, library: Option<&str>) {
    ui.spacing_mut().item_spacing.x = 8.0;
    ui.add(Icon::ServerAudiobookshelf.image(16.0, MUTED));
    ui.label(RichText::new("Audiobookshelf").size(13.0).color(MUTED));
    for part in [Some(server), library].into_iter().flatten() {
        ui.label(
            RichText::new("/")
                .size(13.0)
                .color(Color32::from_rgb(110, 110, 117)),
        );
        ui.add(egui::Label::new(RichText::new(part).size(13.0).color(MUTED)).truncate());
    }
}

fn book_panel() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .corner_radius(0)
        .inner_margin(24)
}

impl App {
    pub fn book_started(&mut self, item: &Item, source: &Source) {
        self.book.completed = false;
        let mut covers = Artwork::default();
        let mut cover_rx = None;
        let server = self.servers.iter().find(|s| Arc::ptr_eq(&s.source, source));
        if let Some(path) = &item.thumb {
            let texture = self
                .covers
                .get(path)
                .or_else(|| server.and_then(|s| s.artwork.get(path)));
            if let Some((texture, _)) = texture {
                covers.insert(path.clone(), texture.clone());
            } else {
                // A browse generation can change while its cover request is in flight.
                // Give the dock its own source-scoped copy for the lifetime of the book.
                let (tx, rx) = mpsc::channel();
                let (source, path) = (source.clone(), path.clone());
                thread::spawn(move || {
                    if let Some(image) = source
                        .cached_artwork(&path)
                        .or_else(|| source.artwork(&path).ok())
                    {
                        let size = [image.width() as usize, image.height() as usize];
                        let _ = tx.send(egui::ColorImage::from_rgba_unmultiplied(
                            size,
                            image.as_raw(),
                        ));
                    }
                });
                cover_rx = Some(rx);
            }
        }
        self.book.loaded = Some(LoadedBook {
            item: Arc::new(item.clone()),
            source: source.clone(),
            server_name: server
                .map(Server::name)
                .unwrap_or_else(|| "Audiobookshelf".into()),
            covers,
            cover_rx,
        });
        self.book.full = false;
        self.book.show_bookmarks = false;
        self.book.sleep = None;
        self.book.scrub_range = None;
    }

    pub fn book_completed(&mut self, ctx: &egui::Context) {
        self.book.completed = true;
        self.book.sleep = None;
        self.book.scrub_range = None;
        self.playing_seconds = 0.0;
        self.seek_position = None;
        self.buffered = 0.0;
        self.paused = true;
        self.book_toast(ctx, format!("Finished {}", self.playing_title));
    }

    pub fn book_chapter_sleep_expired(&mut self, ctx: &egui::Context, position: f64) {
        self.paused = true;
        self.playing_seconds = position;
        if matches!(self.book.sleep, Some(Sleep::Chapter)) {
            self.book.sleep = None;
            self.book_toast(ctx, "Paused at the end of the chapter.");
        }
    }

    pub fn book_ended(&mut self, ctx: &egui::Context) {
        // Audiobookshelf decides when a book is finished. Refresh only its visible
        // detail, preserving navigation and requests started while the dock was open.
        let refresh = self
            .book
            .loaded
            .as_ref()
            .filter(|loaded| {
                !self.busy
                    && !self.overview
                    && !self.connection_open
                    && self
                        .source
                        .as_ref()
                        .is_some_and(|s| Arc::ptr_eq(s, &loaded.source))
                    && self.detail.as_ref().is_some_and(|d| d.id == loaded.item.id)
            })
            .map(|loaded| loaded.item.clone());
        self.book_finished();
        if let Some(item) = refresh {
            self.open_item(&item, ctx);
        }
    }

    pub fn book_finished(&mut self) {
        self.book.completed = false;
        self.book.loaded = None;
        self.book.full = false;
        self.book.sleep = None;
        self.book.scrub_range = None;
    }

    fn book_toast(&mut self, ctx: &egui::Context, message: impl Into<String>) {
        self.book.toast = Some((message.into(), ctx.input(|i| i.time) + 2.8));
        ctx.request_repaint();
    }

    pub(super) fn show_book_toast(&mut self, ctx: &egui::Context) {
        let Some((text, until)) = &self.book.toast else {
            return;
        };
        let remaining = until - ctx.input(|i| i.time);
        if remaining <= 0.0 {
            self.book.toast = None;
            return;
        }
        egui::Area::new(egui::Id::new("audiobook_toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -112.0])
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(CONTROL)
                    .corner_radius(0)
                    .inner_margin(egui::Margin::symmetric(18, 10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 12],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(128),
                    })
                    .show(ui, |ui| {
                        let label = ui.label(
                            RichText::new(text)
                                .size(13.0)
                                .line_height(Some(19.5))
                                .color(TEXT),
                        );
                        ctx.accesskit_node_builder(label.id, |node| {
                            node.set_live(egui::accesskit::Live::Polite)
                        });
                    });
            });
        ctx.request_repaint_after(Duration::from_secs_f64(remaining));
    }

    pub fn book_tick(&mut self, ctx: &egui::Context) {
        if let Some(loaded) = self.book.loaded.as_mut()
            && let Some(rx) = &loaded.cover_rx
        {
            match rx.try_recv() {
                Ok(image) => {
                    if let Some(path) = &loaded.item.thumb {
                        let texture = ctx.load_texture(path, image, egui::TextureOptions::LINEAR);
                        loaded.covers.insert(path.clone(), texture);
                    }
                    loaded.cover_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => loaded.cover_rx = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let expired = self
            .book
            .sleep
            .as_mut()
            .is_some_and(|sleep| sleep.expired(Instant::now(), self.paused));
        if expired && let Some(player) = &self.player {
            player.set_paused(true);
            self.book_toast(ctx, "Sleep timer ended. Playback paused.");
            self.book.sleep = None;
        }
        let Some(pending) = &self.book.pending else {
            return;
        };
        let result = match pending.reply.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(200));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Audiobookshelf could not confirm the bookmark change.".into())
            }
        };
        match result {
            Ok(edit) => {
                let apply = |item: &mut Item| {
                    if let Some(book) = item.book.as_mut() {
                        let time = match &edit {
                            BookmarkEdit::Add(mark) => mark.time,
                            BookmarkEdit::Remove(time) => *time,
                        };
                        book.bookmarks.retain(|m| m.time != time);
                        if let BookmarkEdit::Add(mark) = &edit {
                            book.bookmarks.push(mark.clone());
                            book.bookmarks.sort_by(|a, b| a.time.total_cmp(&b.time));
                        }
                    }
                };
                if let Some(loaded) = &mut self.book.loaded
                    && loaded.item.id == pending.id
                    && Arc::ptr_eq(&loaded.source, &pending.source)
                {
                    apply(Arc::make_mut(&mut loaded.item));
                    if matches!(edit, BookmarkEdit::Add(_)) {
                        self.book.show_bookmarks = true;
                    }
                }
                if let (Some(detail), Some(source)) = (&mut self.detail, &self.source)
                    && detail.id == pending.id
                    && Arc::ptr_eq(source, &pending.source)
                {
                    apply(detail);
                }
                self.book_toast(
                    ctx,
                    match edit {
                        BookmarkEdit::Add(mark) => {
                            format!("Bookmark added at {}", transport::short_clock(mark.time))
                        }
                        BookmarkEdit::Remove(_) => "Bookmark deleted".into(),
                    },
                );
            }
            Err(error) => self.error = Some(error),
        }
        self.book.pending = None;
    }

    fn book_edit_mark(&mut self, item: &Item, source: &Source, edit: BookmarkEdit) {
        if self.book.pending.is_some() {
            return;
        }
        let (tx, reply) = mpsc::channel();
        let (id, source) = (item.id.clone(), source.clone());
        self.book.pending = Some(PendingBookmark {
            source: source.clone(),
            id: id.clone(),
            reply,
        });
        thread::spawn(move || {
            let result = match edit {
                BookmarkEdit::Add(mark) => source.add_bookmark(&id, &mark).map(BookmarkEdit::Add),
                BookmarkEdit::Remove(time) => source
                    .remove_bookmark(&id, time)
                    .map(|()| BookmarkEdit::Remove(time)),
            };
            let _ = tx.send(result);
        });
    }

    fn book_add_mark(&mut self, item: &Item, source: &Source) {
        let time = self.playing_seconds.floor().max(0.0);
        let title = item
            .book
            .as_ref()
            .and_then(|book| chapter_index(&book.chapters, time).map(|i| &book.chapters[i]))
            .map_or_else(
                || format!("{} · {}", item.title, transport::short_clock(time)),
                |c| {
                    format!(
                        "{} · {}",
                        c.title,
                        transport::short_clock(time - c.start_ms as f64 / 1000.0)
                    )
                },
            );
        self.book_edit_mark(item, source, BookmarkEdit::Add(Bookmark { title, time }));
    }

    fn book_jump(&mut self, seconds: f64) {
        if let Some(player) = &self.player {
            player.seek(seconds);
            player.set_paused(false);
        }
    }

    pub fn book_details(&mut self, ui: &mut egui::Ui, item: &Item) -> bool {
        let Some(source) = self.source.clone() else {
            return false;
        };
        let Some(book) = &item.book else {
            ui.heading(&item.title);
            ui.label("No audiobook details are available.");
            return false;
        };
        let current = self.book.is_current(&source, &item.id);
        let position = if current {
            self.playing_seconds
        } else {
            item.resume_ms.unwrap_or(0) as f64 / 1000.0
        };
        let playable = !item.versions.is_empty();
        let finished = if current {
            self.book.completed
        } else {
            item.watched
        };
        let in_progress = position > 0.0 && !finished;
        let server = self
            .servers
            .iter()
            .find(|s| Arc::ptr_eq(&s.source, &source));
        let server_name = server.map(Server::name);
        let library = server
            .and_then(|s| {
                s.sections
                    .iter()
                    .find(|l| Some(&l.id) == item.library_id.as_ref())
            })
            .map(|l| l.title.clone());
        let mut play = false;
        let mut jump = None;
        egui::ScrollArea::vertical()
            .id_salt(("book_detail", &item.id))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let viewport = ui.ctx().content_rect().width();
                ui.spacing_mut().item_spacing.y = 0.0;
                if let Some(server_name) = &server_name {
                    ui.horizontal(|ui| book_crumb(ui, server_name, library.as_deref()));
                    ui.add_space(24.0);
                }
                book_panel().inner_margin(28).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let compact = viewport <= 1000.0;
                    let art_size = if compact { 140.0 } else { 200.0 };
                    ui.spacing_mut().item_spacing.x = if compact { 24.0 } else { 32.0 };
                    ui.horizontal_top(|ui| {
                        cover_view(ui, item, &self.covers, Vec2::splat(art_size), false);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 8.0;
                            ui.label(
                                RichText::new("AUDIOBOOK")
                                    .color(ACCENT)
                                    .size(12.0)
                                    .extra_letter_spacing(0.72),
                            );
                            ui.label(RichText::new(&item.title).size(if compact {
                                30.0
                            } else {
                                36.0
                            }));
                            let credits = byline(book);
                            if !credits.is_empty() {
                                ui.label(RichText::new(credits).size(16.0));
                            }
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(16.0, 6.0);
                                for value in [
                                    (!book.series.is_empty()).then_some(book.series.clone()),
                                    item.year.map(|year| year.to_string()),
                                    item.duration_ms.map(|ms| book_duration(ms as f64 / 1000.0)),
                                    (!book.chapters.is_empty())
                                        .then(|| format!("{} chapters", book.chapters.len())),
                                ]
                                .into_iter()
                                .flatten()
                                {
                                    ui.label(RichText::new(value).size(13.0).color(MUTED));
                                }
                            });
                            if !item.summary.is_empty() {
                                ui.add_space(8.0);
                                ui.set_max_width(ui.available_width().min(580.0));
                                ui.label(
                                    RichText::new(&item.summary)
                                        .size(15.0)
                                        .line_height(Some(22.5)),
                                );
                            }
                        });
                    });
                });
                ui.add_space(32.0);
                let progress_width = ui.available_width().min(320.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(12.0);
                    if !playable {
                        ui.colored_label(MUTED, "No audio file is available for this book.");
                        return;
                    }
                    let (icon, label) = if current && !self.paused {
                        (Icon::Pause, "Pause")
                    } else if in_progress {
                        (Icon::Resume, "Resume")
                    } else {
                        (Icon::Play, if finished { "Listen again" } else { "Play" })
                    };
                    let action = |ui: &mut egui::Ui, mut pill: Pill<'_>| {
                        // The detail actions use the prototype's 44-point pills with 24-point padding.
                        pill.tall = true;
                        let width = pill.galley(ui).size().x + Pill::ICON + Pill::GAP + 48.0;
                        ui.add(pill.min_width(width))
                    };
                    if action(ui, Pill::primary(icon, label)).clicked() {
                        if current && let Some(player) = &self.player {
                            player.pause();
                        } else {
                            self.book.start_at_ms = None;
                            self.resume = in_progress;
                            play = true;
                        }
                    }
                    if in_progress
                        && action(
                            ui,
                            Pill::secondary(Icon::PlayFromBeginning, "Play from beginning"),
                        )
                        .clicked()
                    {
                        if current {
                            self.book_jump(0.0);
                        } else {
                            self.book.start_at_ms = None;
                            self.resume = false;
                            play = true;
                        }
                    }
                    if current
                        && action(ui, Pill::secondary(Icon::Headphones, "Open player")).clicked()
                    {
                        self.book.full = true;
                    }
                    if in_progress && let Some(duration) = item.duration_ms.filter(|&ms| ms > 0) {
                        ui.allocate_ui_with_layout(
                            Vec2::new(progress_width, 28.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                let (track, _) = ui.allocate_exact_size(
                                    Vec2::new(progress_width, 4.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(track, 2.0, Color32::from_gray(65));
                                let fraction =
                                    (position * 1000.0 / duration as f64).clamp(0.0, 1.0) as f32;
                                let filled = egui::Rect::from_min_size(
                                    track.min,
                                    Vec2::new(track.width() * fraction, 4.0),
                                );
                                ui.painter().rect_filled(filled, 2.0, ACCENT);
                                let chapter = chapter_index(&book.chapters, position)
                                    .map_or(String::new(), |i| {
                                        format!("Chapter {} of {} · ", i + 1, book.chapters.len())
                                    });
                                let left = book_duration(duration as f64 / 1000.0 - position);
                                ui.label(
                                    RichText::new(format!("{chapter}{left} left"))
                                        .size(12.0)
                                        .color(MUTED),
                                );
                            },
                        );
                    } else if finished {
                        ui.label(RichText::new("Finished").size(12.0).color(MUTED));
                    }
                });
                ui.add_space(32.0);
                let chapters = |ui: &mut egui::Ui, app: &mut App| {
                    book_panel()
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 12.0;
                            ui.label(RichText::new("Chapters").size(18.0));
                            app.book_lists(ui, item, &source, current, false)
                        })
                        .inner
                };
                if viewport > 1180.0 {
                    let left = ui.available_width() - 320.0 - 32.0;
                    ui.spacing_mut().item_spacing.x = 32.0;
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(left);
                            jump = chapters(ui, self);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(320.0);
                            jump = self.book_side_details(ui, item, &source, current).or(jump);
                        });
                    });
                } else {
                    jump = chapters(ui, self);
                    ui.add_space(32.0);
                    jump = self.book_side_details(ui, item, &source, current).or(jump);
                }
            });
        if let Some(seconds) = jump
            && playable
        {
            if current {
                self.book_jump(seconds);
            } else {
                self.book.start_at_ms = Some((seconds * 1000.0).round() as u64);
                self.resume = true;
                play = true;
            }
        }
        play
    }

    fn book_side_details(
        &mut self,
        ui: &mut egui::Ui,
        item: &Item,
        source: &Source,
        current: bool,
    ) -> Option<f64> {
        let jump = book_panel()
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.label(RichText::new("Bookmarks").size(18.0));
                self.book_lists(ui, item, source, current, true)
            })
            .inner;
        ui.add_space(16.0);
        book_panel().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(RichText::new("Details").size(18.0));
            let value_width = ui.available_width() - 156.0;
            egui::Grid::new(("book_metadata", &item.id))
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, key: &str, value: &str| {
                        ui.add_sized(
                            [140.0, 20.0],
                            egui::Label::new(RichText::new(key).size(13.0).color(MUTED))
                                .halign(egui::Align::Min),
                        );
                        ui.vertical(|ui| {
                            ui.set_width(value_width);
                            ui.label(RichText::new(value).size(13.0));
                        });
                        ui.end_row();
                    };
                    if let Some(server) =
                        self.servers.iter().find(|s| Arc::ptr_eq(&s.source, source))
                    {
                        row(ui, "Server", &format!("Audiobookshelf · {}", server.name()));
                        if let Some(library) = server
                            .sections
                            .iter()
                            .find(|l| Some(&l.id) == item.library_id.as_ref())
                        {
                            row(ui, "Library", &library.title);
                        }
                    }
                    if let Some(book) = &item.book
                        && !book.narrator.is_empty()
                    {
                        row(ui, "Narrator", &book.narrator);
                    }
                    if let Some(year) = item.year {
                        row(ui, "Released", &year.to_string());
                    }
                    if let Some(duration) = item.duration_ms {
                        row(
                            ui,
                            "Length",
                            &transport::short_clock(duration as f64 / 1000.0),
                        );
                    }
                });
        });
        jump
    }

    fn book_lists(
        &mut self,
        ui: &mut egui::Ui,
        item: &Item,
        source: &Source,
        current: bool,
        bookmarks: bool,
    ) -> Option<f64> {
        let book = item.book.as_ref()?;
        let mut jump = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        if bookmarks {
            if book.bookmarks.is_empty() {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 16))
                    .show(ui, |ui| {
                        ui.label(RichText::new("No bookmarks yet. Add one while listening to save your place with a note.")
                            .size(13.0).line_height(Some(19.5)).color(MUTED));
                    });
            }
            for (index, mark) in book.bookmarks.iter().enumerate() {
                ui.push_id(mark.time.to_bits(), |ui| {
                    if index > 0 {
                        let (line, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 1.0),
                            egui::Sense::hover(),
                        );
                        ui.painter()
                            .rect_filled(line, 0, Color32::from_rgb(42, 42, 46));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (icon, _) =
                            ui.allocate_exact_size(Vec2::new(20.0, 42.5), egui::Sense::hover());
                        Icon::Bookmark.paint(ui, icon.center(), 18.0, ACCENT);
                        let mut caption = transport::short_clock(mark.time);
                        if let Some(i) = chapter_index(&book.chapters, mark.time) {
                            caption.push_str(" · ");
                            caption.push_str(&book.chapters[i].title);
                        }
                        let width = (ui.available_width() - 28.0 - 12.0).max(1.0);
                        let jump_button = ui
                            .add_enabled_ui(!item.versions.is_empty(), |ui| {
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(width, 42.5),
                                    egui::Sense::click(),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        ui.is_enabled(),
                                        format!("{} · {caption}", mark.title),
                                    )
                                });
                                draw_list_text(
                                    ui,
                                    &mark.title,
                                    rect.left_top() + Vec2::new(0.0, 11.25),
                                    width,
                                    15.0,
                                    TEXT,
                                );
                                draw_list_text(
                                    ui,
                                    &caption,
                                    rect.left_top() + Vec2::new(0.0, 33.5),
                                    width,
                                    12.0,
                                    MUTED,
                                );
                                if response.has_focus() {
                                    ui.painter().rect_stroke(
                                        rect,
                                        2,
                                        egui::Stroke::new(2.0_f32, ACCENT),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                                response.on_hover_cursor(egui::CursorIcon::PointingHand)
                            })
                            .inner;
                        if jump_button.clicked() {
                            jump = Some(mark.time);
                        }
                        let label = format!("Delete bookmark {}", mark.title);
                        ui.spacing_mut().button_padding = Vec2::ZERO;
                        let delete = ui.add_enabled(
                            self.book.pending.is_none(),
                            egui::Button::image(Icon::Close.image(14.0, MUTED).alt_text(&label))
                                .min_size(Vec2::splat(28.0))
                                .corner_radius(EDGE)
                                .frame_when_inactive(false),
                        );
                        delete.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                delete.enabled(),
                                &label,
                            )
                        });
                        if delete.on_hover_text(label).clicked() {
                            self.book_edit_mark(item, source, BookmarkEdit::Remove(mark.time));
                        }
                    });
                    ui.add_space(10.0);
                });
            }
        } else {
            if book.chapters.is_empty() {
                ui.colored_label(MUTED, "No chapters available.");
            }
            let position = if current {
                self.playing_seconds
            } else {
                item.resume_ms.unwrap_or(0) as f64 / 1000.0
            };
            let index = chapter_index(&book.chapters, position);
            let finished = if current {
                self.book.completed
            } else {
                item.watched
            };
            for (n, chapter) in book.chapters.iter().enumerate() {
                let selected = (current || position > 0.0) && !finished && index == Some(n);
                let done = finished || chapter.end_ms as f64 / 1000.0 <= position;
                let row = ui.add_enabled_ui(!item.versions.is_empty(), |ui| {
                    chapter_row(ui, chapter, n, done, selected, position)
                });
                if row.inner.clicked() {
                    jump = Some(chapter.start_ms as f64 / 1000.0);
                }
            }
        }
        jump
    }
}

/// Draw a single ellipsized line from its left edge and vertical center.
fn draw_list_text(
    ui: &egui::Ui,
    text: &str,
    at: egui::Pos2,
    width: f32,
    size: f32,
    color: Color32,
) {
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width,
        egui::FontId::proportional(size),
    );
    ui.painter()
        .galley(at - Vec2::new(0.0, galley.size().y / 2.0), galley, color);
}

fn chapter_row(
    ui: &mut egui::Ui,
    chapter: &Chapter,
    index: usize,
    done: bool,
    current: bool,
    position: f64,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            current,
            &chapter.title,
        )
    });
    if ui.is_rect_visible(rect) {
        let fill = if current {
            Color32::from_rgb(23, 42, 64)
        } else if response.hovered() {
            Color32::from_rgb(38, 38, 42)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, EDGE, fill);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                EDGE,
                egui::Stroke::new(2.0_f32, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let center = egui::pos2(rect.left() + 12.0 + 16.0, rect.center().y);
        let font = egui::FontId::proportional(13.0);
        if done {
            Icon::Check.paint(ui, center, 16.0, GREEN);
        } else if current {
            Icon::NowPlaying.paint(ui, center, 16.0, ACCENT);
        } else {
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                index + 1,
                font.clone(),
                MUTED,
            );
        }
        let length = chapter.end_ms.saturating_sub(chapter.start_ms) as f64 / 1000.0;
        let duration = ui.painter().text(
            egui::pos2(rect.right() - 12.0, center.y),
            egui::Align2::RIGHT_CENTER,
            transport::short_clock(length),
            font,
            MUTED,
        );
        let text_x = center.x + 16.0 + 12.0;
        let width = (duration.left() - 12.0 - text_x).max(0.0);
        let ink = if done { MUTED } else { TEXT };
        let text_y = if current { center.y - 4.5 } else { center.y };
        draw_list_text(
            ui,
            &chapter.title,
            egui::pos2(text_x, text_y),
            width,
            15.0,
            ink,
        );
        if current && width > 0.0 && length > 0.0 {
            let bar = egui::Rect::from_min_size(
                egui::pos2(text_x, center.y + 12.75),
                Vec2::new(width, 3.0),
            );
            ui.painter().rect_filled(bar, 2, Color32::from_gray(65));
            let ratio =
                ((position - chapter.start_ms as f64 / 1000.0) / length).clamp(0.0, 1.0) as f32;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(bar.min, Vec2::new(width * ratio, 3.0)),
                2,
                ACCENT,
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
