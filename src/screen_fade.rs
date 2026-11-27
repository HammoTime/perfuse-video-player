//! A short, paint-only transition inside the browsing pane. The live destination
//! owns all interaction. Window chrome and popup layers never enter the snapshot.
use crate::{egui, prefs};
use egui::{Color32, Id, Rect, Shape, TextureHandle, TextureId, Ui, epaint::ClippedShape};
use std::time::Duration;

const FADE_SECONDS: f64 = 0.120;
const LOADING_HINT: f64 = 0.180;

#[derive(Default)]
pub(super) struct ScreenFade {
    destination: Option<Id>,
    current: PanePaint,
    outgoing: PanePaint,
    window: Option<(Rect, f32)>,
    started: Option<f64>,
    waiting_since: Option<f64>,
}

#[derive(Default)]
struct PanePaint {
    shapes: Vec<ClippedShape>,
    textures: Vec<TextureHandle>,
}

impl PanePaint {
    /// Paint shapes reference an image by ID. Keep those textures alive if the
    /// artwork cache evicts them while their previous screen is still visible.
    fn pin(&mut self, ui: &Ui) {
        if !self.textures.is_empty() {
            return;
        }
        let manager = ui.ctx().tex_manager();
        let mut textures = manager.write();
        fn visit(shape: &Shape, f: &mut impl FnMut(TextureId)) {
            if let Shape::Vec(shapes) = shape {
                for child in shapes {
                    visit(child, f);
                }
            } else {
                f(shape.texture_id());
            }
        }
        for entry in &self.shapes {
            visit(&entry.shape, &mut |id| {
                if let TextureId::Managed(n) = id
                    && n != 0
                    && !self.textures.iter().any(|keep| keep.id() == id)
                    && textures.meta(id).is_some()
                {
                    textures.retain(id);
                    self.textures.push(TextureHandle::new(manager.clone(), id));
                }
            });
        }
    }

    fn paint(&self, ui: &Ui, pane: Rect, opacity: f32) {
        let mut painter = ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(pane);
        painter.set_opacity(opacity);
        painter.rect_filled(pane, 0, Color32::from_rgb(13, 13, 16));
        for entry in &self.shapes {
            painter
                .with_clip_rect(entry.clip_rect)
                .add(entry.shape.clone());
        }
    }
}

impl ScreenFade {
    pub(super) fn first_shape(ui: &Ui) -> usize {
        ui.ctx().graphics(|layers| {
            layers
                .get(ui.layer_id())
                .map_or(0, |list| list.next_idx().0)
        })
    }

    fn window(ui: &Ui) -> (Rect, f32) {
        (ui.ctx().content_rect(), ui.ctx().pixels_per_point())
    }

    /// Keep the previous pixels in place while a navigation request is pending.
    /// The old widgets are absent, so a second click cannot invoke a stale item.
    pub(super) fn hold(&mut self, ui: &mut Ui, pane: Rect) -> bool {
        if self.current.shapes.is_empty() || self.window != Some(Self::window(ui)) {
            return false;
        }
        let now = ui.input(|i| i.time);
        let since = *self.waiting_since.get_or_insert_with(|| {
            self.current.pin(ui);
            now
        });
        self.current.paint(ui, pane, 1.0);
        self.paint_fade(ui, pane, now);
        let delay = LOADING_HINT - (now - since);
        if delay > 0.0 {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(delay));
        } else {
            let bounds = Rect::from_min_size(
                egui::pos2(pane.right() - 36.0, pane.top() + 12.0),
                egui::Vec2::splat(20.0),
            );
            ui.put(bounds, egui::Spinner::new().size(16.0));
        }
        true
    }

    pub(super) fn present(&mut self, ui: &Ui, pane: Rect, key: Id, first: usize) {
        if ui.ctx().will_discard() {
            return;
        }
        let now = ui.input(|i| i.time);
        let window = Self::window(ui);
        if self.window != Some(window) {
            self.started = None;
            self.outgoing = PanePaint::default();
            self.destination = None;
            self.window = Some(window);
        }
        self.waiting_since = None;
        if self
            .destination
            .replace(key)
            .is_some_and(|last| last != key)
        {
            // Retarget a rapid second navigation without restarting a fade at full opacity.
            if prefs::client_animations_enabled() {
                if self.started.is_none_or(|start| now - start >= FADE_SECONDS) {
                    std::mem::swap(&mut self.current, &mut self.outgoing);
                    self.outgoing.pin(ui);
                    self.started = Some(now);
                }
            } else {
                self.started = None;
                self.outgoing = PanePaint::default();
            }
        }
        self.current.textures.clear();
        self.current.shapes.clear();
        ui.ctx().graphics(|layers| {
            if let Some(list) = layers.get(ui.layer_id()) {
                self.current.shapes.extend(
                    list.all_entries()
                        .skip(first)
                        .filter(|entry| {
                            entry.clip_rect.intersects(pane)
                                && !matches!(entry.shape, egui::Shape::Noop)
                        })
                        .cloned(),
                );
            }
        });
        self.paint_fade(ui, pane, now);
    }

    fn paint_fade(&mut self, ui: &Ui, pane: Rect, now: f64) {
        if let Some(started) = self.started {
            let opacity = (1.0 - (now - started) / FADE_SECONDS).clamp(0.0, 1.0) as f32;
            if opacity > 0.0 {
                self.outgoing.paint(ui, pane, opacity);
                ui.ctx().request_repaint();
            } else {
                self.started = None;
                self.outgoing = PanePaint::default();
            }
        }
    }
}
