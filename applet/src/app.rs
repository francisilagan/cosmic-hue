//! The panel applet: a bar button and a popup with rooms, lights, scenes,
//! pairing and wallpaper sync. All bridge work goes through the backend.

use std::collections::{HashMap, HashSet};

use cosmic::app::{Core, Task};
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Color, Length, Rectangle, Subscription};
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::widget::{self, button, container, divider, icon, settings, text, toggler};
use cosmic::{Element, theme};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::backend;
use crate::model::{Group, Snapshot, Wallpaper};

pub const APP_ID: &str = "io.github.francisilagan.CosmicHue";
const BULB_ICON: &[u8] = include_bytes!("../res/icons/cosmic-hue-symbolic.svg");

#[derive(Default)]
pub struct HueApplet {
    core: Core,
    popup: Option<Id>,
    tx: Option<UnboundedSender<String>>,
    next_id: u64,
    backend_down: Option<String>,
    conn: String,
    conn_detail: String,
    snapshot: Option<Snapshot>,
    wallpaper: Wallpaper,
    expanded: HashSet<String>,
    /// Slider values while the user drags, keyed by "g:<grouped_light>" or "l:<light>".
    drag: HashMap<String, f32>,
    host: String,
    pair_state: String,
    pair_message: String,
    /// The id of the running pair.discover, pair.start or pair.importOpenhue command.
    pair_cmd: Option<u64>,
    discovered: Vec<String>,
    notice: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    PopupClosed(Id),
    Surface(cosmic::surface::Action<Message>),
    Backend(backend::Event),
    ToggleAll,
    Refresh,
    Expand(String),
    GroupOn(String, bool),
    GroupBrightness(String, f32),
    GroupBrightnessDone(String),
    LightOn(String, bool),
    LightBrightness(String, f32),
    LightBrightnessDone(String),
    Scene(String),
    Host(String),
    PairStart,
    PairCancel,
    Discover,
    ImportOpenhue,
    Trust,
    Forget,
    WallpaperEnabled(bool),
    WallpaperTurnOn(bool),
    WallpaperSync,
    DismissNotice,
}

impl HueApplet {
    /// Send a command; returns its id, or None when the backend is not running.
    fn send(&mut self, cmd: &str, mut fields: Value) -> Option<u64> {
        let Some(tx) = &self.tx else {
            self.notice = Some("The backend is not running.".into());
            return None;
        };
        self.next_id += 1;
        fields["id"] = json!(self.next_id);
        fields["cmd"] = json!(cmd);
        if tx.send(fields.to_string()).is_err() {
            self.tx = None;
            return None;
        }
        Some(self.next_id)
    }

    /// Send a pairing command and show `status` until its result arrives.
    fn send_pair(&mut self, cmd: &str, fields: Value, status: &str) {
        self.pair_cmd = self.send(cmd, fields);
        if self.pair_cmd.is_some() {
            self.pair_message = status.to_string();
        }
    }

    fn on_line(&mut self, v: Value) {
        match v["event"].as_str().unwrap_or_default() {
            "conn" => {
                self.conn = v["state"].as_str().unwrap_or_default().to_string();
                self.conn_detail = v["detail"].as_str().unwrap_or_default().to_string();
                if matches!(self.conn.as_str(), "unpaired") {
                    self.snapshot = None;
                }
            }
            "state" => {
                if let Ok(s) = serde_json::from_value::<Snapshot>(v["data"].clone()) {
                    self.snapshot = Some(s);
                }
            }
            "wallpaper" => {
                if let Ok(w) = serde_json::from_value::<Wallpaper>(v) {
                    self.wallpaper = w;
                }
            }
            "pair" => {
                self.pair_state = v["state"].as_str().unwrap_or_default().to_string();
                self.pair_message = match (self.pair_state.as_str(), v["secondsLeft"].as_u64()) {
                    ("waiting_button", Some(s)) => {
                        format!("Press the round link button on the bridge ({s} s left).")
                    }
                    ("waiting_button", None) => "Press the round link button on the bridge.".into(),
                    ("paired", _) => "Paired.".into(),
                    ("cancelled", _) => "Pairing cancelled.".into(),
                    _ => v["message"]
                        .as_str()
                        .unwrap_or("Pairing failed.")
                        .to_string(),
                };
            }
            "discovered" => {
                self.discovered = v["bridges"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|b| b["host"].as_str().map(str::to_string))
                    .collect();
                self.pair_message = match self.discovered.as_slice() {
                    [] => "No bridge found. Type the address of the bridge.".into(),
                    [one] => format!("Found a bridge at {one}. Click Pair."),
                    many => format!("Found {} bridges. Click one, then click Pair.", many.len()),
                };
                if !self.discovered.contains(&self.host) {
                    self.host = self.discovered.first().cloned().unwrap_or_default();
                }
            }
            "result" if self.pair_cmd.is_some() && v["id"].as_u64() == self.pair_cmd => {
                self.pair_cmd = None;
                if v["ok"] == json!(false) {
                    let message = v["message"].as_str().unwrap_or("it failed");
                    self.pair_message = format!("That did not work: {message}.");
                }
            }
            "result" if v["ok"] == json!(false) => {
                let message = v["message"].as_str().unwrap_or_default();
                let code = v["code"].as_str().unwrap_or("error");
                self.notice = Some(if message.is_empty() {
                    code.to_string()
                } else {
                    message.to_string()
                });
            }
            _ => {}
        }
    }

    fn set_group(&mut self, grouped_light: String, on: bool) {
        if let Some(s) = &mut self.snapshot {
            s.assume_group_on(&grouped_light, on);
        }
        self.send(
            "set",
            json!({"kind": "group", "ids": [grouped_light], "on": on}),
        );
    }

    /// A fresh popup action. It must be built per click: libcosmic takes sole
    /// ownership of the boxed settings, so a cloned action would be rejected.
    fn open_popup(offset: cosmic::iced::Vector, bounds: Rectangle) -> Message {
        Message::Surface(app_popup::<HueApplet>(
            |_| Default::default(),
            move |state: &mut HueApplet| {
                let new_id = Id::unique();
                state.popup = Some(new_id);
                let mut popup = state.core.applet.get_popup_settings(
                    state.core.main_window_id().unwrap(),
                    new_id,
                    None,
                    None,
                    None,
                );
                popup.positioner.size_limits = cosmic::iced::Limits::NONE
                    .min_width(360.0)
                    .max_width(400.0)
                    .min_height(1.0)
                    .max_height(760.0);
                popup.positioner.anchor_rect = Rectangle {
                    x: (bounds.x - offset.x) as i32,
                    y: (bounds.y - offset.y) as i32,
                    width: bounds.width as i32,
                    height: bounds.height as i32,
                };
                popup
            },
            Some(Box::new(|state: &HueApplet| {
                Element::from(state.core.applet.popup_container(state.popup_view()))
                    .map(cosmic::Action::App)
            })),
        ))
    }

    fn popup_view(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        let mut col = widget::Column::new()
            .spacing(space.space_xs)
            .padding([space.space_xs, 0]);

        col = col.push(self.header());
        if let Some(notice) = &self.notice {
            col = col.push(
                widget::Row::new()
                    .push(text::caption(notice.clone()).width(Length::Fill))
                    .push(
                        button::icon(icon::from_name("window-close-symbolic"))
                            .on_press(Message::DismissNotice),
                    )
                    .align_y(Alignment::Center)
                    .padding([0, space.space_s]),
            );
        }
        col = col.push(divider::horizontal::default());

        let body: Element<'_, Message> = if let Some(reason) = &self.backend_down {
            self.status_card(
                "The Hue backend is not running.",
                &format!("{reason}. Install the backend with `just install`, then make sure python3 is installed."),
                None,
            )
        } else {
            match self.conn.as_str() {
                "unpaired" | "pairing" => self.pairing_view(),
                "unauthorized" => self.status_card(
                    "The bridge rejected the key.",
                    "This can occur after a bridge reset. Pair again.",
                    Some(("Pair again", Message::Forget)),
                ),
                "cert_changed" => self.cert_view(),
                "unreachable" if self.snapshot.is_none() => self.status_card(
                    "The bridge is unreachable.",
                    "Make sure the bridge has power and is on your network. The applet tries again.",
                    Some(("Retry", Message::Refresh)),
                ),
                _ => match &self.snapshot {
                    Some(s) => self.lights_view(s),
                    None => self.status_card("Connecting to the bridge…", "", None),
                },
            }
        };
        col = col.push(widget::scrollable(body).height(Length::Shrink));
        col.into()
    }

    fn header(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        let title = match &self.snapshot {
            Some(s) if !s.bridge.name.is_empty() => s.bridge.name.clone(),
            _ => "Hue".to_string(),
        };
        let mut row = widget::Row::new()
            .push(text::heading(title).width(Length::Fill))
            .push(
                button::icon(icon::from_name("view-refresh-symbolic"))
                    .tooltip("Refresh")
                    .on_press(Message::Refresh),
            )
            .align_y(Alignment::Center)
            .spacing(space.space_xs)
            .padding([0, space.space_s]);
        if let Some(s) = &self.snapshot
            && s.home_group.is_some()
        {
            row = row.push(toggler(s.any_on()).on_toggle(|_| Message::ToggleAll));
        }
        row.into()
    }

    fn status_card<'a>(
        &'a self,
        title: &str,
        body: &str,
        action: Option<(&str, Message)>,
    ) -> Element<'a, Message> {
        let space = theme::spacing();
        let mut col = widget::Column::new()
            .spacing(space.space_xs)
            .padding([space.space_xs, space.space_s])
            .push(text::body(title.to_string()));
        if !body.is_empty() {
            col = col.push(text::caption(body.to_string()));
        }
        if let Some((label, msg)) = action {
            col = col.push(button::standard(label.to_string()).on_press(msg));
        }
        col.into()
    }

    fn cert_view(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        widget::Column::new()
            .spacing(space.space_xs)
            .padding([space.space_xs, space.space_s])
            .push(text::body("The certificate of the bridge changed."))
            .push(text::caption(
                "This is normal after a bridge reset or replacement. If you did not do one of these, do not trust it.",
            ))
            .push(
                widget::Row::new()
                    .spacing(space.space_xs)
                    .push(button::destructive("Trust the new certificate").on_press(Message::Trust))
                    .push(button::standard("Forget bridge").on_press(Message::Forget)),
            )
            .into()
    }

    fn pairing_view(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        let pairing = self.conn == "pairing";
        let mut col = widget::Column::new()
            .spacing(space.space_s)
            .padding([space.space_xs, space.space_s])
            .push(text::body("Pair with your Hue bridge"))
            .push(
                widget::text_input(
                    "Bridge address, for example 192.168.1.2",
                    self.host.as_str(),
                )
                .on_input(Message::Host)
                .on_submit(|_| Message::PairStart),
            );
        let busy = self.pair_cmd.is_some();
        let mut buttons = widget::Row::new().spacing(space.space_xs);
        if pairing {
            buttons = buttons.push(button::standard("Cancel").on_press(Message::PairCancel));
        } else {
            buttons = buttons
                .push(button::suggested("Pair").on_press_maybe(
                    (!busy && !self.host.trim().is_empty()).then_some(Message::PairStart),
                ))
                .push(
                    button::standard("Find my bridge")
                        .on_press_maybe((!busy).then_some(Message::Discover)),
                );
        }
        col = col.push(buttons);
        if !pairing {
            col = col.push(
                button::link("Use my openhue login")
                    .on_press_maybe((!busy).then_some(Message::ImportOpenhue)),
            );
        }
        if !self.pair_message.is_empty() {
            col = col.push(text::caption(self.pair_message.clone()));
        }
        if self.discovered.len() > 1 {
            for host in &self.discovered {
                col = col.push(button::link(host.clone()).on_press(Message::Host(host.clone())));
            }
        }
        col = col.push(text::caption(
            "\"Find my bridge\" searches your local network. Only if it finds nothing there, it asks the Philips discovery service. After you click Pair, press the link button on the bridge within 40 seconds.",
        ));
        col.into()
    }

    fn lights_view<'a>(&'a self, s: &'a Snapshot) -> Element<'a, Message> {
        let space = theme::spacing();
        let mut col = widget::Column::new().spacing(space.space_xxs);
        if s.groups.is_empty() {
            col = col.push(self.status_card("No rooms", "Make rooms in the Hue app.", None));
        }
        for g in s.sorted_groups() {
            col = col.push(self.group_view(s, g));
        }
        col = col.push(divider::horizontal::default());
        col = col.push(self.wallpaper_view());
        col.into()
    }

    fn group_view<'a>(&'a self, s: &'a Snapshot, g: &'a Group) -> Element<'a, Message> {
        let space = theme::spacing();
        let expanded = self.expanded.contains(&g.id);
        let key = format!("g:{}", g.group);
        let value = self
            .drag
            .get(&key)
            .copied()
            .unwrap_or_else(|| s.group_brightness(g));
        let gl = g.group.clone();
        let gl2 = g.group.clone();
        let gl3 = g.group.clone();

        let chevron = if expanded {
            "go-down-symbolic"
        } else {
            "go-next-symbolic"
        };
        let title = widget::Row::new()
            .push(
                button::custom(
                    widget::Row::new()
                        .push(icon::from_name(chevron).size(16))
                        .push(text::body(g.name.clone()))
                        .spacing(space.space_xxs)
                        .align_y(Alignment::Center),
                )
                .class(theme::Button::Text)
                .on_press(Message::Expand(g.id.clone()))
                .width(Length::Fill),
            )
            .push(toggler(g.on).on_toggle(move |on| Message::GroupOn(gl.clone(), on)))
            .align_y(Alignment::Center);

        let mut col = widget::Column::new()
            .push(title)
            .padding([0, space.space_s]);
        if g.on {
            col = col.push(
                widget::slider(1.0..=100.0, value.max(1.0), move |v| {
                    Message::GroupBrightness(gl2.clone(), v)
                })
                .on_release(Message::GroupBrightnessDone(gl3)),
            );
        }
        if expanded {
            let scenes: Vec<_> = g
                .scene_ids
                .iter()
                .filter_map(|id| s.scenes.get(id))
                .collect();
            if !scenes.is_empty() {
                let mut sorted = scenes;
                sorted.sort_by_key(|sc| sc.name.to_lowercase());
                let chips: Vec<Element<'a, Message>> = sorted
                    .into_iter()
                    .map(|sc| {
                        let b = if sc.active {
                            button::suggested(sc.name.clone())
                        } else {
                            button::standard(sc.name.clone())
                        };
                        b.on_press(Message::Scene(sc.id.clone())).into()
                    })
                    .collect();
                let chips = widget::flex_row(chips)
                    .row_spacing(space.space_xxs)
                    .column_spacing(space.space_xxs);
                col = col.push(container(chips).padding([space.space_xxs, 0]));
            }
            let mut lights = settings::section();
            for l in s.group_lights(g) {
                let key = format!("l:{}", l.id);
                let id1 = l.id.clone();
                let id2 = l.id.clone();
                let id3 = l.id.clone();
                let swatch = swatch(l.hex.as_deref().filter(|_| l.on && l.color), 12.0);
                let mut name_row = widget::Row::new()
                    .push(swatch)
                    .push(text::body(l.name.clone()).width(Length::Fill))
                    .spacing(space.space_xxs)
                    .align_y(Alignment::Center);
                if !l.reachable {
                    name_row = name_row.push(text::caption("unreachable"));
                }
                name_row = name_row
                    .push(toggler(l.on).on_toggle(move |on| Message::LightOn(id1.clone(), on)));
                let mut lc = widget::Column::new()
                    .push(name_row)
                    .spacing(space.space_xxs);
                if l.on && l.dimmable {
                    let v = self.drag.get(&key).copied().unwrap_or(l.brightness);
                    lc = lc.push(
                        widget::slider(1.0..=100.0, v.max(1.0), move |v| {
                            Message::LightBrightness(id2.clone(), v)
                        })
                        .on_release(Message::LightBrightnessDone(id3)),
                    );
                }
                lights = lights.add(lc);
            }
            col = col.push(lights);
        }
        col.into()
    }

    fn wallpaper_view(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        let w = &self.wallpaper;
        let mut swatches = widget::Row::new().spacing(space.space_xxs);
        for hex in &w.palette {
            swatches = swatches.push(swatch(Some(hex.as_str()), 18.0));
        }
        let status = if !w.config.enabled {
            "Off. The palette below comes from your COSMIC wallpaper.".to_string()
        } else if w.eligible == 0 {
            "No colour light can be reached. Turn the lights on, or turn on \"Also switch lights on\".".to_string()
        } else {
            format!("{} colour lights follow the wallpaper.", w.eligible)
        };
        widget::Column::new()
            .spacing(space.space_xxs)
            .padding([space.space_xs, space.space_s])
            .push(
                widget::Row::new()
                    .push(text::body("Wallpaper sync").width(Length::Fill))
                    .push(toggler(w.config.enabled).on_toggle(Message::WallpaperEnabled))
                    .align_y(Alignment::Center),
            )
            .push(text::caption(status))
            .push(swatches)
            .push(
                widget::Row::new()
                    .push(text::caption("Also switch lights on").width(Length::Fill))
                    .push(toggler(w.config.turn_on).on_toggle(Message::WallpaperTurnOn))
                    .align_y(Alignment::Center),
            )
            .push(button::standard("Sync now").on_press(Message::WallpaperSync))
            .into()
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(Color::from_rgb8((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// A small rounded square of one colour (a faint outline when there is none).
fn swatch<'a>(hex: Option<&str>, size: f32) -> Element<'a, Message> {
    let fill = hex.and_then(parse_hex);
    container(
        widget::Space::new()
            .width(Length::Fixed(size))
            .height(Length::Fixed(size)),
    )
    .class(theme::Container::custom(move |t| {
        let outline = t.cosmic().palette.neutral_5;
        container::Style {
            background: fill.map(Into::into),
            border: cosmic::iced::Border {
                radius: (size / 4.0).into(),
                width: if fill.is_some() { 0.0 } else { 1.0 },
                color: outline.into(),
            },
            ..Default::default()
        }
    }))
    .into()
}

impl cosmic::Application for HueApplet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        (
            Self {
                core,
                ..Default::default()
            },
            Task::none(),
        )
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(backend::run).map(Message::Backend)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::Surface(a) => return cosmic::task::message(cosmic::Action::Surface(a)),
            Message::Backend(backend::Event::Started(tx)) => {
                self.tx = Some(tx);
                self.backend_down = None;
            }
            Message::Backend(backend::Event::Line(v)) => self.on_line(v),
            Message::Backend(backend::Event::Stopped(reason)) => {
                self.tx = None;
                self.backend_down = Some(reason);
            }
            Message::ToggleAll => {
                if let Some(s) = &self.snapshot
                    && let Some(home) = s.home_group.clone()
                {
                    let on = !s.any_on();
                    self.set_group(home, on);
                }
            }
            Message::Refresh => {
                self.notice = None;
                self.send("refresh", json!({}));
            }
            Message::Expand(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Message::GroupOn(gl, on) => self.set_group(gl, on),
            Message::GroupBrightness(gl, v) => {
                self.drag.insert(format!("g:{gl}"), v);
            }
            Message::GroupBrightnessDone(gl) => {
                if let Some(v) = self.drag.remove(&format!("g:{gl}")) {
                    let pct = v.round().clamp(1.0, 100.0);
                    if let Some(s) = &mut self.snapshot {
                        let ids: Vec<String> = s
                            .groups
                            .iter()
                            .filter(|g| g.group == gl)
                            .flat_map(|g| g.light_ids.clone())
                            .collect();
                        for id in ids {
                            if let Some(l) = s.lights.get_mut(&id)
                                && l.on
                            {
                                l.brightness = pct;
                            }
                        }
                    }
                    self.send("set", json!({"kind": "group", "ids": [gl], "pct": pct}));
                }
            }
            Message::LightOn(id, on) => {
                if let Some(s) = &mut self.snapshot
                    && let Some(l) = s.lights.get_mut(&id)
                {
                    l.on = on;
                }
                self.send("set", json!({"kind": "light", "ids": [id], "on": on}));
            }
            Message::LightBrightness(id, v) => {
                self.drag.insert(format!("l:{id}"), v);
            }
            Message::LightBrightnessDone(id) => {
                if let Some(v) = self.drag.remove(&format!("l:{id}")) {
                    let pct = v.round().clamp(1.0, 100.0);
                    if let Some(l) = self.snapshot.as_mut().and_then(|s| s.lights.get_mut(&id)) {
                        l.brightness = pct;
                    }
                    self.send("set", json!({"kind": "light", "ids": [id], "pct": pct}));
                }
            }
            Message::Scene(id) => {
                self.send("scene.recall", json!({"id": id, "mode": "active"}));
            }
            Message::Host(h) => self.host = h,
            Message::PairStart => {
                let host = self.host.trim().to_string();
                if !host.is_empty() {
                    self.send_pair("pair.start", json!({"host": host}), "Starting…");
                }
            }
            Message::PairCancel => {
                self.send("pair.cancel", json!({}));
            }
            Message::Discover => {
                self.discovered.clear();
                self.send_pair(
                    "pair.discover",
                    json!({}),
                    "Looking for bridges on your network…",
                );
            }
            Message::ImportOpenhue => {
                self.send_pair(
                    "pair.importOpenhue",
                    json!({}),
                    "Reading the openhue config…",
                );
            }
            Message::Trust => {
                self.send("pair.trust", json!({}));
            }
            Message::Forget => {
                self.send("pair.forget", json!({}));
            }
            Message::WallpaperEnabled(on) => {
                self.wallpaper.config.enabled = on;
                self.send("wallpaper.config", json!({"enabled": on}));
            }
            Message::WallpaperTurnOn(on) => {
                self.wallpaper.config.turn_on = on;
                self.send("wallpaper.config", json!({"turnOn": on}));
            }
            Message::WallpaperSync => {
                self.send("wallpaper.sync", json!({}));
            }
            Message::DismissNotice => self.notice = None,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let popup = self.popup;
        let btn = self
            .core
            .applet
            .icon_button_from_handle(icon::from_svg_bytes(BULB_ICON).symbolic(true))
            .on_press_with_rectangle(move |offset, bounds| match popup {
                Some(id) => Message::Surface(destroy_popup(id)),
                None => HueApplet::open_popup(offset, bounds),
            });
        let area = widget::mouse_area(btn)
            .on_right_press(Message::ToggleAll)
            .on_middle_press(Message::Refresh);
        let tip = match &self.snapshot {
            Some(s) if s.any_on() => "Hue: lights on",
            Some(_) => "Hue: lights off",
            None => "Hue",
        };
        Element::from(self.core.applet.applet_tooltip::<Message>(
            area,
            tip,
            self.popup.is_some(),
            Message::Surface,
            None,
        ))
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        widget::text("").into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
