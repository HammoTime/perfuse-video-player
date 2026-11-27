use super::transport::{book_icon, book_tool};
use super::*;

impl App {
    pub(super) fn book_volume(&mut self, ui: &mut egui::Ui, dock: bool) {
        let mut gain = self.playback.volume.gain();
        let muted = gain == 0.0;
        let icon = if muted {
            Icon::VolumeMuted
        } else if gain < 50.0 {
            Icon::VolumeLow
        } else {
            Icon::VolumeHigh
        };
        let trigger = if dock {
            book_icon(ui, icon, "Volume", 32.0, 16.0)
        } else {
            book_tool(ui, icon, "Volume", muted)
        };
        let popup = egui::Popup::from_toggle_button_response(&trigger)
            .align(if dock {
                egui::RectAlign::TOP_END
            } else {
                egui::RectAlign::BOTTOM_START
            })
            .gap(8.0)
            .frame(popover_frame().inner_margin(12))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let escape = popup.is_open() && ui.input(|i| i.key_pressed(egui::Key::Escape));
        let shown = popup
            .show(|ui| {
                ui.set_width(176.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("VOLUME").size(12.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{gain:.0}%")).size(13.0));
                    });
                });
                ui.spacing_mut().slider_width = ui.available_width();
                let slider = ui.add(
                    egui::Slider::new(&mut gain, 0.0..=100.0)
                        .step_by(1.0)
                        .show_value(false),
                );
                ui.ctx()
                    .accesskit_node_builder(slider.id, |node| node.set_label("Audiobook volume"));
                if trigger.clicked() {
                    slider.request_focus();
                }
                let toggle = book_tool(ui, icon, if muted { "Unmute" } else { "Mute" }, muted);
                if toggle.clicked() {
                    gain = if muted {
                        self.playback.volume.level()
                    } else {
                        0.0
                    };
                }
                if (slider.changed() || toggle.clicked())
                    && self.playback.volume.set(gain)
                    && let Some(player) = &self.playback.player
                {
                    player.volume(gain);
                }
                if !slider.dragged() {
                    self.playback.volume.save();
                }
            })
            .is_some();
        ui.ctx()
            .accesskit_node_builder(trigger.id, |node| node.set_expanded(shown));
        if escape {
            trigger.request_focus();
        }
    }
}
