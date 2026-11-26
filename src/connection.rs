//! First-run and Settings share the add-server flow in docs/design/multi-server/js/app.js.
use super::*;
use perfuse::plugins::{BUILTIN, audiobookshelf, jellyfin};
use std::collections::{BTreeMap, VecDeque};

const ADDRESS: &str = "Server address";
const USERNAME: &str = "Username";
const PASSWORD: &str = "Password";
const TOKEN: &str = "Access token";
const MANUAL: &str = "Connect manually with an address and token";
const QUICK: &str = "Use Quick Connect";
const PLEX_SIGN_IN: &str = "Sign in with Plex";
const NAME: &str = "Name in Perfuse";

#[derive(Clone, Copy)]
enum SignIn {
    PlexToken,
    JellyfinPassword,
    AudiobookshelfPassword,
    QuickConnect,
}

struct PlexChoice {
    server: auth::Server,
    address: usize,
    checked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Step {
    Types,
    Plex,
    Jellyfin,
    Audiobookshelf,
    Browser,
    QuickConnect,
    PlexChoices,
    Connected,
}

#[derive(Default)]
pub(super) struct Form {
    kind: Option<ServerKind>,
    manual: bool,
    next_focus: Option<&'static str>,
    issues: BTreeMap<&'static str, String>,
    choices: Vec<PlexChoice>,
    pub(super) pending: VecDeque<usize>,
    added: Vec<usize>,
    name: String,
    credential_error: Option<String>,
}

impl Form {
    pub(super) fn focus(&mut self, target: &str, response: &egui::Response) {
        if self.next_focus == Some(target) && response.enabled() {
            response.request_focus();
            self.next_focus = None;
        }
    }
}

pub(super) fn source_icon(kind: ServerKind) -> Icon {
    match kind {
        ServerKind::Plex => Icon::ServerPlex,
        ServerKind::Jellyfin => Icon::ServerJellyfin,
        ServerKind::Audiobookshelf => Icon::ServerAudiobookshelf,
    }
}

fn text_field(
    ui: &mut egui::Ui,
    label: &'static str,
    value: &mut String,
    required: bool,
    help: &str,
    error: Option<&str>,
) -> egui::Response {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
        let label_id = ui
            .horizontal(|ui| {
                let id = ui
                    .label(RichText::new(label).size(13.0).line_height(Some(19.5)))
                    .id;
                if required {
                    ui.label(
                        RichText::new("(required)")
                            .size(13.0)
                            .line_height(Some(19.5))
                            .color(MUTED),
                    );
                }
                id
            })
            .inner;
        let id = ui.make_persistent_id(label);
        let response = ui
            .scope(|ui| {
                let visuals = ui.visuals_mut();
                let border = error.map_or(Color32::from_rgb(58, 58, 64), |_| ERROR_TEXT);
                for widget in [
                    &mut visuals.widgets.inactive,
                    &mut visuals.widgets.hovered,
                    &mut visuals.widgets.active,
                ] {
                    widget.bg_stroke = (1.0, border).into();
                    widget.corner_radius = egui::CornerRadius::same(EDGE);
                    widget.expansion = 0.0;
                }
                ui.add(
                    egui::TextEdit::singleline(value)
                        .id(id)
                        .desired_width(f32::INFINITY)
                        .password(matches!(label, TOKEN | PASSWORD))
                        .char_limit(if label == NAME { 40 } else { usize::MAX })
                        .min_size(Vec2::new(0.0, 44.0))
                        .margin(egui::Margin::symmetric(16, 0))
                        .vertical_align(egui::Align::Center)
                        .background_color(Color32::from_rgb(20, 20, 23)),
                )
            })
            .inner
            .labelled_by(label_id);
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect.expand(1.5),
                EDGE + 1,
                (3.0, Color32::from_rgba_unmultiplied(61, 155, 255, 64)),
                egui::StrokeKind::Middle,
            );
        }
        ui.ctx().accesskit_node_builder(id, |node| {
            if required {
                node.set_required();
            }
            if let Some(error) = error {
                node.set_invalid(egui::accesskit::Invalid::True);
                node.set_description(error);
            } else if !help.is_empty() {
                node.set_description(help);
            }
        });
        if !help.is_empty() {
            ui.label(
                RichText::new(help)
                    .size(12.0)
                    .line_height(Some(18.0))
                    .color(MUTED),
            );
        }
        if let Some(error) = error {
            ui.label(RichText::new(error).size(12.0).color(ERROR_TEXT));
        }
        response
    })
    .inner
}

fn remember_field(ui: &mut egui::Ui, remember: &mut bool) {
    ui.scope(|ui| {
        let checked = *remember;
        ui.spacing_mut().icon_width = 18.0;
        ui.spacing_mut().icon_spacing = 12.0;
        let visuals = ui.visuals_mut();
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.corner_radius = egui::CornerRadius::same(2);
            if checked {
                widget.bg_fill = ACCENT;
                widget.fg_stroke.color = PANEL;
            }
        }
        ui.add_enabled(
            cfg!(windows),
            egui::Checkbox::new(
                remember,
                RichText::new("Remember me on this computer")
                    .size(13.0)
                    .color(TEXT),
            ),
        );
    });
}

impl App {
    fn validate_field(&mut self, field: &'static str) -> bool {
        let issue = match field {
            ADDRESS if self.server.trim().is_empty() => Some("Enter your server address.".into()),
            ADDRESS => match perfuse::plugins::base_url(&self.server) {
                Err(message) => Some(message),
                Ok(url) => self
                    .servers
                    .iter()
                    .find(|s| {
                        Some(s.session.kind) == self.form.kind
                            && session::origin(&s.session.server).as_deref()
                                == Some(&url.origin().ascii_serialization())
                    })
                    .map(|s| format!("This server is already added as \"{}\".", s.name())),
            },
            USERNAME
                if self.username.trim().is_empty()
                    && self.form.kind == Some(ServerKind::Jellyfin) =>
            {
                Some("Enter your Jellyfin username.".into())
            }
            USERNAME if self.username.trim().is_empty() => Some("Enter your username.".into()),
            PASSWORD
                if self.form.kind == Some(ServerKind::Audiobookshelf)
                    && self.password.is_empty() =>
            {
                Some("Enter your password.".into())
            }
            TOKEN if self.token.trim().is_empty() => Some("Enter your Plex access token.".into()),
            _ => None,
        };
        if let Some(issue) = issue {
            self.form.issues.insert(field, issue);
            false
        } else {
            self.form.issues.remove(field);
            true
        }
    }

    fn connect(&mut self, ctx: &egui::Context, method: SignIn) {
        let kind = match method {
            SignIn::PlexToken => ServerKind::Plex,
            SignIn::JellyfinPassword | SignIn::QuickConnect => ServerKind::Jellyfin,
            SignIn::AudiobookshelfPassword => ServerKind::Audiobookshelf,
        };
        if !self.validate_connection(method) {
            return;
        }
        let generation = self.begin();
        let mut session = session::Session {
            kind,
            server: self.server.clone(),
            token: self.token.clone().into(),
            client_id: self.client_id.clone(),
        };
        let username = self.username.clone();
        let password = std::mem::take(&mut self.password);
        if matches!(method, SignIn::QuickConnect) {
            self.quick_code = Some(String::new());
            self.form.next_focus = Some("Cancel");
        }
        let (tx, ctx, current) = (self.tx.clone(), ctx.clone(), self.generation.clone());
        thread::spawn(move || {
            let result = (|| {
                match method {
                    SignIn::PlexToken => {}
                    SignIn::AudiobookshelfPassword => {
                        audiobookshelf::auth::password(&mut session, &username, &password)?;
                    }
                    SignIn::JellyfinPassword => {
                        session.token = jellyfin::auth::password(
                            &session.server,
                            &session.client_id,
                            &username,
                            &password,
                        )?
                        .into();
                    }
                    SignIn::QuickConnect => {
                        session.token = jellyfin::auth::quick_connect(
                            &session.server,
                            &session.client_id,
                            |code| {
                                let _ = tx.send((generation, Reply::QuickConnectCode(code)));
                                ctx.request_repaint();
                            },
                            || current.load(Ordering::Relaxed) != generation,
                        )?
                        .into();
                    }
                }
                let source = session::open(&session)?;
                let sections = source.libraries()?;
                Ok(Reply::Connected(source, sections, session))
            })()
            .unwrap_or_else(Reply::Failed);
            let _ = tx.send((generation, result));
            ctx.request_repaint();
        });
    }

    fn validate_connection(&mut self, method: SignIn) -> bool {
        self.error = None;
        self.form.issues.clear();
        let required: &[&str] = match method {
            SignIn::PlexToken => &[ADDRESS, TOKEN],
            SignIn::JellyfinPassword => &[ADDRESS, USERNAME],
            SignIn::AudiobookshelfPassword => &[ADDRESS, USERNAME, PASSWORD],
            SignIn::QuickConnect => &[ADDRESS],
        };
        for &field in required {
            self.validate_field(field);
        }
        self.form.next_focus = required
            .iter()
            .copied()
            .find(|field| self.form.issues.contains_key(field));
        self.form.issues.is_empty()
    }

    fn credential_field(
        &mut self,
        ui: &mut egui::Ui,
        label: &'static str,
        help: &str,
    ) -> egui::Response {
        let value = match label {
            ADDRESS => &mut self.server,
            USERNAME => &mut self.username,
            TOKEN => &mut self.token,
            _ => &mut self.password,
        };
        let response = text_field(
            ui,
            label,
            value,
            label != PASSWORD || self.form.kind == Some(ServerKind::Audiobookshelf),
            help,
            self.form.issues.get(label).map(String::as_str),
        );
        self.form.focus(label, &response);
        if response.lost_focus() || (response.changed() && self.form.issues.contains_key(label)) {
            self.validate_field(label);
        }
        response
    }

    fn submit_button(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        method: SignIn,
        enter: bool,
    ) {
        ui.add_space(8.0);
        let clicked = if self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Connecting…");
            });
            false
        } else {
            ui.add(Pill::primary(Icon::Server, "Connect").min_width(126.0))
                .clicked()
        };
        if clicked || (enter && !self.busy) {
            self.connect(ctx, method);
        }
    }

    fn plex_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.colored_label(
            MUTED,
            "Sign in securely in your browser, then choose which of your Plex servers to add.",
        );
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.add_space(8.0);
            let sign_in = ui.add(Pill::primary(Icon::SignInBrowser, PLEX_SIGN_IN).min_width(175.0));
            self.form.focus(PLEX_SIGN_IN, &sign_in);
            if sign_in.clicked() {
                self.error = None;
                self.form.issues.clear();
                self.form.next_focus = Some("Cancel");
                self.login = Some(auth::Login::start(self.client_id.clone()));
            }
            remember_field(ui, &mut self.remember);
            let angle = if self.form.manual {
                0.0
            } else {
                -std::f32::consts::FRAC_PI_2
            };
            let chevron = Icon::ChevronDown
                .image(16.0, TEXT)
                .rotate(angle, Vec2::splat(0.5));
            let toggle = ui
                .scope(|ui| {
                    ui.spacing_mut().button_padding = Vec2::ZERO;
                    ui.add(
                        egui::Button::image_and_text(chevron, RichText::new(MANUAL).size(13.0))
                            .frame(false),
                    )
                })
                .inner;
            if toggle.clicked() {
                self.form.manual = !self.form.manual;
                self.form.issues.clear();
            }
            ui.ctx()
                .accesskit_node_builder(toggle.id, |node| node.set_expanded(self.form.manual));
            if self.form.manual {
                self.credential_field(ui, ADDRESS, "For example https://192.168.1.20:32400");
                let token = self.credential_field(ui, TOKEN, "");
                self.submit_button(
                    ui,
                    ctx,
                    SignIn::PlexToken,
                    token.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)),
                );
            }
        });
    }

    fn password_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let jellyfin = self.form.kind == Some(ServerKind::Jellyfin);
        ui.add_enabled_ui(!self.busy, |ui| {
            let example = if jellyfin {
                "For example http://192.168.1.30:8096"
            } else {
                "For example https://abs.example.com"
            };
            self.credential_field(ui, ADDRESS, example);
            ui.spacing_mut().item_spacing.x = 16.0;
            let password = ui.columns(2, |cols| {
                self.credential_field(&mut cols[0], USERNAME, "");
                self.credential_field(
                    &mut cols[1],
                    PASSWORD,
                    if jellyfin {
                        "Leave empty if your account has no password."
                    } else {
                        ""
                    },
                )
            });
            remember_field(ui, &mut self.remember);
            self.submit_button(
                ui,
                ctx,
                if jellyfin {
                    SignIn::JellyfinPassword
                } else {
                    SignIn::AudiobookshelfPassword
                },
                password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)),
            );
            if !jellyfin {
                return;
            }
            ui.horizontal(|ui| {
                let mid = ui.available_rect_before_wrap().center().x;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 18.0),
                    egui::Sense::hover(),
                );
                let y = rect.center().y;
                let line = (1.0, Color32::from_rgb(42, 42, 46));
                ui.painter().hline(rect.left()..=mid - 18.0, y, line);
                ui.painter().hline(mid + 18.0..=rect.right(), y, line);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "or",
                    egui::FontId::proportional(12.0),
                    MUTED,
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let mut quick = Pill::secondary(Icon::QuickConnect, QUICK).min_width(193.0);
                quick.tall = true;
                let quick = ui.add(quick);
                self.form.focus(QUICK, &quick);
                if quick.clicked() {
                    self.connect(ctx, SignIn::QuickConnect);
                }
                ui.label(
                    RichText::new(
                        "Approve from a device that's already signed in. Needs the server address.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
            });
        });
    }

    fn wait_for_sign_in(&mut self, ui: &mut egui::Ui) {
        let plex = self.login.is_some();
        if let Some(code) = self.quick_code.as_ref().filter(|c| !c.is_empty()) {
            ui.colored_label(MUTED, "On a device that's already signed in to Jellyfin, open your user settings, choose Quick Connect and enter this code.");
            let display = match (code.get(..3), code.get(3..)) {
                (Some(first), Some(last)) => format!("{first} {last}"),
                _ => code.clone(),
            };
            ui.label(RichText::new(display).size(32.0));
        }
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(if plex {
                "Waiting for you to sign in to Plex in your browser…"
            } else if self.quick_code.as_deref() == Some("") {
                "Requesting a Quick Connect code…"
            } else {
                "Waiting for approval…"
            });
        });
        if plex {
            ui.colored_label(MUTED, "Perfuse continues automatically when you're done.");
        }
        ui.horizontal(|ui| {
            let cancel = ui.add(Pill::secondary(Icon::Close, "Cancel"));
            self.form.focus("Cancel", &cancel);
            if cancel.clicked() {
                self.generation.fetch_add(1, Ordering::Relaxed);
                self.busy = false;
                self.login = None;
                self.login_url = None;
                self.quick_code = None;
                self.form.next_focus = Some(if plex { PLEX_SIGN_IN } else { QUICK });
            }
            if let Some(url) = &self.login_url {
                ui.hyperlink_to("Open the sign-in page again", url);
            }
        });
    }
}

fn glyph(ui: &egui::Ui, top_left: egui::Pos2, kind: ServerKind, size: f32) {
    let tile = egui::Rect::from_min_size(top_left, Vec2::splat(size));
    ui.painter().rect_filled(tile, 0, CONTROL);
    source_icon(kind).paint(ui, tile.center(), size * 0.55, TEXT);
}

fn already_added(servers: &[Server], resource: &auth::Server) -> bool {
    servers.iter().any(|s| {
        s.session.kind == ServerKind::Plex
            && resource
                .connections
                .iter()
                .any(|c| session::origin(&c.uri) == session::origin(&s.session.server))
    })
}

impl App {
    pub(super) fn offer_plex_servers(&mut self, servers: Vec<auth::Server>) {
        self.form.choices = servers
            .into_iter()
            .map(|server| {
                let checked = server.owned == Some(true) && !already_added(&self.servers, &server);
                PlexChoice {
                    server,
                    checked,
                    address: 0,
                }
            })
            .collect();
        self.form.next_focus = Some("Plex choice");
    }

    fn plex_choices(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.colored_label(MUTED, "Choose the servers to add to Perfuse.");
        let mut count = 0;
        ui.spacing_mut().item_spacing.y = 12.0;
        for (index, choice) in self.form.choices.iter_mut().enumerate() {
            let added = already_added(&self.servers, &choice.server);
            let can_choose = !added && !self.busy;
            choice.checked &= !added;
            let frame = egui::Frame::new()
                .fill(if choice.checked {
                    Color32::from_rgb(28, 34, 48)
                } else {
                    CONTROL
                })
                .stroke((1.0, if choice.checked { ACCENT } else { CONTROL }))
                .inner_margin(12)
                .corner_radius(0);
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    let check = ui.add_enabled(
                        can_choose,
                        egui::Checkbox::new(&mut choice.checked, &choice.server.name),
                    );
                    if can_choose && self.form.next_focus == Some("Plex choice") {
                        check.request_focus();
                        self.form.next_focus = None;
                    }
                    let ownership = match choice.server.owned {
                        Some(true) => "Your server",
                        Some(false) => "Shared with you",
                        None => "",
                    };
                    ui.label(
                        RichText::new(if added { "Already added" } else { ownership })
                            .size(12.0)
                            .color(MUTED),
                    );
                });
                let address = &choice.server.connections[choice.address].uri;
                if choice.server.connections.len() == 1 || added {
                    ui.label(RichText::new(address).size(12.0).color(MUTED));
                } else {
                    ui.add_enabled_ui(can_choose, |ui| {
                        egui::ComboBox::from_id_salt(("plex_address", index))
                            .selected_text(address)
                            .show_ui(ui, |ui| {
                                for (i, connection) in choice.server.connections.iter().enumerate()
                                {
                                    ui.selectable_value(&mut choice.address, i, &connection.uri);
                                }
                            });
                    });
                }
            });
            count += usize::from(choice.checked);
        }
        let label = match count {
            _ if self.busy => "Adding…".into(),
            0 => "Choose a server".into(),
            1 => "Add 1 server".into(),
            n => format!("Add {n} servers"),
        };
        if ui
            .add_enabled(!self.busy && count > 0, Pill::primary(Icon::Add, &label))
            .clicked()
        {
            self.form.pending = self
                .form
                .choices
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c.checked.then_some(i))
                .collect();
            self.connect_next_plex(ctx);
        }
        if !self.busy
            && !self.form.added.is_empty()
            && ui
                .add(Pill::secondary(
                    Icon::Check,
                    "Finish with connected servers",
                ))
                .clicked()
        {
            self.show_connected();
        }
    }

    fn connect_next_plex(&mut self, ctx: &egui::Context) {
        while let Some(i) = self.form.pending.pop_front() {
            let choice = &self.form.choices[i];
            if already_added(&self.servers, &choice.server) {
                continue;
            }
            self.server = choice.server.connections[choice.address].uri.clone();
            self.token = choice.server.access_token.clone().unwrap_or_default();
            self.connect(ctx, SignIn::PlexToken);
            return;
        }
        self.show_connected();
    }

    pub(super) fn connected(
        &mut self,
        ctx: &egui::Context,
        source: Source,
        sections: Vec<Library>,
        session: session::Session,
    ) {
        self.busy = false;
        let save_error = if self.remember {
            session::save(&session).err()
        } else {
            None
        };
        let mut server = Server::new(
            session,
            source,
            sections,
            self.remember && save_error.is_none(),
        );
        if server.prefs.name.is_empty() {
            let origin = session::origin(&server.session.server);
            let discovered = self.form.choices.iter().find(|choice| {
                choice
                    .server
                    .connections
                    .iter()
                    .any(|connection| session::origin(&connection.uri) == origin)
            });
            server.prefs.name = if let Some(choice) = discovered {
                choice.server.name.clone()
            } else {
                let domain = reqwest::Url::parse(server.session.server.trim())
                    .ok()
                    .and_then(|url| url.domain()?.split('.').next().map(str::to_owned));
                let mut name = domain
                    .unwrap_or_else(|| format!("{} server", server.session.kind.plugin().name));
                if let Some(initial) = name.get_mut(..1) {
                    initial.make_ascii_uppercase();
                }
                name
            };
        }
        let index = self.servers.len();
        server.prefs.position = index;
        server.persist();
        server.check(ctx);
        self.servers.push(server);
        self.form.added.push(index);
        if let Some(message) = save_error {
            self.form.credential_error = Some(message);
        }
        if self.form.pending.is_empty() {
            self.show_connected();
        } else {
            self.connect_next_plex(ctx);
        }
    }

    fn show_connected(&mut self) {
        self.clear_sign_in();
        self.error = None;
        self.form.choices.clear();
        self.form.issues.clear();
        if let [i] = *self.form.added {
            self.form.name = self.servers[i].name();
            self.form.next_focus = Some(NAME);
        } else {
            self.form.next_focus = Some("Finish");
        }
    }

    fn save_connection_name(&mut self) {
        if let [i] = *self.form.added {
            let name = self.form.name.trim();
            if !name.is_empty() && self.servers[i].name() != name {
                self.servers[i].prefs.name = name.to_owned();
                self.servers[i].persist();
            }
        }
    }

    fn finish_connection(&mut self) {
        self.save_connection_name();
        let from_settings = self.adding;
        let last = self
            .form
            .added
            .last()
            .copied()
            .or_else(|| self.servers.len().checked_sub(1));
        self.choose_source(None);
        self.adding = false;
        if from_settings {
            self.form.next_focus = Some("Add server");
        } else if let Some(i) = last {
            self.activate(i);
            self.overview = true;
        }
    }

    fn connection_success(&mut self, ui: &mut egui::Ui) {
        if let [i] = *self.form.added {
            let help = format!(
                "Shown in the sidebar as {} · name.",
                self.servers[i].session.kind.plugin().name
            );
            let field = text_field(ui, NAME, &mut self.form.name, false, &help, None);
            self.form.focus(NAME, &field);
            if field.lost_focus() {
                self.save_connection_name();
            }
        }
        for &i in &self.form.added {
            let server = &self.servers[i];
            ui.horizontal(|ui| {
                ui.add(server.icon().image(22.0, TEXT));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(server.name());
                    let n = server.sections.len();
                    let names = server
                        .sections
                        .iter()
                        .map(|lib| lib.title.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let text = if n == 0 && server.session.kind == ServerKind::Audiobookshelf {
                        "Found no audiobook libraries".into()
                    } else if n == 0 {
                        "Found no movie or TV libraries".into()
                    } else {
                        format!(
                            "Found {n} {}: {names}",
                            if n == 1 { "library" } else { "libraries" }
                        )
                    };
                    ui.label(RichText::new(text).size(12.0).color(MUTED));
                });
            });
        }
        ui.horizontal(|ui| {
            let label = if self.adding {
                "Done"
            } else {
                "Continue to Perfuse"
            };
            let (finish, another) = Pill::pair(
                ui,
                Pill::primary(Icon::Check, label),
                Pill::secondary(Icon::Add, "Add another server"),
            );
            let finish = ui.add(finish);
            self.form.focus("Finish", &finish);
            if finish.clicked() {
                self.finish_connection();
            }
            if ui.add(another).clicked() {
                self.save_connection_name();
                self.choose_source(None);
            }
        });
    }
}

fn source_tile(ui: &mut egui::Ui, kind: ServerKind, compact: bool) -> egui::Response {
    let title = kind.plugin().name;
    let description = match kind {
        ServerKind::Plex => "Movies and TV. Sign in with your Plex account.",
        ServerKind::Jellyfin => "Movies and TV. Use your server address and Jellyfin account.",
        ServerKind::Audiobookshelf => "Audiobooks with chapters and bookmarks.",
    };
    let width = ui.available_width();
    let left = if compact { 76.0 } else { 20.0 };
    let caption = ui.painter().layout(
        description.into(),
        egui::FontId::proportional(12.0),
        MUTED,
        width - left - 20.0,
    );
    let title_y = if compact { 20.0 } else { 80.0 };
    let height = (title_y + 30.0 + caption.size().y + 20.0).max(if compact { 84.0 } else { 170.0 });
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(description));
    let fill = if response.hovered() {
        Color32::from_rgb(28, 34, 48)
    } else {
        Color32::from_rgb(25, 25, 28)
    };
    let border = if response.hovered() {
        ACCENT
    } else {
        Color32::from_rgb(52, 52, 58)
    };
    ui.painter()
        .rect(rect, EDGE, fill, (1.0, border), egui::StrokeKind::Inside);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            EDGE + 2,
            (2.0, ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    glyph(ui, rect.min + Vec2::splat(20.0), kind, 44.0);
    let pos = rect.min + Vec2::new(left, title_y);
    ui.painter().text(
        pos,
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(16.0),
        TEXT,
    );
    ui.painter()
        .galley(pos + Vec2::new(0.0, 30.0), caption, MUTED);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl App {
    fn clear_sign_in(&mut self) {
        self.login = None; // Dropping Login cancels the Plex worker.
        self.login_url = None;
        self.quick_code = None;
        self.token.clear();
        self.username.clear();
        self.password.clear();
        self.server.clear();
    }

    pub(super) fn choose_source(&mut self, kind: Option<ServerKind>) {
        let previous = self.form.kind.unwrap_or(ServerKind::Plex);
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.busy = false;
        self.clear_sign_in();
        self.error = None;
        self.remember = cfg!(windows);
        self.form = Form {
            kind,
            next_focus: Some(match kind {
                None => previous.plugin().name,
                Some(ServerKind::Jellyfin | ServerKind::Audiobookshelf) => ADDRESS,
                Some(ServerKind::Plex) => PLEX_SIGN_IN,
            }),
            ..Form::default()
        };
        if !cfg!(test) && kind == Some(ServerKind::Plex) && !self.adding && self.servers.is_empty()
        {
            self.server = std::env::var("PLEX_URL").unwrap_or_default();
            self.token = std::env::var("PLEX_TOKEN").unwrap_or_default();
        }
    }

    pub(super) fn onboarding(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let width = 720.0_f32.min(ui.available_width());
        egui::ScrollArea::vertical()
            .id_salt(("onboarding", self.step()))
            .show(ui, |ui| {
                ui.add_space(36.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                    egui::Frame::new()
                        .fill(PANEL)
                        .corner_radius(0)
                        .inner_margin(44)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(width - 88.0);
                                ui.spacing_mut().item_spacing = Vec2::new(16.0, 32.0);
                                ui.allocate_ui_with_layout(
                                    Vec2::new(ui.available_width(), 60.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.add(Icon::Mark.image(40.0, Color32::WHITE));
                                        ui.label(RichText::new("PERFUSE").size(40.0).color(ACCENT));
                                    },
                                );
                                self.connection_fields(ui, ctx);
                            });
                        });
                });
            });
    }

    fn step(&self) -> Step {
        if !self.form.choices.is_empty() {
            Step::PlexChoices
        } else if !self.form.added.is_empty() {
            Step::Connected
        } else if self.login.is_some() {
            Step::Browser
        } else if self.quick_code.is_some() {
            Step::QuickConnect
        } else {
            match self.form.kind {
                Some(ServerKind::Plex) => Step::Plex,
                Some(ServerKind::Jellyfin) => Step::Jellyfin,
                Some(ServerKind::Audiobookshelf) => Step::Audiobookshelf,
                None => Step::Types,
            }
        }
    }
    pub(super) fn add_server_dialog(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect().size();
        let modal = egui::Modal::new("add_server".into())
            .backdrop_color(Color32::from_black_alpha(153))
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(0)
                    .inner_margin(32)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 24],
                        blur: 64,
                        spread: 0,
                        color: Color32::from_black_alpha(153),
                    }),
            )
            .show(ctx, |ui| {
                let width = 640.0_f32.min(screen.x - 48.0);
                ui.set_width(width - 64.0);
                // An anchored Area inherits the previous step's size. Measure each
                // scroll area against the viewport so a newly opened form can grow.
                ui.set_max_height(screen.y - 128.0);
                egui::ScrollArea::vertical()
                    .id_salt(self.step())
                    .max_height(screen.y - 128.0)
                    .show(ui, |ui| self.connection_fields(ui, ctx));
            });
        if self.adding && modal.should_close() {
            self.finish_connection();
        }
    }

    fn connection_fields(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.spacing_mut().item_spacing = Vec2::new(12.0, 20.0);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        let step = self.step();
        let title = match step {
            Step::Types if self.adding => "Add a server".into(),
            Step::Types if self.servers.is_empty() => "Make yourself at home.".into(),
            Step::Types => "Add another server".into(),
            Step::Plex | Step::Browser => "Connect to Plex".into(),
            Step::Jellyfin => "Connect to Jellyfin".into(),
            Step::Audiobookshelf => "Connect to Audiobookshelf".into(),
            Step::QuickConnect => "Jellyfin Quick Connect".into(),
            Step::PlexChoices => "Choose Plex servers".into(),
            Step::Connected if self.form.added.len() == 1 => {
                format!("{} is connected", self.servers[self.form.added[0]].name())
            }
            Step::Connected => format!("{} servers connected", self.form.added.len()),
        };
        ui.spacing_mut().item_spacing.y = if step == Step::Types && !self.adding {
            8.0
        } else {
            24.0
        };
        let mut back = false;
        let mut close = false;
        let height = if step == Step::Types { 32.0 } else { 40.0 };
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if step == Step::Connected {
                    let (tile, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(tile.center(), 20.0, Color32::from_rgb(23, 48, 31));
                    Icon::Check.paint(ui, tile.center(), 22.0, GREEN);
                } else if let Some(kind) = self.form.kind {
                    back = ui
                        .add(IconButton::new(Icon::Back, "Back to server types"))
                        .clicked();
                    let (tile, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
                    glyph(ui, tile.min, kind, 40.0);
                }
                let width = ui.available_width() - if self.adding { 44.0 } else { 0.0 };
                ui.allocate_ui_with_layout(
                    Vec2::new(width, height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(width);
                        ui.add(
                            egui::Label::new(RichText::new(&title).heading().size(22.0)).truncate(),
                        )
                        .on_hover_text(&title);
                    },
                );
                if self.adding {
                    close = ui.add(IconButton::new(Icon::Close, "Close")).clicked();
                }
            },
        );
        ui.spacing_mut().item_spacing.y = 20.0;
        if close {
            self.finish_connection();
            return;
        }
        if back {
            self.choose_source(None);
            return;
        }
        if let Some(error) = self
            .error
            .as_ref()
            .or(self.form.credential_error.as_ref())
            .cloned()
        {
            egui::Frame::new()
                .fill(Color32::from_rgb(44, 26, 26))
                .inner_margin(12)
                .corner_radius(0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(Icon::Alert.image(16.0, ERROR_TEXT));
                        ui.colored_label(ERROR_TEXT, error);
                        if ui.add(IconButton::new(Icon::Dismiss, "Dismiss")).clicked() {
                            self.error = None;
                            self.form.credential_error = None;
                        }
                    });
                });
        }
        if self.form.issues.len() > 1 {
            egui::Frame::new()
                .fill(Color32::from_rgb(44, 26, 26))
                .inner_margin(12)
                .corner_radius(0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Check the highlighted fields").color(ERROR_TEXT));
                    ui.horizontal_wrapped(|ui| {
                        for &field in self.form.issues.keys() {
                            if ui.link(field).clicked() {
                                self.form.next_focus = Some(field);
                            }
                        }
                    });
                });
        }
        match step {
            Step::Types => self.source_chooser(ui),
            Step::Plex => self.plex_form(ui, ctx),
            Step::Jellyfin | Step::Audiobookshelf => self.password_form(ui, ctx),
            Step::Browser | Step::QuickConnect => self.wait_for_sign_in(ui),
            Step::PlexChoices => self.plex_choices(ui, ctx),
            Step::Connected => self.connection_success(ui),
        }
    }
    fn source_chooser(&mut self, ui: &mut egui::Ui) {
        if !self.adding && !self.servers.is_empty() {
            ui.colored_label(MUTED, "Connected so far");
            for server in &self.servers {
                ui.horizontal(|ui| {
                    ui.add(server.icon().image(20.0, TEXT));
                    ui.label(server.name());
                });
            }
        } else {
            let (text, size) = if self.adding {
                ("What kind of server are you connecting to?", 15.0)
            } else {
                (
                    "Your films, shows and audiobooks, in original quality. Connect a server to get started. You can add more later.",
                    16.0,
                )
            };
            ui.label(
                RichText::new(text)
                    .size(size)
                    .line_height(Some(size * 1.5))
                    .color(MUTED),
            );
            if !self.adding {
                ui.add_space(12.0);
            }
        }
        let compact = ui.ctx().content_rect().width() <= 1000.0;
        let mut tile = |ui: &mut egui::Ui, kind: ServerKind| {
            let button = source_tile(ui, kind, compact);
            self.form.focus(kind.plugin().name, &button);
            if button.clicked() {
                self.choose_source(Some(kind));
            }
        };
        if compact {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                for plugin in BUILTIN {
                    tile(ui, plugin.kind);
                }
            });
        } else {
            ui.columns(BUILTIN.len(), |columns| {
                for (ui, plugin) in columns.iter_mut().zip(BUILTIN) {
                    tile(ui, plugin.kind);
                }
            });
        }
        if !self.adding
            && !self.servers.is_empty()
            && ui
                .add(Pill::primary(Icon::PageNext, "Continue to Perfuse"))
                .clicked()
        {
            self.finish_connection();
        }
    }
}
