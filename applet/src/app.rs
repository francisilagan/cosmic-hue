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

mod manage;

use crate::backend;
use crate::colour;
use crate::model::{Group, Snapshot, Wallpaper};

pub const APP_ID: &str = "io.github.francisilagan.CosmicHue";
/// Set by build.rs: the local override icon if there is one, else the bundled one.
const BULB_ICON: &[u8] = include_bytes!(env!("COSMIC_HUE_ICON"));

/// What a colour control acts on: a light id, or a grouped_light id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Light(String),
    Group(String),
}

impl Target {
    fn key(&self) -> String {
        match self {
            Target::Light(id) => format!("l:{id}"),
            Target::Group(id) => format!("g:{id}"),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Target::Light(_) => "light",
            Target::Group(_) => "group",
        }
    }

    fn id(&self) -> &str {
        match self {
            Target::Light(id) | Target::Group(id) => id,
        }
    }
}

/// The tabs of the popup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Lights,
    Scenes,
    Automations,
    Devices,
}

/// The tabs of a colour control.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TunerMode {
    #[default]
    Colour,
    White,
    Effects,
}

/// How long a sunrise or sunset runs.
const TIMED_EFFECT_MS: u64 = 30 * 60 * 1000;

/// "underwater" -> "Underwater", "no_effect" -> "None".
fn effect_label(id: &str) -> String {
    if id == "no_effect" {
        return "None".into();
    }
    let words = id.replace('_', " ");
    let mut c = words.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// What the colour control of a target can do, and what it shows now.
struct TunerInfo {
    /// Effects and timed effects apply to one light only.
    effects: Vec<String>,
    effect: String,
    timed: Vec<String>,
    timed_effect: String,
    color: bool,
    ct: bool,
    mirek_min: f32,
    mirek_max: f32,
    hex: Option<String>,
    mirek: Option<f32>,
}

/// Hues (degrees) of the colour presets.
const PRESET_HUES: [f32; 9] = [0.0, 25.0, 45.0, 60.0, 120.0, 180.0, 220.0, 270.0, 320.0];
/// White presets: (label, mirek).
const WHITE_PRESETS: [(&str, f32); 4] = [
    ("Warm", 454.0),
    ("Soft", 370.0),
    ("Neutral", 250.0),
    ("Cool", 153.0),
];

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
    /// The open colour control (one at a time) and whether it shows whites.
    tuner: Option<Target>,
    tuner_mode: TunerMode,
    tab: Tab,
    /// Rename drafts keyed "rtype:id".
    names: HashMap<String, String>,
    /// A destructive action armed by its first click: (key, when).
    confirm: Option<(String, std::time::Instant)>,
    scene_name: String,
    scene_group: Option<String>,
    zone_name: String,
    zone_lights: HashSet<String>,
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
    ToggleTuner(Target),
    TunerMode(TunerMode),
    Tab(Tab),
    Effect(String, String),
    TimedEffect(String, String),
    SmartScene(String, bool),
    Automation(String, bool),
    EntertainmentStop(String),
    Enable(String, String, bool),
    WallpaperTarget(String),
    SceneRecall(String, String),
    SceneSpeed(String, f32),
    SceneSpeedDone(String),
    SceneAutoDynamic(String, bool),
    SceneResave(String),
    SceneDelete(String),
    SceneGroup(String),
    SceneName(String),
    SceneCreate,
    RenameInput(String, String),
    /// (rtype, id, current name)
    RenameSubmit(String, String, String),
    ZoneName(String),
    ZoneLight(String),
    ZoneCreate,
    ZoneMember(String, String),
    ZoneDelete(String),
    GroupEffect(String, String),
    Identify(String),
    Powerup(String, String),
    ForgetBridge,
    Hue(Target, f32),
    Saturation(Target, f32),
    ColourDone(Target),
    Mirek(Target, f32),
    MirekDone(Target),
    SetHex(Target, String),
    SetMirek(Target, f32),
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
        let main = self.backend_down.is_none()
            && self.snapshot.is_some()
            && !matches!(
                self.conn.as_str(),
                "unpaired" | "pairing" | "unauthorized" | "cert_changed"
            );
        if main {
            col = col.push(
                widget::Row::new()
                    .spacing(space.space_xxs)
                    .padding([0, space.space_s])
                    .push(tab_button(
                        "Lights",
                        self.tab == Tab::Lights,
                        Message::Tab(Tab::Lights),
                    ))
                    .push(tab_button(
                        "Scenes",
                        self.tab == Tab::Scenes,
                        Message::Tab(Tab::Scenes),
                    ))
                    .push(tab_button(
                        "Auto",
                        self.tab == Tab::Automations,
                        Message::Tab(Tab::Automations),
                    ))
                    .push(tab_button(
                        "Devices",
                        self.tab == Tab::Devices,
                        Message::Tab(Tab::Devices),
                    )),
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
                "unreachable" if self.snapshot.is_none() => self.recovery_view(),
                _ => match &self.snapshot {
                    Some(s) => match self.tab {
                        Tab::Lights => self.lights_view(s),
                        Tab::Scenes => self.scenes_view(s),
                        Tab::Automations => self.automations_view(s),
                        Tab::Devices => self.devices_view(s),
                    },
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
        col = col.push(self.new_zone_view(s));
        col = col.push(divider::horizontal::default());
        col = col.push(self.wallpaper_view(s));
        col.into()
    }

    fn automations_view<'a>(&'a self, s: &'a Snapshot) -> Element<'a, Message> {
        let space = theme::spacing();
        let mut col = widget::Column::new()
            .spacing(space.space_xs)
            .padding([space.space_xs, space.space_s]);
        col = col.push(text::heading("Automations"));
        if s.automations.is_empty() {
            col = col.push(text::caption(
                "The bridge has no automations. Make them in the Hue app.",
            ));
        } else {
            let mut list: Vec<_> = s.automations.iter().collect();
            list.sort_by_key(|a| a.name.to_lowercase());
            let mut section = settings::section();
            for a in list {
                let id = a.id.clone();
                let mut info = widget::Column::new().push(text::body(a.name.clone()));
                let detail = match (a.script.is_empty(), a.status.as_str()) {
                    (true, "") => String::new(),
                    (true, st) => effect_label(st),
                    (false, "") => a.script.clone(),
                    (false, st) => format!("{} · {}", a.script, effect_label(st)),
                };
                if !detail.is_empty() {
                    info = info.push(text::caption(detail));
                }
                section = section.add(
                    widget::Row::new()
                        .push(info.width(Length::Fill))
                        .push(
                            toggler(a.enabled)
                                .on_toggle(move |on| Message::Automation(id.clone(), on)),
                        )
                        .align_y(Alignment::Center),
                );
            }
            col = col.push(section);
        }
        col = col.push(text::heading("Entertainment areas"));
        if s.entertainment.is_empty() {
            col = col.push(text::caption("The bridge has no entertainment areas."));
        } else {
            let mut section = settings::section();
            for e in &s.entertainment {
                let active = e.status == "active";
                let detail = format!(
                    "{} {} · {}",
                    e.lights,
                    if e.lights == 1 { "light" } else { "lights" },
                    if active { "Streaming" } else { "Not streaming" }
                );
                let mut row = widget::Row::new()
                    .push(
                        widget::Column::new()
                            .push(text::body(e.name.clone()))
                            .push(text::caption(detail))
                            .width(Length::Fill),
                    )
                    .align_y(Alignment::Center);
                if active {
                    row = row.push(
                        button::destructive("Stop")
                            .on_press(Message::EntertainmentStop(e.id.clone())),
                    );
                }
                section = section.add(row);
            }
            col = col.push(section);
        }
        col.into()
    }

    fn devices_view<'a>(&'a self, s: &'a Snapshot) -> Element<'a, Message> {
        let space = theme::spacing();
        let col = widget::Column::new()
            .spacing(space.space_xs)
            .padding([space.space_xs, space.space_s])
            .push(self.bridge_card(s));
        if s.devices.is_empty() {
            return col
                .push(text::caption("The bridge reports no accessories."))
                .into();
        }
        let mut section = settings::section();
        for d in &s.devices {
            let mut title = widget::Row::new()
                .push(text::body(d.name.clone()).width(Length::Fill))
                .spacing(space.space_xxs)
                .align_y(Alignment::Center);
            if !d.reachable {
                title = title.push(text::caption("offline"));
            } else if d.battery.is_none() {
                title = title.push(text::caption("online"));
            }
            if let Some(b) = d.battery.filter(|_| d.reachable) {
                let name = match b as u32 {
                    0..=10 => "battery-level-10-symbolic",
                    11..=30 => "battery-level-30-symbolic",
                    31..=60 => "battery-level-60-symbolic",
                    61..=90 => "battery-level-90-symbolic",
                    _ => "battery-level-100-symbolic",
                };
                title = title
                    .push(icon::from_name(name).size(16))
                    .push(text::caption(if b <= 15.0 {
                        format!("{}% · low", b.round())
                    } else {
                        format!("{}%", b.round())
                    }));
            }
            let mut detail = vec![];
            if d.bridge {
                detail.push("Bridge".to_string());
            }
            if !d.product.is_empty() && d.product != d.name {
                detail.push(d.product.clone());
            }
            if !d.software.is_empty() {
                detail.push(format!("software {}", d.software));
            }
            let mut dc = widget::Column::new().spacing(space.space_xxxs).push(title);
            if !detail.is_empty() {
                dc = dc.push(text::caption(detail.join(" · ")));
            }
            if !d.reachable {
                dc = dc.push(text::caption("Unreachable"));
            }
            match d.update.as_deref() {
                None | Some("no_update") | Some("") => {}
                Some(state) => {
                    dc = dc.push(text::caption(format!(
                        "Firmware update: {}",
                        effect_label(state)
                    )));
                }
            }
            for r in &d.readings {
                dc = dc.push(
                    widget::Row::new()
                        .push(text::caption(r.label.clone()).width(Length::Fill))
                        .push(text::caption(r.value.clone())),
                );
            }
            for c in &d.controls {
                let (rtype, id) = (c.rtype.clone(), c.id.clone());
                dc =
                    dc.push(
                        widget::Row::new()
                            .push(text::caption(c.label.clone()).width(Length::Fill))
                            .push(toggler(c.enabled).on_toggle(move |on| {
                                Message::Enable(rtype.clone(), id.clone(), on)
                            }))
                            .align_y(Alignment::Center),
                    );
            }
            section = section.add(dc);
        }
        col.push(section).into()
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
                        .push_maybe((g.kind == "zone").then(|| text::caption("zone")))
                        .spacing(space.space_xxs)
                        .align_y(Alignment::Center),
                )
                .class(theme::Button::Text)
                .on_press(Message::Expand(g.id.clone()))
                .width(Length::Fill),
            )
            .push_maybe(
                g.on.then(|| self.tuner_button(s, &Target::Group(g.group.clone())))
                    .flatten(),
            )
            .push(toggler(g.on).on_toggle(move |on| Message::GroupOn(gl.clone(), on)))
            .spacing(space.space_xxs)
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
            let target = Target::Group(g.group.clone());
            if self.tuner.as_ref() == Some(&target) {
                col = col.push(self.tuner_view(s, &target));
            }
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
            let mut routines: Vec<_> = g
                .smart_scene_ids
                .iter()
                .filter_map(|id| s.smart_scenes.get(id))
                .collect();
            if !routines.is_empty() {
                routines.sort_by_key(|r| r.name.to_lowercase());
                let chips: Vec<Element<'a, Message>> = routines
                    .into_iter()
                    .map(|r| {
                        tab_button(
                            r.name.clone(),
                            r.active,
                            Message::SmartScene(r.id.clone(), !r.active),
                        )
                        .into()
                    })
                    .collect();
                col = col.push(text::caption("Routines")).push(
                    container(
                        widget::flex_row(chips)
                            .row_spacing(space.space_xxs)
                            .column_spacing(space.space_xxs),
                    )
                    .padding([space.space_xxs, 0]),
                );
            }
            let mut lights = settings::section();
            for l in s.group_lights(g) {
                let key = format!("l:{}", l.id);
                let id1 = l.id.clone();
                let id2 = l.id.clone();
                let id3 = l.id.clone();
                let swatch = swatch(l.hex.as_deref().filter(|_| l.on), 12.0);
                let target = Target::Light(l.id.clone());
                let mut name_row = widget::Row::new()
                    .push(swatch)
                    .push(text::body(l.name.clone()).width(Length::Fill))
                    .spacing(space.space_xxs)
                    .align_y(Alignment::Center);
                if !l.reachable {
                    name_row = name_row.push(text::caption("unreachable"));
                }
                if let Some(effect) = manage::running_effect(l) {
                    name_row = name_row.push(text::caption(effect));
                }
                if l.on {
                    name_row = name_row.push_maybe(self.tuner_button(s, &target));
                }
                let more_key = format!("more:l:{}", l.id);
                let more_open = self.expanded.contains(&more_key);
                name_row = name_row.push(
                    button::icon(icon::from_name("view-more-symbolic"))
                        .tooltip("Light settings")
                        .selected(more_open)
                        .on_press(Message::Expand(more_key)),
                );
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
                if l.on && self.tuner.as_ref() == Some(&target) {
                    lc = lc.push(self.tuner_view(s, &target));
                }
                if more_open {
                    lc = lc.push(self.light_settings(l));
                }
                lights = lights.add(lc);
            }
            col = col.push(lights);
            let key = format!("more:g:{}", g.id);
            let open = self.expanded.contains(&key);
            let label = match (open, g.kind.as_str()) {
                (true, _) => "Hide settings",
                (false, "zone") => "Zone settings",
                (false, _) => "Room settings",
            };
            col = col.push(button::link(label).on_press(Message::Expand(key)));
            if open {
                col = col.push(self.group_settings(s, g));
            }
        }
        col.into()
    }

    fn tuner_info(&self, s: &Snapshot, target: &Target) -> TunerInfo {
        let ids = match target {
            Target::Light(id) => vec![id.clone()],
            Target::Group(gl) => s.lights_of(gl),
        };
        let lights: Vec<_> = ids
            .iter()
            .filter_map(|id| s.lights.get(id))
            .filter(|l| l.on)
            .collect();
        let ct: Vec<_> = lights.iter().filter(|l| l.ct).collect();
        let single = match target {
            Target::Light(id) => s.lights.get(id).filter(|l| l.on),
            Target::Group(_) => None,
        };
        TunerInfo {
            effects: match target {
                Target::Light(_) => single.map(|l| l.effects.clone()).unwrap_or_default(),
                Target::Group(_) => manage::group_effects(lights.iter().copied()),
            },
            effect: single.map(|l| l.effect.clone()).unwrap_or_default(),
            timed: single.map(|l| l.timed_effects.clone()).unwrap_or_default(),
            timed_effect: single.map(|l| l.timed_effect.clone()).unwrap_or_default(),
            color: lights.iter().any(|l| l.color),
            ct: !ct.is_empty(),
            mirek_min: ct
                .iter()
                .map(|l| l.mirek_min)
                .fold(f32::INFINITY, f32::min)
                .min(500.0),
            mirek_max: ct
                .iter()
                .map(|l| l.mirek_max)
                .fold(0.0, f32::max)
                .max(153.0),
            hex: lights
                .iter()
                .find(|l| l.color && l.mirek.is_none())
                .and_then(|l| l.hex.clone()),
            mirek: ct.iter().find_map(|l| l.mirek),
        }
    }

    /// The modes a colour control offers, in tab order.
    fn tuner_modes(info: &TunerInfo) -> Vec<TunerMode> {
        let mut modes = Vec::new();
        if info.color {
            modes.push(TunerMode::Colour);
        }
        if info.ct {
            modes.push(TunerMode::White);
        }
        if !info.effects.is_empty() || !info.timed.is_empty() {
            modes.push(TunerMode::Effects);
        }
        modes
    }

    /// The colour button of a light or room (None when nothing there can change).
    fn tuner_button<'a>(&self, s: &Snapshot, target: &Target) -> Option<Element<'a, Message>> {
        let info = self.tuner_info(s, target);
        if Self::tuner_modes(&info).is_empty() {
            return None;
        }
        let open = self.tuner.as_ref() == Some(target);
        let b = button::icon(icon::from_name("color-select-symbolic"))
            .tooltip(if open {
                "Hide colour"
            } else {
                "Colour, white and effects"
            })
            .selected(open)
            .on_press(Message::ToggleTuner(target.clone()));
        Some(b.into())
    }

    fn tuner_view<'a>(&'a self, s: &'a Snapshot, target: &Target) -> Element<'a, Message> {
        let space = theme::spacing();
        let info = self.tuner_info(s, target);
        let key = target.key();
        let modes = Self::tuner_modes(&info);
        let mode = if modes.contains(&self.tuner_mode) {
            self.tuner_mode
        } else {
            modes.first().copied().unwrap_or_default()
        };
        let white = mode == TunerMode::White;
        let mut col = widget::Column::new().spacing(space.space_xxs).padding([
            space.space_xxs,
            0,
            space.space_xs,
            0,
        ]);

        if modes.len() > 1 {
            let mut tabs = widget::Row::new().spacing(space.space_xxs);
            for m in &modes {
                let label = match m {
                    TunerMode::Colour => "Colour",
                    TunerMode::White => "White",
                    TunerMode::Effects => "Effects",
                };
                tabs = tabs.push(tab_button(label, *m == mode, Message::TunerMode(*m)));
            }
            col = col.push(tabs);
        }

        if mode == TunerMode::Effects {
            if let Target::Group(gl) = target {
                let chips: Vec<Element<'a, Message>> = std::iter::once("no_effect".to_string())
                    .chain(info.effects.iter().cloned())
                    .map(|e| {
                        button::standard(effect_label(&e))
                            .on_press(Message::GroupEffect(gl.clone(), e))
                            .into()
                    })
                    .collect();
                return col
                    .push(text::caption("Effects (on the lights that support them)"))
                    .push(
                        widget::flex_row(chips)
                            .row_spacing(space.space_xxs)
                            .column_spacing(space.space_xxs),
                    )
                    .into();
            }
            let Target::Light(light) = target else {
                return col.into();
            };
            if !info.effects.is_empty() {
                let chips: Vec<Element<'a, Message>> = std::iter::once("no_effect".to_string())
                    .chain(info.effects.iter().cloned())
                    .map(|e| {
                        let active =
                            e == info.effect || (e == "no_effect" && info.effect.is_empty());
                        tab_button(effect_label(&e), active, Message::Effect(light.clone(), e))
                            .into()
                    })
                    .collect();
                col = col.push(
                    widget::flex_row(chips)
                        .row_spacing(space.space_xxs)
                        .column_spacing(space.space_xxs),
                );
            }
            if !info.timed.is_empty() {
                let mut row = widget::Row::new().spacing(space.space_xxs);
                for e in &info.timed {
                    let active = *e == info.timed_effect;
                    let target = if active {
                        "no_effect".to_string()
                    } else {
                        e.clone()
                    };
                    row = row.push(tab_button(
                        effect_label(e),
                        active,
                        Message::TimedEffect(light.clone(), target),
                    ));
                }
                col = col
                    .push(text::caption("Over 30 minutes (click again to stop):"))
                    .push(row);
            }
        } else if white {
            let mut presets = widget::Row::new().spacing(space.space_xxs);
            for (label, m) in WHITE_PRESETS {
                let m = m.clamp(info.mirek_min, info.mirek_max);
                presets = presets.push(
                    button::custom(
                        widget::Row::new()
                            .push(swatch(Some(&colour::mirek_to_hex(m)), 12.0))
                            .push(text::caption(label))
                            .spacing(space.space_xxxs)
                            .align_y(Alignment::Center),
                    )
                    .class(theme::Button::Standard)
                    .on_press(Message::SetMirek(target.clone(), m)),
                );
            }
            let m = self
                .drag
                .get(&format!("m:{key}"))
                .copied()
                .or(info.mirek)
                .unwrap_or((info.mirek_min + info.mirek_max) / 2.0)
                .clamp(info.mirek_min, info.mirek_max);
            let (t1, t2) = (target.clone(), target.clone());
            col = col
                .push(presets)
                .push(
                    widget::Row::new()
                        .push(text::caption("Cool"))
                        .push(
                            widget::slider(info.mirek_min..=info.mirek_max, m, move |v| {
                                Message::Mirek(t1.clone(), v)
                            })
                            .on_release(Message::MirekDone(t2))
                            .width(Length::Fill),
                        )
                        .push(text::caption("Warm"))
                        .spacing(space.space_xs)
                        .align_y(Alignment::Center),
                )
                .push(
                    widget::Row::new()
                        .push(swatch(Some(&colour::mirek_to_hex(m)), 14.0))
                        .push(text::caption(format!("{} K", colour::kelvin(m))))
                        .spacing(space.space_xxs)
                        .align_y(Alignment::Center),
                );
        } else {
            let (h0, s0) = info
                .hex
                .as_deref()
                .and_then(colour::hex_to_hs)
                .unwrap_or((30.0, 80.0));
            let hue = self.drag.get(&format!("h:{key}")).copied().unwrap_or(h0);
            let sat = self.drag.get(&format!("s:{key}")).copied().unwrap_or(s0);
            let mut presets = widget::Row::new().spacing(space.space_xxs);
            for h in PRESET_HUES {
                let hex = colour::hs_to_hex(h, 100.0);
                presets = presets.push(
                    button::custom(swatch(Some(&hex), 22.0))
                        .padding(2)
                        .class(theme::Button::Text)
                        .on_press(Message::SetHex(target.clone(), hex)),
                );
            }
            let (t1, t2, t3, t4) = (
                target.clone(),
                target.clone(),
                target.clone(),
                target.clone(),
            );
            col = col
                .push(presets)
                .push(
                    widget::Row::new()
                        .push(text::caption("Hue").width(Length::Fixed(72.0)))
                        .push(
                            widget::slider(0.0..=359.0, hue, move |v| Message::Hue(t1.clone(), v))
                                .on_release(Message::ColourDone(t2))
                                .width(Length::Fill),
                        )
                        .spacing(space.space_xs)
                        .align_y(Alignment::Center),
                )
                .push(
                    widget::Row::new()
                        .push(text::caption("Saturation").width(Length::Fixed(72.0)))
                        .push(
                            widget::slider(0.0..=100.0, sat, move |v| {
                                Message::Saturation(t3.clone(), v)
                            })
                            .on_release(Message::ColourDone(t4))
                            .width(Length::Fill),
                        )
                        .spacing(space.space_xs)
                        .align_y(Alignment::Center),
                )
                .push(
                    widget::Row::new()
                        .push(swatch(Some(&colour::hs_to_hex(hue, sat)), 14.0))
                        .push(text::caption(colour::hs_to_hex(hue, sat)))
                        .spacing(space.space_xxs)
                        .align_y(Alignment::Center),
                );
        }
        col.into()
    }

    /// Send a colour or a white to a target, and show it at once.
    fn set_colour(&mut self, target: &Target, hex: Option<String>, mirek: Option<f32>) {
        if let Some(s) = &mut self.snapshot {
            let ids = match target {
                Target::Light(id) => vec![id.clone()],
                Target::Group(gl) => s.lights_of(gl),
            };
            s.assume_colour(&ids, hex.as_deref(), mirek);
        }
        let mut cmd = json!({"kind": target.kind(), "ids": [target.id()]});
        if let Some(hex) = hex {
            cmd["hex"] = json!(hex);
        } else if let Some(m) = mirek {
            cmd["mirek"] = json!(m.round() as i64);
        }
        self.send("set", cmd);
    }
}

/// A button that shows as selected (suggested) or not (standard).
fn tab_button<'a>(
    label: impl Into<std::borrow::Cow<'a, str>>,
    active: bool,
    msg: Message,
) -> button::TextButton<'a, Message> {
    if active {
        button::suggested(label)
    } else {
        button::standard(label)
    }
    .on_press(msg)
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
            Message::ToggleTuner(target) => {
                if self.tuner.as_ref() == Some(&target) {
                    self.tuner = None;
                } else {
                    self.tuner_mode =
                        match self.snapshot.as_ref().map(|s| self.tuner_info(s, &target)) {
                            Some(info) if info.ct && (info.mirek.is_some() || !info.color) => {
                                TunerMode::White
                            }
                            Some(info) if !info.color && !info.ct => TunerMode::Effects,
                            _ => TunerMode::Colour,
                        };
                    self.tuner = Some(target);
                }
            }
            Message::TunerMode(mode) => self.tuner_mode = mode,
            Message::Tab(tab) => {
                self.tab = tab;
                self.tuner = None;
            }
            Message::Effect(id, effect) => {
                if let Some(l) = self.snapshot.as_mut().and_then(|s| s.lights.get_mut(&id)) {
                    l.effect = effect.clone();
                }
                self.send(
                    "set",
                    json!({"kind": "light", "ids": [id], "effect": effect}),
                );
            }
            Message::TimedEffect(id, effect) => {
                let light = self.snapshot.as_mut().and_then(|s| s.lights.get_mut(&id));
                if effect == "no_effect" {
                    // The bridge has no stop for a timed effect: any new state ends it, so the
                    // current brightness is sent again (the light keeps its look).
                    let pct = light.map(|l| {
                        l.timed_effect = "no_effect".into();
                        l.brightness
                    });
                    self.send(
                        "set",
                        json!({"kind": "light", "ids": [id], "pct": pct.unwrap_or(100.0).clamp(1.0, 100.0)}),
                    );
                } else {
                    if let Some(l) = light {
                        l.timed_effect = effect.clone();
                    }
                    self.send(
                        "set",
                        json!({"kind": "light", "ids": [id], "timed": {"effect": effect, "duration": TIMED_EFFECT_MS}}),
                    );
                }
            }
            Message::SmartScene(id, activate) => {
                if let Some(r) = self
                    .snapshot
                    .as_mut()
                    .and_then(|s| s.smart_scenes.get_mut(&id))
                {
                    r.active = activate;
                }
                let action = if activate { "activate" } else { "deactivate" };
                self.send("smartScene", json!({"id": id, "action": action}));
            }
            Message::Automation(id, enabled) => {
                if let Some(a) = self
                    .snapshot
                    .as_mut()
                    .and_then(|s| s.automations.iter_mut().find(|a| a.id == id))
                {
                    a.enabled = enabled;
                }
                self.send("automation", json!({"id": id, "enabled": enabled}));
            }
            Message::EntertainmentStop(id) => {
                self.send("entertainment.stop", json!({"id": id}));
            }
            Message::WallpaperTarget(id) => {
                let mut targets = self.wallpaper.config.targets.clone();
                if let Some(i) = targets.iter().position(|t| *t == id) {
                    targets.remove(i);
                } else {
                    targets.push(id);
                }
                self.wallpaper.config.targets = targets.clone();
                self.send("wallpaper.config", json!({"targets": targets}));
            }
            Message::SceneRecall(id, mode) => {
                self.send("scene.recall", json!({"id": id, "mode": mode}));
            }
            Message::SceneSpeed(id, v) => {
                self.drag.insert(format!("speed:{id}"), v);
            }
            Message::SceneSpeedDone(id) => {
                if let Some(v) = self.drag.remove(&format!("speed:{id}")) {
                    if let Some(sc) = self.snapshot.as_mut().and_then(|s| s.scenes.get_mut(&id)) {
                        sc.speed = v;
                    }
                    self.send(
                        "scene.update",
                        json!({"id": id, "speed": (v * 100.0).round() / 100.0}),
                    );
                }
            }
            Message::SceneAutoDynamic(id, on) => {
                if let Some(sc) = self.snapshot.as_mut().and_then(|s| s.scenes.get_mut(&id)) {
                    sc.auto_dynamic = on;
                }
                self.send("scene.update", json!({"id": id, "autoDynamic": on}));
            }
            Message::SceneResave(id) => {
                self.send("scene.update", json!({"id": id, "snapshot": true}));
            }
            Message::SceneDelete(id) => {
                if self.confirmed(&format!("scene.delete:{id}")) {
                    self.send("scene.delete", json!({"id": id}));
                }
            }
            Message::SceneGroup(id) => self.scene_group = Some(id),
            Message::SceneName(name) => self.scene_name = name,
            Message::SceneCreate => {
                let name = self.scene_name.trim().to_string();
                let group = self.scene_group.clone().or_else(|| {
                    self.snapshot
                        .as_ref()
                        .and_then(|s| s.sorted_groups().first().map(|g| g.id.clone()))
                });
                if let (false, Some(group)) = (name.is_empty(), group) {
                    self.send("scene.create", json!({"name": name, "group": group}));
                    self.scene_name.clear();
                }
            }
            Message::RenameInput(key, text) => {
                self.names.insert(key, text);
            }
            Message::RenameSubmit(rtype, id, current) => {
                let key = format!("{rtype}:{id}");
                let name = self
                    .names
                    .get(&key)
                    .map(|n| n.trim().to_string())
                    .unwrap_or_default();
                if !name.is_empty() && name != current {
                    self.send("rename", json!({"rtype": rtype, "id": id, "name": name}));
                    self.names.remove(&key);
                }
            }
            Message::ZoneName(name) => self.zone_name = name,
            Message::ZoneLight(id) => {
                if !self.zone_lights.remove(&id) {
                    self.zone_lights.insert(id);
                }
            }
            Message::ZoneCreate => {
                let name = self.zone_name.trim().to_string();
                if !name.is_empty() && !self.zone_lights.is_empty() {
                    let lights: Vec<String> = self.zone_lights.drain().collect();
                    self.send("zone.create", json!({"name": name, "lights": lights}));
                    self.zone_name.clear();
                    self.expanded.remove("newzone");
                }
            }
            Message::ZoneMember(zone, light) => {
                let members = self
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.groups.iter().find(|g| g.id == zone))
                    .map(|g| {
                        let mut m = g.light_ids.clone();
                        if let Some(i) = m.iter().position(|l| *l == light) {
                            m.remove(i);
                        } else {
                            m.push(light.clone());
                        }
                        m
                    });
                match members {
                    Some(m) if m.is_empty() => {
                        self.notice = Some(
                            "A zone needs at least one light. Delete the zone instead.".into(),
                        );
                    }
                    Some(m) => {
                        if let Some(g) = self
                            .snapshot
                            .as_mut()
                            .and_then(|s| s.groups.iter_mut().find(|g| g.id == zone))
                        {
                            g.light_ids = m.clone();
                        }
                        self.send("zone.members", json!({"id": zone, "lights": m}));
                    }
                    None => {}
                }
            }
            Message::ZoneDelete(id) => {
                if self.confirmed(&format!("zone.delete:{id}")) {
                    self.send("zone.delete", json!({"id": id}));
                }
            }
            Message::GroupEffect(gl, effect) => {
                let targets: Vec<String> = self
                    .snapshot
                    .as_ref()
                    .map(|s| {
                        s.lights_of(&gl)
                            .into_iter()
                            .filter(|id| {
                                s.lights.get(id).is_some_and(|l| {
                                    l.on && !l.effects.is_empty()
                                        && (effect == "no_effect" || l.effects.contains(&effect))
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for id in targets {
                    if let Some(l) = self.snapshot.as_mut().and_then(|s| s.lights.get_mut(&id)) {
                        l.effect = effect.clone();
                    }
                    self.send(
                        "set",
                        json!({"kind": "light", "ids": [id], "effect": effect}),
                    );
                }
            }
            Message::Identify(id) => {
                self.send(
                    "set",
                    json!({"kind": "light", "ids": [id], "identify": true}),
                );
            }
            Message::Powerup(id, preset) => {
                if let Some(l) = self.snapshot.as_mut().and_then(|s| s.lights.get_mut(&id)) {
                    l.powerup = Some(preset.clone());
                }
                self.send("powerup", json!({"id": id, "preset": preset}));
            }
            Message::ForgetBridge => {
                if self.confirmed("forget") {
                    self.snapshot = None;
                    self.tab = Tab::Lights;
                    self.send("pair.forget", json!({}));
                }
            }
            Message::Enable(rtype, id, enabled) => {
                if let Some(c) = self.snapshot.as_mut().and_then(|s| {
                    s.devices
                        .iter_mut()
                        .flat_map(|d| d.controls.iter_mut())
                        .find(|c| c.id == id)
                }) {
                    c.enabled = enabled;
                }
                self.send(
                    "enable",
                    json!({"rtype": rtype, "id": id, "enabled": enabled}),
                );
            }
            Message::Hue(t, v) => {
                self.drag.insert(format!("h:{}", t.key()), v);
            }
            Message::Saturation(t, v) => {
                self.drag.insert(format!("s:{}", t.key()), v);
            }
            Message::ColourDone(t) => {
                let key = t.key();
                let info = self.snapshot.as_ref().map(|s| self.tuner_info(s, &t));
                let (h0, s0) = info
                    .and_then(|i| i.hex)
                    .as_deref()
                    .and_then(colour::hex_to_hs)
                    .unwrap_or((30.0, 80.0));
                let hue = self.drag.remove(&format!("h:{key}")).unwrap_or(h0);
                let sat = self.drag.remove(&format!("s:{key}")).unwrap_or(s0);
                self.set_colour(&t, Some(colour::hs_to_hex(hue, sat)), None);
            }
            Message::Mirek(t, v) => {
                self.drag.insert(format!("m:{}", t.key()), v);
            }
            Message::MirekDone(t) => {
                if let Some(m) = self.drag.remove(&format!("m:{}", t.key())) {
                    self.set_colour(&t, None, Some(m));
                }
            }
            Message::SetHex(t, hex) => {
                let key = t.key();
                self.drag.remove(&format!("h:{key}"));
                self.drag.remove(&format!("s:{key}"));
                self.set_colour(&t, Some(hex), None);
            }
            Message::SetMirek(t, m) => {
                self.drag.remove(&format!("m:{}", t.key()));
                self.set_colour(&t, None, Some(m));
            }
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
