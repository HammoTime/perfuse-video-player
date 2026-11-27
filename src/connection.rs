//! First-run and Settings share the add-server flow in docs/design/multi-server/js/app.js.
use super::*;
use perfuse::plugins::{BUILTIN, SignInMethod, SignInRequest};
use std::collections::{BTreeMap, VecDeque};

const ADDRESS: &str = "Server address";
const USERNAME: &str = "Username";
const PASSWORD: &str = "Password";
const TOKEN: &str = "Access token";
const MANUAL: &str = "Connect manually with an address and token";
const QUICK: &str = "Use Quick Connect";
const PLEX_SIGN_IN: &str = "Sign in with Plex";
const NAME: &str = "Name in Perfuse";
// Shown, not enforced: whether to connect over plain HTTP is the user's decision.
const UNENCRYPTED: &str = "This address is not encrypted. Your sign-in and media will cross the internet unprotected. Use an https:// address if the server has one.";

/// Replies from the add-server flow. They never share the navigation channel.
pub(super) enum Reply {
    Connected(Source, Vec<Library>, session::Session, prefs::LibraryOrder),
    QuickConnectCode(String),
    Failed(String),
}

/// Add-server form state. Navigation cannot cancel or clear a sign-in in progress.
pub(super) struct SignIn {
    pub(super) server: String,
    pub(super) token: String,
    pub(super) username: String,
    pub(super) password: String,
    pub(super) quick_code: Option<String>,
    pub(super) client_id: String,
    pub(super) form: Form,
    pub(super) login: Option<auth::Login>,
    pub(super) login_url: Option<String>,
    pub(super) remember: bool,
    pub(super) busy: bool,
    pub(super) error: Option<String>,
    pub(super) generation: Arc<AtomicU64>,
    pub(super) tx: Sender<(u64, Reply)>,
    rx: Receiver<(u64, Reply)>,
}

impl SignIn {
    pub(super) fn new(server: String) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            server,
            token: String::new(),
            username: String::new(),
            password: String::new(),
            quick_code: None,
            // The startup worker restores the installation ID before sign-in is shown.
            client_id: String::new(),
            form: Form::default(),
            login: None,
            login_url: None,
            remember: cfg!(windows),
            busy: false,
            error: None,
            generation: Arc::new(AtomicU64::new(0)),
            tx,
            rx,
        }
    }

    pub(super) fn begin(&mut self) -> u64 {
        self.busy = true;
        self.error = None;
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }
}

struct PlexChoice {
    server: auth::Server,
    address: usize,
    checked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Step {
    Types,
    /// The chosen server type's sign-in form.
    Form(ServerKind),
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
    pub(super) credential_error: Option<String>,
    /// An expired server being signed in again, by kind and origin.
    replacing: Option<(ServerKind, Option<String>)>,
}

impl Form {
    /// Signs in to an existing server again instead of adding a new one.
    pub(super) fn replace(&mut self, kind: ServerKind, address: &str) {
        self.replacing = Some((kind, session::origin(address)));
    }

    fn replaces(&self, kind: ServerKind, address: &str) -> bool {
        self.replacing
            .as_ref()
            .is_some_and(|(k, origin)| *k == kind && *origin == session::origin(address))
    }

    pub(super) fn added_any(&self) -> bool {
        !self.added.is_empty()
    }

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
    warning: Option<&str>,
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
            } else if let Some(warning) = warning {
                node.set_description(warning);
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
        if let Some(warning) = warning {
            unencrypted_warning(ui, warning);
        }
        if let Some(error) = error {
            ui.label(RichText::new(error).size(12.0).color(ERROR_TEXT));
        }
        response
    })
    .inner
}

/// A caution under an address that travels over plain HTTP outside the local network.
fn unencrypted_warning(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(Icon::Alert.image(14.0, ERROR_TEXT));
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .size(12.0)
                    .line_height(Some(18.0))
                    .color(ERROR_TEXT),
            )
            .wrap(),
        );
    });
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
    /// The plugin for the server type the form is signing in to.
    fn sign_in_plugin(&self) -> &'static perfuse::plugins::Plugin {
        self.sign_in.form.kind.unwrap_or_default().plugin()
    }

    fn validate_field(&mut self, field: &'static str) -> bool {
        let plugin = self.sign_in_plugin();
        let issue = match field {
            ADDRESS if self.sign_in.server.trim().is_empty() => {
                Some("Enter your server address.".into())
            }
            ADDRESS => match perfuse::plugins::base_url(&self.sign_in.server) {
                Err(message) => Some(message),
                Ok(url) => {
                    let origin = Some(url.origin().ascii_serialization());
                    self.servers
                        .iter()
                        .find(|s| {
                            Some(s.session.kind) == self.sign_in.form.kind
                                && session::origin(&s.session.server) == origin
                                && !self
                                    .sign_in
                                    .form
                                    .replaces(s.session.kind, &s.session.server)
                        })
                        .map(|s| format!("This server is already added as \"{}\".", s.name()))
                }
            },
            // A server that allows an empty password names itself, so the username stands out.
            USERNAME
                if self.sign_in.username.trim().is_empty()
                    && plugin.password_sign_in() == Some(false) =>
            {
                Some(format!("Enter your {} username.", plugin.name))
            }
            USERNAME if self.sign_in.username.trim().is_empty() => {
                Some("Enter your username.".into())
            }
            PASSWORD
                if plugin.password_sign_in() == Some(true) && self.sign_in.password.is_empty() =>
            {
                Some("Enter your password.".into())
            }
            TOKEN if self.sign_in.token.trim().is_empty() => {
                Some(format!("Enter your {} access token.", plugin.name))
            }
            _ => None,
        };
        if let Some(issue) = issue {
            self.sign_in.form.issues.insert(field, issue);
            false
        } else {
            self.sign_in.form.issues.remove(field);
            true
        }
    }

    fn connect(&mut self, ctx: &egui::Context, method: SignInMethod) {
        let Some(kind) = self.sign_in.form.kind else {
            return;
        };
        if !self.validate_connection(method) {
            return;
        }
        let generation = self.sign_in.begin();
        let mut session = session::Session {
            kind,
            server: self.sign_in.server.clone(),
            token: self.sign_in.token.clone().into(),
            client_id: self.sign_in.client_id.clone(),
        };
        let username = self.sign_in.username.clone();
        let password = std::mem::take(&mut self.sign_in.password);
        if method == SignInMethod::QuickConnect {
            self.sign_in.quick_code = Some(String::new());
            self.sign_in.form.next_focus = Some("Cancel");
        }
        let (tx, ctx, current) = (
            self.sign_in.tx.clone(),
            ctx.clone(),
            self.sign_in.generation.clone(),
        );
        // Its own thread: Quick Connect waits up to ten minutes for approval.
        thread::spawn(move || {
            let result = (|| {
                let show_code = |code| {
                    let _ = tx.send((generation, Reply::QuickConnectCode(code)));
                    ctx.request_repaint();
                };
                let cancelled = || current.load(Ordering::Relaxed) != generation;
                let request = SignInRequest {
                    method,
                    username: &username,
                    password: &password,
                    show_code: &show_code,
                    cancelled: &cancelled,
                };
                (kind.plugin().authenticate)(&mut session, &request).map_err(String::from)?;
                let source = session::open(&session)?;
                let sections = source.libraries().map_err(String::from)?;
                let prefs = prefs::load_library_order(&session);
                Ok(Reply::Connected(source, sections, session, prefs))
            })()
            .unwrap_or_else(Reply::Failed);
            let _ = tx.send((generation, result));
            ctx.request_repaint();
        });
    }

    fn validate_connection(&mut self, method: SignInMethod) -> bool {
        self.sign_in.error = None;
        self.sign_in.form.issues.clear();
        let required: &[&str] = match method {
            SignInMethod::Token => &[ADDRESS, TOKEN],
            SignInMethod::Password { required: false } => &[ADDRESS, USERNAME],
            SignInMethod::Password { required: true } => &[ADDRESS, USERNAME, PASSWORD],
            SignInMethod::QuickConnect => &[ADDRESS],
            SignInMethod::Account => &[],
        };
        for &field in required {
            self.validate_field(field);
        }
        self.sign_in.form.next_focus = required
            .iter()
            .copied()
            .find(|field| self.sign_in.form.issues.contains_key(field));
        self.sign_in.form.issues.is_empty()
    }

    fn credential_field(
        &mut self,
        ui: &mut egui::Ui,
        label: &'static str,
        help: &str,
    ) -> egui::Response {
        let password_required = self.sign_in_plugin().password_sign_in() == Some(true);
        let warning = (label == ADDRESS && perfuse::unencrypted_remote(&self.sign_in.server))
            .then_some(UNENCRYPTED);
        let value = match label {
            ADDRESS => &mut self.sign_in.server,
            USERNAME => &mut self.sign_in.username,
            TOKEN => &mut self.sign_in.token,
            _ => &mut self.sign_in.password,
        };
        let response = text_field(
            ui,
            label,
            value,
            label != PASSWORD || password_required,
            help,
            self.sign_in.form.issues.get(label).map(String::as_str),
            warning,
        );
        self.sign_in.form.focus(label, &response);
        if response.lost_focus()
            || (response.changed() && self.sign_in.form.issues.contains_key(label))
        {
            self.validate_field(label);
        }
        response
    }

    fn submit_button(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        method: SignInMethod,
        enter: bool,
    ) {
        ui.add_space(8.0);
        let clicked = if self.sign_in.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Connecting…");
            });
            false
        } else {
            ui.add(Pill::primary(Icon::Server, "Connect").min_width(126.0))
                .clicked()
        };
        if clicked || (enter && !self.sign_in.busy) {
            self.connect(ctx, method);
        }
    }

    fn plex_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.colored_label(
            MUTED,
            format!(
                "Sign in securely in your browser, then choose which of your {} servers to add.",
                self.sign_in_plugin().name
            ),
        );
        ui.add_enabled_ui(!self.sign_in.busy, |ui| {
            ui.add_space(8.0);
            let sign_in = ui.add(Pill::primary(Icon::SignInBrowser, PLEX_SIGN_IN).min_width(175.0));
            self.sign_in.form.focus(PLEX_SIGN_IN, &sign_in);
            if sign_in.clicked() {
                self.sign_in.error = None;
                self.sign_in.form.issues.clear();
                self.sign_in.form.next_focus = Some("Cancel");
                self.sign_in.login = Some(auth::Login::start(self.sign_in.client_id.clone()));
            }
            remember_field(ui, &mut self.sign_in.remember);
            let angle = if self.sign_in.form.manual {
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
                self.sign_in.form.manual = !self.sign_in.form.manual;
                self.sign_in.form.issues.clear();
            }
            ui.ctx().accesskit_node_builder(toggle.id, |node| {
                node.set_expanded(self.sign_in.form.manual)
            });
            if self.sign_in.form.manual {
                let example = format!("For example {}", self.sign_in_plugin().address_example);
                self.credential_field(ui, ADDRESS, &example);
                let token = self.credential_field(ui, TOKEN, "");
                self.submit_button(
                    ui,
                    ctx,
                    SignInMethod::Token,
                    token.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)),
                );
            }
        });
    }

    fn password_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let plugin = self.sign_in_plugin();
        let required = plugin.password_sign_in().unwrap_or(true);
        ui.add_enabled_ui(!self.sign_in.busy, |ui| {
            let example = format!("For example {}", plugin.address_example);
            self.credential_field(ui, ADDRESS, &example);
            ui.spacing_mut().item_spacing.x = 16.0;
            let password = ui.columns(2, |cols| {
                self.credential_field(&mut cols[0], USERNAME, "");
                self.credential_field(
                    &mut cols[1],
                    PASSWORD,
                    if !required {
                        "Leave empty if your account has no password."
                    } else {
                        ""
                    },
                )
            });
            remember_field(ui, &mut self.sign_in.remember);
            self.submit_button(
                ui,
                ctx,
                SignInMethod::Password { required },
                password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)),
            );
            if !plugin.offers(SignInMethod::QuickConnect) {
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
                self.sign_in.form.focus(QUICK, &quick);
                if quick.clicked() {
                    self.connect(ctx, SignInMethod::QuickConnect);
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
        let plex = self.sign_in.login.is_some();
        if let Some(code) = self.sign_in.quick_code.as_ref().filter(|c| !c.is_empty()) {
            ui.colored_label(MUTED, format!("On a device that's already signed in to {}, open your user settings, choose Quick Connect and enter this code.", self.sign_in_plugin().name));
            let display = match (code.get(..3), code.get(3..)) {
                (Some(first), Some(last)) => format!("{first} {last}"),
                _ => code.clone(),
            };
            ui.label(RichText::new(display).size(32.0));
        }
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(if plex {
                format!(
                    "Waiting for you to sign in to {} in your browser…",
                    self.sign_in_plugin().name
                )
            } else if self.sign_in.quick_code.as_deref() == Some("") {
                "Requesting a Quick Connect code…".into()
            } else {
                "Waiting for approval…".into()
            });
        });
        if plex {
            ui.colored_label(MUTED, "Perfuse continues automatically when you're done.");
        }
        ui.horizontal(|ui| {
            let cancel = ui.add(Pill::secondary(Icon::Close, "Cancel"));
            self.sign_in.form.focus("Cancel", &cancel);
            if cancel.clicked() {
                self.sign_in.generation.fetch_add(1, Ordering::Relaxed);
                self.sign_in.busy = false;
                self.sign_in.login = None;
                self.sign_in.login_url = None;
                self.sign_in.quick_code = None;
                self.sign_in.form.next_focus = Some(if plex { PLEX_SIGN_IN } else { QUICK });
            }
            if let Some(url) = &self.sign_in.login_url {
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

/// Whether a discovered server is already in Perfuse, other than the one being signed in again.
fn already_added(
    servers: &[Server],
    resource: &auth::Server,
    replacing: &Option<(ServerKind, Option<String>)>,
) -> bool {
    servers.iter().any(|s| {
        s.session.kind == ServerKind::Plex
            && replacing.as_ref().is_none_or(|(kind, origin)| {
                *kind != s.session.kind || *origin != session::origin(&s.session.server)
            })
            && resource
                .connections
                .iter()
                .any(|c| session::origin(&c.uri) == session::origin(&s.session.server))
    })
}

impl App {
    pub(super) fn poll_sign_in(&mut self, ctx: &egui::Context) {
        while let Ok((generation, reply)) = self.sign_in.rx.try_recv() {
            if generation != self.sign_in.generation.load(Ordering::Relaxed) {
                continue;
            }
            match reply {
                Reply::QuickConnectCode(code) => self.sign_in.quick_code = Some(code),
                Reply::Connected(source, sections, session, prefs) => {
                    self.connected(ctx, source, sections, session, prefs);
                }
                Reply::Failed(error) => {
                    self.sign_in.busy = false;
                    self.sign_in.quick_code = None;
                    self.sign_in.form.pending.clear();
                    self.sign_in.error = Some(error);
                }
            }
        }
        let auth_events: Vec<_> = self
            .sign_in
            .login
            .as_ref()
            .map(|login| login.events.try_iter().collect())
            .unwrap_or_default();
        for event in auth_events {
            match event {
                auth::Event::Browser(url) => {
                    self.sign_in.login_url = Some(url.clone());
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
                auth::Event::Failed(error) => {
                    self.sign_in.login = None;
                    self.sign_in.login_url = None;
                    self.sign_in.error = Some(error);
                }
                auth::Event::SignedIn(servers) => {
                    self.sign_in.login = None;
                    self.sign_in.login_url = None;
                    if servers.is_empty() {
                        self.sign_in.error = Some("Signed in, but Plex returned no compatible movie or TV servers for this account.".into());
                    } else {
                        self.offer_plex_servers(servers);
                    }
                }
            }
        }
    }

    pub(super) fn offer_plex_servers(&mut self, servers: Vec<auth::Server>) {
        self.sign_in.form.choices = servers
            .into_iter()
            .map(|server| {
                let checked = server.owned == Some(true)
                    && !already_added(&self.servers, &server, &self.sign_in.form.replacing);
                PlexChoice {
                    server,
                    checked,
                    address: 0,
                }
            })
            .collect();
        self.sign_in.form.next_focus = Some("Plex choice");
    }

    fn plex_choices(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.colored_label(MUTED, "Choose the servers to add to Perfuse.");
        let mut count = 0;
        ui.spacing_mut().item_spacing.y = 12.0;
        let replacing = self.sign_in.form.replacing.clone();
        for (index, choice) in self.sign_in.form.choices.iter_mut().enumerate() {
            let added = already_added(&self.servers, &choice.server, &replacing);
            let can_choose = !added && !self.sign_in.busy;
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
                    if can_choose && self.sign_in.form.next_focus == Some("Plex choice") {
                        check.request_focus();
                        self.sign_in.form.next_focus = None;
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
                if !added
                    && perfuse::unencrypted_remote(&choice.server.connections[choice.address].uri)
                {
                    unencrypted_warning(ui, UNENCRYPTED);
                }
            });
            count += usize::from(choice.checked);
        }
        let label = match count {
            _ if self.sign_in.busy => "Adding…".into(),
            0 => "Choose a server".into(),
            1 => "Add 1 server".into(),
            n => format!("Add {n} servers"),
        };
        if ui
            .add_enabled(
                !self.sign_in.busy && count > 0,
                Pill::primary(Icon::Add, &label),
            )
            .clicked()
        {
            self.sign_in.form.pending = self
                .sign_in
                .form
                .choices
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c.checked.then_some(i))
                .collect();
            self.connect_next_plex(ctx);
        }
        if !self.sign_in.busy
            && !self.sign_in.form.added.is_empty()
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
        while let Some(i) = self.sign_in.form.pending.pop_front() {
            let choice = &self.sign_in.form.choices[i];
            if already_added(&self.servers, &choice.server, &self.sign_in.form.replacing) {
                continue;
            }
            self.sign_in.server = choice.server.connections[choice.address].uri.clone();
            self.sign_in.token = choice.server.access_token.clone().unwrap_or_default();
            self.connect(ctx, SignInMethod::Token);
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
        prefs: prefs::LibraryOrder,
    ) {
        self.sign_in.busy = false;
        let remember = self.sign_in.remember;
        // Credential Manager can wait on another instance, so the save reports back later.
        if remember {
            self.remember_in_background(session.clone());
        }
        let replaced = self.servers.iter().position(|s| {
            self.sign_in
                .form
                .replaces(s.session.kind, &s.session.server)
                && s.session.kind == session.kind
                && session::origin(&s.session.server) == session::origin(&session.server)
        });
        if let Some(index) = replaced {
            // Signing in again keeps the server's place, name and library choices.
            let server = &mut self.servers[index];
            if server.remembered && !remember {
                // The expired sign-in would otherwise return at the next launch.
                let previous = server.session.clone();
                tasks::spawn(move || {
                    let _ = session::clear(&previous);
                });
            }
            server.reset_previews();
            server.session = session;
            server.source = source;
            server.sections = sections;
            server.online = true;
            server.expired = false;
            server.remembered = remember;
            server.check(ctx);
            self.sign_in.form.replacing = None;
            self.sign_in.form.added.push(index);
            if self.active == Some(index) {
                self.open_server(index, ctx);
            }
        } else {
            let mut server = Server::new(session, source, sections, remember);
            server.prefs = prefs;
            if server.prefs.name.is_empty() {
                let origin = session::origin(&server.session.server);
                let discovered = self.sign_in.form.choices.iter().find(|choice| {
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
            self.sign_in.form.added.push(index);
        }
        if self.sign_in.form.pending.is_empty() {
            self.show_connected();
        } else {
            self.connect_next_plex(ctx);
        }
    }

    fn show_connected(&mut self) {
        self.clear_sign_in();
        self.sign_in.error = None;
        self.sign_in.form.choices.clear();
        self.sign_in.form.issues.clear();
        if let [i] = *self.sign_in.form.added {
            self.sign_in.form.name = self.servers[i].name();
            self.sign_in.form.next_focus = Some(NAME);
        } else {
            self.sign_in.form.next_focus = Some("Finish");
        }
    }

    fn save_connection_name(&mut self) {
        if let [i] = *self.sign_in.form.added {
            let name = self.sign_in.form.name.trim();
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
            .sign_in
            .form
            .added
            .last()
            .copied()
            .or_else(|| self.servers.len().checked_sub(1));
        self.choose_source(None);
        self.adding = false;
        if from_settings {
            self.sign_in.form.next_focus = Some("Add server");
        } else if let Some(i) = last {
            self.activate(i);
            self.overview = true;
        }
    }

    fn connection_success(&mut self, ui: &mut egui::Ui) {
        if let [i] = *self.sign_in.form.added {
            let help = format!(
                "Shown in the sidebar as {} · name.",
                self.servers[i].session.kind.plugin().name
            );
            let field = text_field(
                ui,
                NAME,
                &mut self.sign_in.form.name,
                false,
                &help,
                None,
                None,
            );
            self.sign_in.form.focus(NAME, &field);
            if field.lost_focus() {
                self.save_connection_name();
            }
        }
        for &i in &self.sign_in.form.added {
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
                    let text = if n == 0 {
                        format!(
                            "Found no {} libraries",
                            server.session.kind.plugin().library_nouns()
                        )
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
            self.sign_in.form.focus("Finish", &finish);
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
    let description = kind.plugin().description;
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
        self.sign_in.login = None; // Dropping Login cancels the Plex worker.
        self.sign_in.login_url = None;
        self.sign_in.quick_code = None;
        self.sign_in.token.clear();
        self.sign_in.username.clear();
        self.sign_in.password.clear();
        self.sign_in.server.clear();
    }

    pub(super) fn choose_source(&mut self, kind: Option<ServerKind>) {
        let previous = self.sign_in.form.kind.unwrap_or(ServerKind::Plex);
        self.sign_in.generation.fetch_add(1, Ordering::Relaxed);
        self.sign_in.busy = false;
        self.clear_sign_in();
        self.sign_in.error = None;
        self.sign_in.remember = cfg!(windows);
        self.sign_in.form = Form {
            kind,
            next_focus: Some(match kind {
                None => previous.plugin().name,
                Some(kind) if kind.plugin().offers(SignInMethod::Account) => PLEX_SIGN_IN,
                Some(_) => ADDRESS,
            }),
            ..Form::default()
        };
        if !cfg!(test) && kind == Some(ServerKind::Plex) && !self.adding && self.servers.is_empty()
        {
            self.sign_in.server = std::env::var("PLEX_URL").unwrap_or_default();
            self.sign_in.token = std::env::var("PLEX_TOKEN").unwrap_or_default();
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
        if !self.sign_in.form.choices.is_empty() {
            Step::PlexChoices
        } else if !self.sign_in.form.added.is_empty() {
            Step::Connected
        } else if self.sign_in.login.is_some() {
            Step::Browser
        } else if self.sign_in.quick_code.is_some() {
            Step::QuickConnect
        } else {
            self.sign_in.form.kind.map_or(Step::Types, Step::Form)
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
            Step::Form(_) | Step::Browser => format!("Connect to {}", self.sign_in_plugin().name),
            Step::QuickConnect => format!("{} Quick Connect", self.sign_in_plugin().name),
            Step::PlexChoices => format!("Choose {} servers", self.sign_in_plugin().name),
            Step::Connected if self.sign_in.form.added.len() == 1 => {
                format!(
                    "{} is connected",
                    self.servers[self.sign_in.form.added[0]].name()
                )
            }
            Step::Connected => format!("{} servers connected", self.sign_in.form.added.len()),
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
                } else if let Some(kind) = self.sign_in.form.kind {
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
            .sign_in
            .error
            .as_ref()
            .or(self.sign_in.form.credential_error.as_ref())
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
                            self.sign_in.error = None;
                            self.sign_in.form.credential_error = None;
                        }
                    });
                });
        }
        if self.sign_in.form.issues.len() > 1 {
            egui::Frame::new()
                .fill(Color32::from_rgb(44, 26, 26))
                .inner_margin(12)
                .corner_radius(0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Check the highlighted fields").color(ERROR_TEXT));
                    ui.horizontal_wrapped(|ui| {
                        for &field in self.sign_in.form.issues.keys() {
                            if ui.link(field).clicked() {
                                self.sign_in.form.next_focus = Some(field);
                            }
                        }
                    });
                });
        }
        match step {
            Step::Types => self.source_chooser(ui),
            Step::Form(kind) if kind.plugin().offers(SignInMethod::Account) => {
                self.plex_form(ui, ctx)
            }
            Step::Form(_) => self.password_form(ui, ctx),
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
            self.sign_in.form.focus(kind.plugin().name, &button);
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
