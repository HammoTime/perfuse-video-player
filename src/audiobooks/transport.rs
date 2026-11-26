use super::*;

/// Only Play/Pause has a filled disc at rest.
pub(super) fn book_icon(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    size: f32,
    glyph: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let primary = matches!(icon, Icon::Play | Icon::Pause);
    let fill = if response.is_pointer_button_down_on() {
        CONTROL_PRESSED
    } else if response.hovered() && primary {
        Color32::WHITE
    } else if response.hovered() {
        CONTROL_HOVER
    } else if primary {
        TEXT
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, EDGE, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            EDGE,
            (2.0, ACCENT),
            egui::StrokeKind::Middle,
        );
    }
    icon.paint(
        ui,
        rect.center(),
        glyph,
        if primary {
            Color32::from_rgb(13, 13, 16)
        } else {
            TEXT
        },
    );
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl App {
    pub(crate) fn book_dock(&mut self, ctx: &egui::Context) {
        let Some(loaded) = self.book.loaded.as_ref() else {
            return;
        };
        if self.book.full {
            return;
        }
        let item = loaded.item.clone();
        if self.book.scrub_range.take().is_some() {
            self.seek_position = None;
        }
        let Some(book) = &item.book else { return };
        let current = chapter_index(&book.chapters, self.playing_seconds);
        let chapter = current.map(|i| &book.chapters[i]);
        let viewport = ctx.content_rect().width();
        let show_tools = viewport > 1000.0;
        let timeline = if !show_tools {
            240.0
        } else if viewport <= 1180.0 {
            300.0
        } else {
            420.0
        };
        egui::TopBottomPanel::bottom("audiobook_dock")
            .exact_height(88.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(23, 23, 26))
                    .inner_margin(egui::Margin {
                        left: 24,
                        right: 24,
                        top: 13,
                        bottom: 12,
                    }),
            )
            .show(ctx, |ui| {
                let bounds = ui.max_rect();
                ui.painter().hline(
                    (bounds.left() - 24.0)..=(bounds.right() + 24.0),
                    bounds.top() - 12.5,
                    (1.0, Color32::from_rgb(42, 42, 46)),
                );
                // The middle column is centred in the content pane; the last tool hugs its edge.
                // At intermediate widths, reserve room for the optional timer before placing the transport.
                let tools_width = if show_tools {
                    self.book_menus_width(ui, true) + 4.0 * (32.0 + 8.0)
                } else {
                    32.0
                };
                let tools_x = if show_tools {
                    bounds.right() - tools_width
                } else {
                    bounds.right() - timeline - 24.0 - tools_width
                };
                let timeline_x = if show_tools {
                    (bounds.center().x - timeline * 0.5).min(tools_x - 24.0 - timeline)
                } else {
                    bounds.right() - timeline
                };
                let now_right = timeline_x.min(tools_x) - 24.0;
                let now_rect = egui::Rect::from_center_size(
                    egui::pos2((bounds.left() + now_right) * 0.5, bounds.center().y),
                    Vec2::new((now_right - bounds.left()).max(0.0), 56.0),
                );
                let mut now = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("now")
                        .max_rect(now_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                now.set_clip_rect(now.clip_rect().intersect(now_rect.expand(4.0)));
                now.spacing_mut().item_spacing.x = 12.0;
                if let Some(loaded) = &self.book.loaded {
                    cover_view_rounded(
                        &mut now,
                        &item,
                        &loaded.covers,
                        Vec2::splat(56.0),
                        false,
                        0,
                    );
                }
                let text_width = now.available_width();
                now.allocate_ui_with_layout(
                    Vec2::new(text_width, if chapter.is_some() { 40.5 } else { 22.5 }),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(text_width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(&item.title)
                                    .size(15.0)
                                    .line_height(Some(22.5))
                                    .color(TEXT),
                            )
                            .truncate(),
                        );
                        if let Some(chapter) = chapter {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&chapter.title)
                                        .size(12.0)
                                        .line_height(Some(18.0))
                                        .color(MUTED),
                                )
                                .truncate(),
                            );
                        }
                    },
                );
                let now_response =
                    ui.interact(now_rect, ui.id().with("now_playing"), egui::Sense::click());
                now_response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Open player: {}", item.title),
                    )
                });
                if now_response.has_focus() {
                    ui.painter().rect_stroke(
                        now_rect.expand(3.0),
                        6,
                        (2.0, ACCENT),
                        egui::StrokeKind::Middle,
                    );
                }
                if now_response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.book.full = true;
                }
                let center_rect = egui::Rect::from_center_size(
                    egui::pos2(timeline_x + timeline * 0.5, bounds.center().y),
                    Vec2::new(timeline, 62.5),
                );
                let mut center = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("transport")
                        .max_rect(center_rect)
                        .layout(egui::Layout::top_down(egui::Align::Center)),
                );
                center.spacing_mut().item_spacing.y = 6.0;
                self.book_transport(&mut center, &book.chapters, current, true);
                self.book_scrubber(&mut center, chapter, true);
                let tools_rect = egui::Rect::from_min_size(
                    egui::pos2(tools_x, bounds.top()),
                    Vec2::new(tools_width, bounds.height()),
                );
                let mut tools = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("tools")
                        .max_rect(tools_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                tools.spacing_mut().item_spacing.x = 8.0;
                self.book_volume(&mut tools, true);
                if show_tools {
                    self.book_menus(&mut tools, chapter, true);
                    if book_icon(&mut tools, Icon::Chapters, "Chapters", 32.0, 16.0).clicked() {
                        self.book.show_bookmarks = false;
                        self.book.full = true;
                    }
                    if book_icon(&mut tools, Icon::PlayerExpand, "Open player", 32.0, 16.0)
                        .clicked()
                    {
                        self.book.full = true;
                    }
                    if book_icon(&mut tools, Icon::Close, "Stop", 32.0, 16.0).clicked()
                        && let Some(player) = &self.player
                    {
                        player.stop();
                    }
                }
            });
    }

    pub(crate) fn book_full_player(&mut self, ui: &mut egui::Ui) {
        let Some(active) = &self.book.loaded else {
            return;
        };
        let item = active.item.clone();
        let source = active.source.clone();
        let Some(book) = &item.book else { return };
        let library = self
            .servers
            .iter()
            .find(|server| Arc::ptr_eq(&server.source, &source))
            .and_then(|server| {
                server
                    .sections
                    .iter()
                    .find(|lib| Some(&lib.id) == item.library_id.as_ref())
            });
        let server_name = active.server_name.clone();
        let library_name = library.map(|lib| lib.title.clone());
        let current = chapter_index(&book.chapters, self.playing_seconds);
        let chapter = current.map(|i| &book.chapters[i]);
        let viewport = ui.ctx().content_rect().size();
        let split = viewport.x > 1180.0;
        let cover_size = if viewport.x <= 1000.0 { 160.0 } else { 240.0 };
        let width = ui.available_width();
        let main_column = if split { width - 360.0 - 40.0 } else { width };
        let main = |ui: &mut egui::Ui, app: &mut App| {
            ui.spacing_mut().item_spacing.y = 24.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if ui
                    .add(IconButton::new(Icon::PlayerCollapse, "Collapse player"))
                    .clicked()
                {
                    app.book.full = false;
                }
                book_crumb(ui, &server_name, library_name.as_deref());
            });
            // Align the bottom of the byline with the cover even when a title wraps.
            let text_width = (ui.available_width() - cover_size - 28.0).max(1.0);
            let mut lines = Vec::new();
            if let (Some(i), Some(chapter)) = (current, chapter) {
                lines.push(
                    RichText::new(format!("CHAPTER {} OF {}", i + 1, book.chapters.len()))
                        .size(12.0)
                        .line_height(Some(18.0))
                        .color(ACCENT),
                );
                lines.push(
                    RichText::new(&chapter.title)
                        .size(18.0)
                        .line_height(Some(27.0))
                        .color(ACCENT),
                );
            }
            lines.push(
                RichText::new(&item.title)
                    .size(30.0)
                    .line_height(Some(36.0)),
            );
            let credits = byline(book);
            if !credits.is_empty() {
                lines.push(
                    RichText::new(credits)
                        .size(15.0)
                        .line_height(Some(22.5))
                        .color(MUTED),
                );
            }
            let galleys: Vec<_> = lines
                .into_iter()
                .map(|line| {
                    egui::WidgetText::from(line).into_galley(
                        ui,
                        Some(egui::TextWrapMode::Wrap),
                        text_width,
                        egui::TextStyle::Body,
                    )
                })
                .collect();
            let text_height = galleys.iter().map(|line| line.size().y).sum::<f32>()
                + 8.0 * galleys.len().saturating_sub(1) as f32;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                if let Some(active) = &app.book.loaded {
                    cover_view_rounded(
                        ui,
                        &item,
                        &active.covers,
                        Vec2::splat(cover_size),
                        false,
                        0,
                    );
                }
                ui.vertical(|ui| {
                    ui.set_width(text_width);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.add_space((cover_size - text_height).max(0.0));
                    for line in galleys {
                        ui.label(line);
                    }
                });
            });
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                app.book_scrubber(ui, chapter, false);
            });
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                app.book_track(ui, &book.chapters, current);
            });
            app.book_transport(ui, &book.chapters, current, false);
            let menus = app.book_menus_width(ui, false);
            let mark_width = book_tool_width(ui, "Add bookmark");
            let volume_width = book_tool_width(ui, "Volume");
            let width = ui.available_width();
            let primary_width = menus + 12.0 + mark_width;
            let wrap_mark = primary_width > width;
            let wrap_volume = primary_width + 12.0 + volume_width > width;
            let button_group = if wrap_mark {
                menus
            } else if wrap_volume {
                primary_width
            } else {
                primary_width + 12.0 + volume_width
            };
            let bookmark = |ui: &mut egui::Ui, app: &mut App| {
                let mark = ui
                    .add_enabled_ui(app.book.pending.is_none(), |ui| {
                        book_tool(ui, Icon::BookmarkAdd, "Add bookmark", false)
                    })
                    .inner;
                if mark.clicked() {
                    app.book_add_mark(&item, &source);
                }
            };
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(((width - button_group) / 2.0).max(0.0));
                    app.book_menus(ui, chapter, false);
                    if !wrap_mark {
                        bookmark(ui, app);
                    }
                    if !wrap_volume {
                        app.book_volume(ui, false);
                    }
                });
                if wrap_volume {
                    ui.horizontal(|ui| {
                        let row = if wrap_mark {
                            mark_width + 12.0 + volume_width
                        } else {
                            volume_width
                        };
                        ui.add_space(((width - row) / 2.0).max(0.0));
                        if wrap_mark {
                            bookmark(ui, app);
                        }
                        app.book_volume(ui, false);
                    });
                }
            });
        };
        let side = |ui: &mut egui::Ui, app: &mut App| {
            let panel_width = if split { 360.0 } else { width };
            let panel_height = if split {
                (viewport.y - 256.0).max(420.0)
            } else {
                480.0
            };
            egui::Frame::new()
                .fill(PANEL)
                .corner_radius(0)
                .inner_margin(16)
                .show(ui, |ui| {
                    ui.set_width(panel_width - 32.0);
                    ui.set_min_height(if split { panel_height } else { 420.0 } - 32.0);
                    ui.spacing_mut().item_spacing.y = 12.0;
                    egui::Frame::new()
                        .fill(Color32::from_rgb(20, 20, 23))
                        .corner_radius(EDGE)
                        .inner_margin(4)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.spacing_mut().button_padding.y = 0.0;
                                ui.ctx().accesskit_node_builder(ui.id(), |node| {
                                    node.set_role(egui::accesskit::Role::TabList);
                                    node.set_label("Player panel");
                                });
                                let tab_width = (ui.available_width() - 4.0) * 0.5;
                                for (marks, title) in [
                                    (false, format!("Chapters ({})", book.chapters.len())),
                                    (true, format!("Bookmarks ({})", book.bookmarks.len())),
                                ] {
                                    let selected = app.book.show_bookmarks == marks;
                                    let tab =
                                        ui.add(
                                            egui::Button::new(
                                                RichText::new(title)
                                                    .size(13.0)
                                                    .color(if selected { TEXT } else { MUTED }),
                                            )
                                            .min_size(Vec2::new(tab_width, 36.0))
                                            .corner_radius(EDGE)
                                            .stroke(egui::Stroke::NONE)
                                            .selected(selected)
                                            .fill(
                                                if selected {
                                                    CONTROL
                                                } else {
                                                    Color32::TRANSPARENT
                                                },
                                            ),
                                        );
                                    ui.ctx().accesskit_node_builder(tab.id, |node| {
                                        node.set_role(egui::accesskit::Role::Tab);
                                        node.set_selected(selected);
                                    });
                                    if tab.clicked() {
                                        app.book.show_bookmarks = marks;
                                    }
                                }
                            });
                        });
                    egui::ScrollArea::vertical()
                        .id_salt(("full_player_list", app.book.show_bookmarks))
                        .max_height(panel_height - 32.0 - 44.0 - 12.0)
                        .show(ui, |ui| {
                            if let Some(at) =
                                app.book_lists(ui, &item, &source, true, app.book.show_bookmarks)
                            {
                                app.book_jump(at);
                            }
                        });
                });
        };
        if split {
            ui.spacing_mut().item_spacing.x = 40.0;
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(main_column);
                    egui::ScrollArea::vertical()
                        .id_salt("book_full_player_main")
                        .show(ui, |ui| main(ui, self));
                });
                ui.vertical(|ui| side(ui, self));
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("book_full_player")
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 40.0;
                    ui.vertical(|ui| main(ui, self));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    side(ui, self);
                    ui.add_space(if viewport.x <= 1000.0 { 32.0 } else { 48.0 });
                });
        }
    }

    fn book_transport(
        &self,
        ui: &mut egui::Ui,
        chapters: &[Chapter],
        current: Option<usize>,
        dock: bool,
    ) {
        let Some(player) = &self.player else { return };
        let size = if dock { 32.0 } else { 44.0 };
        let play_size = if dock { 40.0 } else { 64.0 };
        let (glyph, play_glyph) = if dock { (20.0, 25.0) } else { (27.5, 35.0) };
        let gap = if dock { 8.0 } else { 20.0 };
        let position_ms = (self.playing_seconds * 1000.0).round() as u64;
        let next = chapters.partition_point(|chapter| chapter.start_ms <= position_ms);
        control_row(ui, play_size, |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let inset = (ui.available_width() - size * 4.0 - play_size - gap * 4.0) * 0.5;
            if inset > 0.0 {
                ui.add_space(inset);
            }
            if ui
                .add_enabled(next > 0, |ui: &mut egui::Ui| {
                    book_icon(ui, Icon::ChapterPrevious, "Previous chapter", size, glyph)
                })
                .clicked()
                && let Some(index) = next.checked_sub(1)
            {
                let chapter = &chapters[index];
                let target = if current == Some(index) && position_ms - chapter.start_ms <= 3000 {
                    index.saturating_sub(1)
                } else {
                    index
                };
                player.seek(chapters[target].start_ms as f64 / 1000.0);
            }
            if book_icon(ui, Icon::Back30, "Back 30 seconds", size, glyph).clicked() {
                player.seek(self.playing_seconds - 30.0);
            }
            let (icon, label) = if self.paused {
                (Icon::Play, "Play")
            } else {
                (Icon::Pause, "Pause")
            };
            if book_icon(ui, icon, label, play_size, play_glyph).clicked() {
                player.pause();
            }
            if book_icon(ui, Icon::Forward30, "Forward 30 seconds", size, glyph).clicked() {
                player.seek((self.playing_seconds + 30.0).min(self.playing_duration));
            }
            if ui
                .add_enabled(next < chapters.len(), |ui: &mut egui::Ui| {
                    book_icon(ui, Icon::ChapterNext, "Next chapter", size, glyph)
                })
                .clicked()
            {
                player.seek(chapters[next].start_ms as f64 / 1000.0);
            }
        });
    }

    fn book_scrubber(&mut self, ui: &mut egui::Ui, chapter: Option<&Chapter>, dock: bool) {
        let range = chapter.map_or((0.0, self.playing_duration), |chapter| {
            (
                chapter.start_ms as f64 / 1000.0,
                chapter.end_ms as f64 / 1000.0,
            )
        });
        let (start, end) = self.book.scrub_range.unwrap_or(range);
        let duration = (end - start).max(0.0);
        let position = self.seek_position.unwrap_or(self.playing_seconds);
        let mut elapsed = (position - start).clamp(0.0, duration);
        let left = short_clock(elapsed);
        let right = format!("−{}", short_clock(duration - elapsed));
        if dock {
            let (row, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.5), egui::Sense::hover());
            let font = egui::FontId::proportional(11.0);
            let lhs = ui.painter().text(
                row.left_center(),
                egui::Align2::LEFT_CENTER,
                left,
                font.clone(),
                MUTED,
            );
            let rhs = ui.painter().text(
                row.right_center(),
                egui::Align2::RIGHT_CENTER,
                right,
                font,
                MUTED,
            );
            let track = egui::Rect::from_min_max(
                egui::pos2(lhs.right() + 12.0, row.center().y - 1.5),
                egui::pos2(rhs.left() - 12.0, row.center().y + 1.5),
            );
            ui.painter().rect_filled(track, 2, Color32::from_gray(65));
            let fraction = (elapsed / duration.max(1.0)).clamp(0.0, 1.0) as f32;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    track.min,
                    Vec2::new(track.width() * fraction, track.height()),
                ),
                2,
                ACCENT,
            );
        } else {
            let response = seekbar(ui, &mut elapsed, duration, None);
            if response.dragged() {
                self.book.scrub_range = Some((start, end));
                self.seek_position = Some(start + elapsed);
            }
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                if let Some(player) = &self.player {
                    player.seek(start + elapsed.min(duration));
                }
                self.seek_position = None;
                self.book.scrub_range = None;
            }
            captions(
                ui,
                egui::WidgetText::from(
                    RichText::new(left)
                        .size(13.0)
                        .line_height(Some(19.5))
                        .color(TEXT),
                ),
                right,
            );
        }
    }

    fn book_track(&self, ui: &mut egui::Ui, chapters: &[Chapter], current: Option<usize>) {
        let total = self.playing_duration.max(1.0);
        let seconds = self.seek_position.unwrap_or(self.playing_seconds);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), egui::Sense::hover());
        let ratio = |seconds: f64| (seconds / total).clamp(0.0, 1.0) as f32;
        let pos = |seconds| rect.left() + rect.width() * ratio(seconds);
        let y = rect.center().y;
        ui.painter()
            .hline(rect.x_range(), y, (4.0, Color32::from_gray(65)));
        ui.painter().hline(
            rect.left()..=pos(seconds),
            y,
            (4.0, Color32::from_rgb(92, 127, 168)),
        );
        for (index, chapter) in chapters.iter().enumerate().skip(1) {
            let color = if current == Some(index) {
                ACCENT
            } else {
                Color32::from_rgb(13, 13, 16)
            };
            ui.painter().vline(
                pos(chapter.start_ms as f64 / 1000.0),
                (y - 5.0)..=(y + 5.0),
                (2.0, color),
            );
        }
        let mut remaining = format!("{} left", book_duration((total - seconds) / self.speed));
        if self.speed != 1.0 {
            remaining.push_str(&format!(" at {}×", self.speed));
        }
        let mut left = egui::text::LayoutJob::default();
        for (value, color) in [
            ("Book ".into(), MUTED),
            (short_clock(seconds), TEXT),
            (format!(" of {}", short_clock(self.playing_duration)), MUTED),
        ] {
            left.append(
                &value,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(13.0),
                    color,
                    line_height: Some(19.5),
                    ..Default::default()
                },
            );
        }
        captions(ui, left.into(), remaining);
    }

    fn sleep_label(&self) -> Option<String> {
        self.book.sleep.map(|timer| match timer {
            Sleep::Minutes { remaining, .. } => short_clock(remaining.as_secs_f64().ceil()),
            Sleep::Chapter => "End of chapter".into(),
        })
    }

    fn book_menus_width(&self, ui: &egui::Ui, dock: bool) -> f32 {
        let timer = self.sleep_label();
        if dock {
            let speed = (text_width(ui, &format!("{}×", self.speed), 13.0) + 20.0).max(52.0);
            let badge = timer.map_or(0.0, |t| 11.0 + 3.0 + text_width(ui, &t, 11.0) + 8.0);
            badge + speed + 8.0 + 32.0
        } else {
            let sleep = timer.map_or_else(|| "Sleep".into(), |t| format!("Sleep · {t}"));
            book_tool_width(ui, &format!("Speed {}×", self.speed))
                + 12.0
                + book_tool_width(ui, &sleep)
        }
    }

    fn book_menus(&mut self, ui: &mut egui::Ui, chapter: Option<&Chapter>, dock: bool) {
        let timer = self.sleep_label();
        if dock && let Some(value) = &timer {
            let font = egui::FontId::proportional(11.0);
            let (badge, response) = ui.allocate_exact_size(
                Vec2::new(14.0 + text_width(ui, value, 11.0), 18.0),
                egui::Sense::hover(),
            );
            Icon::SleepTimer.paint(
                ui,
                badge.left_center() + Vec2::new(5.5, 0.0),
                11.0,
                TOOL_ON_TEXT,
            );
            ui.painter().text(
                badge.right_center(),
                egui::Align2::RIGHT_CENTER,
                value,
                font,
                TOOL_ON_TEXT,
            );
            response.on_hover_text("Sleep timer");
        } else if dock {
            // Keep keyboard focus on the same tool when its optional countdown appears.
            ui.skip_ahead_auto_ids(1);
        }
        let speed = format!("{}×", self.speed);
        let trigger = if dock {
            let width = (text_width(ui, &speed, 13.0) + 20.0).max(52.0);
            tool_button(ui, None, &speed, self.speed != 1.0, Vec2::new(width, 32.0))
                .on_hover_text("Playback speed")
        } else {
            book_tool(
                ui,
                Icon::Speed,
                &format!("Speed {speed}"),
                self.speed != 1.0,
            )
        };
        if dock {
            trigger.ctx.accesskit_node_builder(trigger.id, |node| {
                node.set_label(format!("Playback speed {speed}"))
            });
        }
        const SPEEDS: [f64; 8] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
        let selected = SPEEDS.iter().position(|&s| s == self.speed);
        if let Some(index) = choice_menu(
            &trigger,
            dock,
            "Speed",
            &[
                "0.75×",
                "1× (normal)",
                "1.25×",
                "1.5×",
                "1.75×",
                "2×",
                "2.5×",
                "3×",
            ],
            selected,
            SPEEDS.len(),
        ) {
            self.speed = SPEEDS[index];
            if let Some(player) = &self.player {
                player.speed(self.speed);
            }
        }
        let trigger = if dock {
            let response = book_icon(ui, Icon::SleepTimer, "Sleep timer", 32.0, 16.0);
            if timer.is_some() {
                Icon::SleepTimer.paint(ui, response.rect.center(), 16.0, ACCENT);
            }
            response
        } else {
            let label = timer
                .as_ref()
                .map_or_else(|| "Sleep".into(), |t| format!("Sleep · {t}"));
            book_tool(ui, Icon::SleepTimer, &label, timer.is_some())
        };
        const MINUTES: [u64; 4] = [15, 30, 45, 60];
        let selected = match self.book.sleep {
            None => Some(0),
            Some(Sleep::Minutes { minutes, .. }) => {
                MINUTES.iter().position(|&m| m == minutes).map(|i| i + 1)
            }
            Some(Sleep::Chapter) => Some(5),
        };
        if let Some(index) = choice_menu(
            &trigger,
            dock,
            "Sleep timer",
            &[
                "Off",
                "15 minutes",
                "30 minutes",
                "45 minutes",
                "60 minutes",
                "End of chapter",
            ],
            selected,
            if chapter.is_some() { 6 } else { 5 },
        ) {
            self.book.sleep = match index {
                0 => None,
                5 => Some(Sleep::Chapter),
                i => Some(Sleep::minutes(MINUTES[i - 1])),
            };
            if let Some(player) = &self.player {
                player.sleep_after_chapter(matches!(self.book.sleep, Some(Sleep::Chapter)));
            }
            self.book_toast(
                ui.ctx(),
                match self.book.sleep {
                    None => "Sleep timer off".into(),
                    Some(Sleep::Chapter) => "Pausing at the end of this chapter".into(),
                    Some(Sleep::Minutes { minutes, .. }) => {
                        format!("Sleep timer set for {minutes} minutes")
                    }
                },
            );
        }
    }
}

const TOOL_ON_TEXT: Color32 = Color32::from_rgb(140, 200, 255);

fn text_width(ui: &egui::Ui, label: &str, size: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(size), TEXT)
        .size()
        .x
}

pub(super) fn book_tool_width(ui: &egui::Ui, label: &str) -> f32 {
    16.0 + 18.0 + 8.0 + text_width(ui, label, 13.0) + 16.0
}

pub(super) fn book_tool(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    active: bool,
) -> egui::Response {
    tool_button(
        ui,
        Some(icon),
        label,
        active,
        Vec2::new(book_tool_width(ui, label), 40.0),
    )
}

fn tool_button(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: &str,
    active: bool,
    size: Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let ink = if active { TOOL_ON_TEXT } else { TEXT };
    let rounding = f32::from(EDGE);
    ui.painter().rect_filled(
        rect,
        rounding,
        if active {
            CONTROL_PRESSED
        } else {
            control_fill(&response)
        },
    );
    let font = egui::FontId::proportional(13.0);
    if let Some(icon) = icon {
        icon.paint(ui, rect.left_center() + Vec2::new(25.0, 0.0), 18.0, ink);
        ui.painter().text(
            rect.left_center() + Vec2::new(42.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            ink,
        );
    } else {
        ui.painter()
            .text(rect.center(), egui::Align2::CENTER_CENTER, label, font, ink);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            rounding + 3.0,
            (2.0, ACCENT),
            egui::StrokeKind::Middle,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn choice_menu(
    trigger: &egui::Response,
    dock: bool,
    title: &str,
    labels: &[&str],
    selected: Option<usize>,
    enabled_rows: usize,
) -> Option<usize> {
    let popup = egui::Popup::menu(trigger)
        .align(if dock {
            egui::RectAlign::TOP_END
        } else {
            egui::RectAlign::BOTTOM_START
        })
        .gap(8.0)
        .width(200.0)
        .frame(popover_frame().inner_margin(6));
    let mut picked = None;
    let shown = popup
        .show(|ui| {
            ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                node.set_role(egui::accesskit::Role::Menu);
                node.set_label(if title == "Speed" {
                    "Playback speed"
                } else {
                    title
                });
            });
            ui.set_width(186.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            // Focus the selected option on open. Only keyboard navigation draws its outline.
            let focus_style = trigger.id.with("menu keyboard focus");
            let key_pressed = ui.input(|i| {
                i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
            });
            let pointer_open = trigger.clicked_by(egui::PointerButton::Primary);
            let opening = trigger.clicked();
            let keyboard_focus = ui.data_mut(|data| {
                let show = data.get_temp_mut_or_default::<bool>(focus_style);
                if opening {
                    *show = !pointer_open;
                } else if key_pressed {
                    *show = true;
                }
                *show
            });
            let (heading, _) = ui.allocate_exact_size(Vec2::new(186.0, 30.0), egui::Sense::hover());
            ui.painter().text(
                heading.left_center() + Vec2::new(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                title.to_uppercase(),
                egui::FontId::proportional(12.0),
                MUTED,
            );
            let mut rows = Vec::with_capacity(enabled_rows);
            for (i, label) in labels.iter().enumerate() {
                let on = selected == Some(i);
                let response = ui
                    .add_enabled_ui(i < enabled_rows, |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::new(186.0, 36.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::RadioButton,
                                ui.is_enabled(),
                                on,
                                label,
                            )
                        });
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_role(egui::accesskit::Role::MenuItemRadio)
                        });
                        if response.hovered() {
                            ui.painter().rect_filled(rect, EDGE, CONTROL);
                        }
                        if keyboard_focus && response.has_focus() {
                            ui.painter().rect_stroke(
                                rect,
                                EDGE,
                                (2.0, ACCENT),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if on {
                            Icon::Check.paint(
                                ui,
                                rect.left_center() + Vec2::new(18.0, 0.0),
                                16.0,
                                ACCENT,
                            );
                        }
                        ui.painter().text(
                            rect.left_center() + Vec2::new(34.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(15.0),
                            if on { TOOL_ON_TEXT } else { TEXT },
                        );
                        response.on_hover_cursor(egui::CursorIcon::PointingHand)
                    })
                    .inner;
                if i < enabled_rows {
                    rows.push(response.id);
                    if trigger.clicked() && i == selected.unwrap_or(0) {
                        response.request_focus();
                    }
                    if response.has_focus() {
                        ui.memory_mut(|m| {
                            m.set_focus_lock_filter(
                                response.id,
                                egui::EventFilter {
                                    vertical_arrows: true,
                                    ..Default::default()
                                },
                            )
                        });
                    }
                    if response.clicked() {
                        picked = Some(i);
                        ui.close();
                    }
                }
            }
            if let Some(i) = rows.iter().position(|id| ui.memory(|m| m.has_focus(*id))) {
                let step = ui.input_mut(|input| {
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                        1
                    } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                        rows.len() - 1
                    } else {
                        0
                    }
                });
                if step != 0 {
                    ui.memory_mut(|m| {
                        m.move_focus(egui::FocusDirection::None);
                        m.request_focus(rows[(i + step) % rows.len()]);
                    });
                }
            }
            if picked.is_some() || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                trigger.request_focus();
            }
        })
        .is_some();
    trigger.ctx.accesskit_node_builder(trigger.id, |node| {
        node.set_has_popup(egui::accesskit::HasPopup::Menu);
        node.set_expanded(shown);
    });
    picked
}

fn captions(ui: &mut egui::Ui, left: egui::WidgetText, right: String) {
    ui.horizontal(|ui| {
        ui.label(left);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(right)
                    .size(13.0)
                    .line_height(Some(19.5))
                    .color(MUTED),
            );
        });
    });
}

pub(super) fn short_clock(seconds: f64) -> String {
    if seconds >= 3600.0 {
        clock(seconds)
    } else {
        let seconds = seconds.max(0.0) as u64;
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}
