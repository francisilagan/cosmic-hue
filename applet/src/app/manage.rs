//! The views that manage the bridge's setup: wallpaper sync, the Scenes tab,
//! room and light settings (rename, zones, power-up, identify) and the bridge
//! card. They are methods of the applet, split out of `app.rs` for size.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::theme;
use cosmic::widget::{self, button, container, settings, text, toggler};

use super::{HueApplet, Message, effect_label, swatch, tab_button};
use crate::model::{Group, Light, Snapshot};

/// How long a "click again" confirmation stays armed.
pub(super) const CONFIRM_FOR: Duration = Duration::from_secs(3);

/// Power-up presets: (bridge id, label).
const POWERUP: [(&str, &str); 3] = [
    ("safety", "Warm white"),
    ("last_on_state", "Last state"),
    ("powerfail", "Power fail"),
];

/// "just now", "5 min ago", "2 h ago", "3 days ago".
pub(super) fn ago(epoch: f64, now: f64) -> String {
    let secs = (now - epoch).max(0.0);
    match secs {
        s if s < 60.0 => "just now".into(),
        s if s < 3600.0 => format!("{} min ago", (s / 60.0) as u64),
        s if s < 86400.0 => format!("{} h ago", (s / 3600.0) as u64),
        s => {
            let d = (s / 86400.0) as u64;
            format!("{d} {} ago", if d == 1 { "day" } else { "days" })
        }
    }
}

fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Chips in a wrapping row.
fn chips<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let space = theme::spacing();
    widget::flex_row(items)
        .row_spacing(space.space_xxs)
        .column_spacing(space.space_xxs)
        .into()
}

impl HueApplet {
    /// True while `key` was clicked once and a second click would confirm.
    pub(super) fn armed(&self, key: &str) -> bool {
        self.confirm
            .as_ref()
            .is_some_and(|(k, at)| k == key && at.elapsed() < CONFIRM_FOR)
    }

    /// The first click arms `key` and returns false; a second click in time returns true.
    pub(super) fn confirmed(&mut self, key: &str) -> bool {
        if self.armed(key) {
            self.confirm = None;
            true
        } else {
            self.confirm = Some((key.to_string(), Instant::now()));
            false
        }
    }

    /// A name field with a Rename button. The draft lives in `names` until it is sent.
    pub(super) fn rename_row<'a>(
        &'a self,
        rtype: &str,
        id: &str,
        current: &str,
    ) -> Element<'a, Message> {
        let space = theme::spacing();
        let key = format!("{rtype}:{id}");
        let value = self
            .names
            .get(&key)
            .cloned()
            .unwrap_or_else(|| current.to_string());
        let changed = {
            let t = value.trim();
            !t.is_empty() && t != current
        };
        let submit = Message::RenameSubmit(rtype.to_string(), id.to_string(), current.to_string());
        let submit2 = submit.clone();
        widget::Row::new()
            .push(
                widget::text_input("Name", value)
                    .on_input(move |t| Message::RenameInput(key.clone(), t))
                    .on_submit(move |_| submit.clone())
                    .width(Length::Fill),
            )
            .push(button::standard("Rename").on_press_maybe(changed.then_some(submit2)))
            .spacing(space.space_xxs)
            .align_y(Alignment::Center)
            .into()
    }

    pub(super) fn wallpaper_view<'a>(&'a self, s: &'a Snapshot) -> Element<'a, Message> {
        let space = theme::spacing();
        let w = &self.wallpaper;
        let connected = self.conn == "connected";

        let mut status = Vec::new();
        match w.applied_at {
            Some(at) if w.applied_count > 0 => status.push(format!(
                "Applied to {}, {}",
                plural(w.applied_count, "light", "lights"),
                ago(at, now_epoch())
            )),
            Some(at) => status.push(format!("Last applied {}", ago(at, now_epoch()))),
            None if w.config.enabled => status.push("Not applied yet".into()),
            None => {}
        }
        if connected {
            status.push(format!(
                "{} eligible",
                plural(w.eligible, "light", "lights")
            ));
        }

        let mut col = widget::Column::new()
            .spacing(space.space_xxs)
            .padding([space.space_xs, space.space_s])
            .push(
                widget::Row::new()
                    .push(text::body("Wallpaper sync").width(Length::Fill))
                    .push(toggler(w.config.enabled).on_toggle(Message::WallpaperEnabled))
                    .align_y(Alignment::Center),
            )
            .push(text::caption(if w.config.enabled {
                "Colour lights follow your wallpaper, and update when it changes. Brightness is left alone."
            } else {
                "Paint your colour lights with the colours of your COSMIC wallpaper."
            }));
        if !status.is_empty() {
            col = col.push(text::caption(status.join("  ·  ")));
        }
        if connected && w.config.enabled && w.eligible == 0 {
            col = col.push(text::caption(if w.config.turn_on {
                "There are no colour lights in the selected rooms. Pick other rooms."
            } else {
                "No colour lights are on in the selected rooms. Turn on \"Also switch lights on\" or pick other rooms."
            }));
        }
        if w.palette.is_empty() {
            if w.config.enabled {
                col = col.push(text::caption(
                    "No palette could be read from the wallpaper. Is ImageMagick (magick) installed?",
                ));
            }
        } else {
            let mut swatches = widget::Row::new().spacing(space.space_xxs);
            for hex in &w.palette {
                swatches = swatches.push(swatch(Some(hex.as_str()), 18.0));
            }
            col = col.push(swatches);
        }

        let rooms: Vec<Element<'a, Message>> = s
            .sorted_groups()
            .into_iter()
            .map(|g| {
                let on = w.config.targets.contains(&g.id);
                tab_button(g.name.clone(), on, Message::WallpaperTarget(g.id.clone())).into()
            })
            .collect();
        if !rooms.is_empty() {
            col = col
                .push(text::caption("Rooms to colour (none selected = all rooms)"))
                .push(chips(rooms));
        }
        col.push(
            widget::Row::new()
                .push(text::caption("Also switch lights on").width(Length::Fill))
                .push(toggler(w.config.turn_on).on_toggle(Message::WallpaperTurnOn))
                .align_y(Alignment::Center),
        )
        .push(button::standard("Sync now").on_press(Message::WallpaperSync))
        .into()
    }

    pub(super) fn scenes_view<'a>(&'a self, s: &'a Snapshot) -> Element<'a, Message> {
        let space = theme::spacing();
        let mut col = widget::Column::new()
            .spacing(space.space_xs)
            .padding([space.space_xs, space.space_s]);
        for g in s.sorted_groups() {
            let mut scenes: Vec<_> = g
                .scene_ids
                .iter()
                .filter_map(|id| s.scenes.get(id))
                .collect();
            let mut routines: Vec<_> = g
                .smart_scene_ids
                .iter()
                .filter_map(|id| s.smart_scenes.get(id))
                .collect();
            if scenes.is_empty() && routines.is_empty() {
                continue;
            }
            scenes.sort_by_key(|sc| sc.name.to_lowercase());
            routines.sort_by_key(|r| r.name.to_lowercase());
            let title = if g.kind == "zone" {
                format!("{}  ·  zone", g.name)
            } else {
                g.name.clone()
            };
            col = col.push(text::heading(title));
            let mut section = settings::section();
            for r in routines {
                let id = r.id.clone();
                section = section.add(
                    widget::Row::new()
                        .push(
                            widget::Column::new()
                                .push(text::body(format!("◷ {}", r.name)))
                                .push(text::caption(if r.active { "Running" } else { "Off" }))
                                .width(Length::Fill),
                        )
                        .push(
                            toggler(r.active)
                                .on_toggle(move |on| Message::SmartScene(id.clone(), on)),
                        )
                        .align_y(Alignment::Center),
                );
            }
            for sc in scenes {
                section = section.add(self.scene_row(sc));
            }
            col = col.push(section);
        }

        // Save the current look of a room as a new scene.
        let groups = s.sorted_groups();
        let chosen = self
            .scene_group
            .clone()
            .filter(|id| groups.iter().any(|g| &g.id == id))
            .or_else(|| groups.first().map(|g| g.id.clone()));
        let picks: Vec<Element<'a, Message>> = groups
            .iter()
            .map(|g| {
                tab_button(
                    g.name.clone(),
                    chosen.as_ref() == Some(&g.id),
                    Message::SceneGroup(g.id.clone()),
                )
                .into()
            })
            .collect();
        let can_save = chosen.is_some() && !self.scene_name.trim().is_empty();
        col.push(text::heading("Save the current look as a scene"))
            .push(chips(picks))
            .push(
                widget::Row::new()
                    .push(
                        widget::text_input("Scene name", self.scene_name.as_str())
                            .on_input(Message::SceneName)
                            .on_submit(|_| Message::SceneCreate)
                            .width(Length::Fill),
                    )
                    .push(
                        button::suggested("Save scene")
                            .on_press_maybe(can_save.then_some(Message::SceneCreate)),
                    )
                    .spacing(space.space_xxs)
                    .align_y(Alignment::Center),
            )
            .into()
    }

    fn scene_row<'a>(&'a self, sc: &'a crate::model::Scene) -> Element<'a, Message> {
        let space = theme::spacing();
        let key = format!("scene:{}", sc.id);
        let open = self.expanded.contains(&key);
        let tag = match (sc.active, sc.mode.as_str()) {
            (true, "dynamic_palette") => " · dynamic",
            (true, _) => " · active",
            _ => "",
        };
        let mut head = widget::Row::new()
            .push(
                button::custom(text::body(format!("{}{}", sc.name, tag)))
                    .class(theme::Button::Text)
                    .on_press(Message::Expand(key))
                    .width(Length::Fill),
            )
            .push(tab_button(
                "Static",
                sc.mode == "static",
                Message::SceneRecall(sc.id.clone(), "active".into()),
            ))
            .spacing(space.space_xxs)
            .align_y(Alignment::Center);
        if sc.dynamic {
            head = head.push(tab_button(
                "Dynamic",
                sc.mode == "dynamic_palette",
                Message::SceneRecall(sc.id.clone(), "dynamic_palette".into()),
            ));
        }
        let mut col = widget::Column::new().push(head).spacing(space.space_xxs);
        if open {
            if sc.dynamic {
                let speed = self
                    .drag
                    .get(&format!("speed:{}", sc.id))
                    .copied()
                    .unwrap_or(sc.speed);
                let (id1, id2, id3) = (sc.id.clone(), sc.id.clone(), sc.id.clone());
                col = col
                    .push(
                        widget::Row::new()
                            .push(text::caption("Speed").width(Length::Fixed(72.0)))
                            .push(
                                widget::slider(0.0..=1.0, speed, move |v| {
                                    Message::SceneSpeed(id1.clone(), v)
                                })
                                .step(0.05_f32)
                                .on_release(Message::SceneSpeedDone(id2))
                                .width(Length::Fill),
                            )
                            .spacing(space.space_xs)
                            .align_y(Alignment::Center),
                    )
                    .push(
                        widget::Row::new()
                            .push(text::caption("Start dynamic automatically").width(Length::Fill))
                            .push(
                                toggler(sc.auto_dynamic).on_toggle(move |on| {
                                    Message::SceneAutoDynamic(id3.clone(), on)
                                }),
                            )
                            .align_y(Alignment::Center),
                    );
            }
            let del_key = format!("scene.delete:{}", sc.id);
            col = col
                .push(
                    widget::Row::new()
                        .push(
                            button::standard("Re-save current look")
                                .on_press(Message::SceneResave(sc.id.clone())),
                        )
                        .push(
                            button::destructive(if self.armed(&del_key) {
                                "Click again to delete"
                            } else {
                                "Delete"
                            })
                            .on_press(Message::SceneDelete(sc.id.clone())),
                        )
                        .spacing(space.space_xxs),
                )
                .push(self.rename_row("scene", &sc.id, &sc.name));
        }
        col.into()
    }

    /// The settings of a room or zone: rename, and for a zone its lights and delete.
    pub(super) fn group_settings<'a>(
        &'a self,
        s: &'a Snapshot,
        g: &'a Group,
    ) -> Element<'a, Message> {
        let space = theme::spacing();
        let mut col = widget::Column::new()
            .spacing(space.space_xxs)
            .padding([space.space_xxs, 0])
            .push(self.rename_row(&g.kind, &g.id, &g.name));
        if g.kind == "zone" {
            let mut lights: Vec<&Light> = s.lights.values().collect();
            lights.sort_by_key(|l| l.name.to_lowercase());
            let members: Vec<Element<'a, Message>> = lights
                .into_iter()
                .map(|l| {
                    tab_button(
                        l.name.clone(),
                        g.light_ids.contains(&l.id),
                        Message::ZoneMember(g.id.clone(), l.id.clone()),
                    )
                    .into()
                })
                .collect();
            let del_key = format!("zone.delete:{}", g.id);
            col = col
                .push(text::caption("Zone members"))
                .push(chips(members))
                .push(
                    button::destructive(if self.armed(&del_key) {
                        "Click again to delete this zone"
                    } else {
                        "Delete zone"
                    })
                    .on_press(Message::ZoneDelete(g.id.clone())),
                );
        }
        col.into()
    }

    /// The settings of one light: product, flash, power-up and rename.
    pub(super) fn light_settings<'a>(&'a self, l: &'a Light) -> Element<'a, Message> {
        let space = theme::spacing();
        let mut col = widget::Column::new()
            .spacing(space.space_xxs)
            .padding([space.space_xxs, 0]);
        if !l.product.is_empty() {
            col = col.push(text::caption(l.product.clone()));
        }
        col = col.push(button::standard("Flash").on_press(Message::Identify(l.id.clone())));
        if let Some(current) = &l.powerup {
            let presets: Vec<Element<'a, Message>> = POWERUP
                .iter()
                .map(|(id, label)| {
                    tab_button(
                        *label,
                        current == id,
                        Message::Powerup(l.id.clone(), id.to_string()),
                    )
                    .into()
                })
                .collect();
            col = col
                .push(text::caption("When power returns"))
                .push(chips(presets));
        }
        col.push(self.rename_row("light", &l.id, &l.name)).into()
    }

    /// The "New zone" form at the end of the Lights tab.
    pub(super) fn new_zone_view<'a>(&'a self, s: &'a Snapshot) -> Element<'a, Message> {
        let space = theme::spacing();
        let open = self.expanded.contains("newzone");
        let mut col = widget::Column::new()
            .spacing(space.space_xxs)
            .padding([0, space.space_s])
            .push(
                button::link(if open { "Hide new zone" } else { "New zone" })
                    .on_press(Message::Expand("newzone".into())),
            );
        if !open {
            return col.into();
        }
        let mut lights: Vec<&Light> = s.lights.values().collect();
        lights.sort_by_key(|l| l.name.to_lowercase());
        let picks: Vec<Element<'a, Message>> = lights
            .into_iter()
            .map(|l| {
                tab_button(
                    l.name.clone(),
                    self.zone_lights.contains(&l.id),
                    Message::ZoneLight(l.id.clone()),
                )
                .into()
            })
            .collect();
        let can = !self.zone_name.trim().is_empty() && !self.zone_lights.is_empty();
        col = col
            .push(text::caption(
                "A zone groups lights from any rooms, so one switch or scene can control them together.",
            ))
            .push(
                widget::text_input("Zone name", self.zone_name.as_str())
                    .on_input(Message::ZoneName)
                    .on_submit(|_| Message::ZoneCreate),
            )
            .push(chips(picks))
            .push(button::suggested("Create zone").on_press_maybe(can.then_some(Message::ZoneCreate)));
        container(col).padding([0, 0, space.space_xs, 0]).into()
    }

    /// The bridge card at the top of the Devices tab.
    pub(super) fn bridge_card<'a>(&'a self, s: &'a Snapshot) -> Element<'a, Message> {
        let space = theme::spacing();
        let name = if s.bridge.name.is_empty() {
            "Hue Bridge"
        } else {
            &s.bridge.name
        };
        let line = if s.bridge.host.is_empty() {
            format!("Paired with {name}")
        } else {
            format!("Paired with {name} at {}", s.bridge.host)
        };
        widget::Column::new()
            .spacing(space.space_xxs)
            .push(text::body(line))
            .push(
                button::destructive(if self.armed("forget") {
                    "Click again to forget this bridge"
                } else {
                    "Forget this bridge"
                })
                .on_press(Message::ForgetBridge),
            )
            .into()
    }

    /// The view when the paired bridge does not answer and nothing has loaded.
    pub(super) fn recovery_view(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        widget::Column::new()
            .spacing(space.space_xs)
            .padding([space.space_xs, space.space_s])
            .push(text::body("Can't reach your bridge."))
            .push(text::caption(
                "The bridge may be off or disconnected, or its IP address may have changed. Retry, or forget it and pair again at its new address.",
            ))
            .push(
                widget::Row::new()
                    .spacing(space.space_xxs)
                    .push(button::suggested("Retry").on_press(Message::Refresh))
                    .push(
                        button::standard(if self.armed("forget") {
                            "Click again to forget this bridge"
                        } else {
                            "Pair with a different bridge"
                        })
                        .on_press(Message::ForgetBridge),
                    ),
            )
            .into()
    }
}

/// The effects that at least one light that is on supports, in first-seen order.
pub(super) fn group_effects<'a>(lights: impl Iterator<Item = &'a Light>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in lights.filter(|l| l.on) {
        for e in &l.effects {
            if !out.contains(e) {
                out.push(e.clone());
            }
        }
    }
    out
}

/// A label for the effect a light shows now, or None.
pub(super) fn running_effect(l: &Light) -> Option<String> {
    if !l.on {
        return None;
    }
    [&l.effect, &l.timed_effect]
        .into_iter()
        .find(|e| !e.is_empty() && e.as_str() != "no_effect")
        .map(|e| effect_label(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_reads_like_a_person() {
        assert_eq!(ago(1000.0, 1030.0), "just now");
        assert_eq!(ago(1000.0, 1000.0 + 5.0 * 60.0), "5 min ago");
        assert_eq!(ago(0.0, 2.5 * 3600.0), "2 h ago");
        assert_eq!(ago(0.0, 86400.0 * 1.2), "1 day ago");
        assert_eq!(ago(0.0, 86400.0 * 3.0), "3 days ago");
        assert_eq!(
            ago(2000.0, 1000.0),
            "just now",
            "a clock in the future is not negative"
        );
    }

    #[test]
    fn group_effects_merge_lights_that_are_on() {
        let mk = |on, effects: &[&str]| Light {
            on,
            effects: effects.iter().map(|e| e.to_string()).collect(),
            ..Default::default()
        };
        let lights = [
            mk(true, &["candle"]),
            mk(true, &["candle", "fire"]),
            mk(false, &["prism"]),
        ];
        assert_eq!(group_effects(lights.iter()), ["candle", "fire"]);
    }

    #[test]
    fn the_running_effect_is_named() {
        let mut l = Light {
            on: true,
            ..Default::default()
        };
        assert_eq!(running_effect(&l), None);
        l.effect = "fire".into();
        assert_eq!(running_effect(&l).as_deref(), Some("Fire"));
        l.effect = "no_effect".into();
        l.timed_effect = "sunrise".into();
        assert_eq!(running_effect(&l).as_deref(), Some("Sunrise"));
        l.on = false;
        assert_eq!(running_effect(&l), None);
    }
}
