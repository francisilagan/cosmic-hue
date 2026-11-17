//! The parts of the backend snapshot the applet shows (see docs/PROTOCOL.md).
//! Every field has a default, so a newer backend with more fields still parses.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub bridge: Bridge,
    #[serde(rename = "homeGroup")]
    pub home_group: Option<String>,
    pub groups: Vec<Group>,
    pub lights: HashMap<String, Light>,
    pub scenes: HashMap<String, Scene>,
    #[serde(rename = "smartScenes")]
    pub smart_scenes: HashMap<String, SmartScene>,
    pub automations: Vec<Automation>,
    pub devices: Vec<Device>,
    pub entertainment: Vec<Entertainment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SmartScene {
    pub id: String,
    pub name: String,
    pub group: String,
    pub active: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub status: String,
    pub script: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Entertainment {
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub lights: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Reading {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Control {
    pub rtype: String,
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub product: String,
    pub software: String,
    pub bridge: bool,
    pub reachable: bool,
    pub battery: Option<f32>,
    pub update: Option<String>,
    #[serde(deserialize_with = "readings")]
    pub readings: Vec<Reading>,
    pub controls: Vec<Control>,
}

/// Readings whose value may be a string or a number in the snapshot.
fn readings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Reading>, D::Error> {
    let raw: Vec<serde_json::Value> = Vec::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|v| {
            let label = v.get("label")?.as_str()?.to_string();
            let value = match v.get("value")? {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            Some(Reading { label, value })
        })
        .collect())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Bridge {
    pub name: String,
    pub host: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Group {
    pub id: String,
    pub kind: String,
    pub name: String,
    /// The grouped_light id: the target of group commands.
    pub group: String,
    pub on: bool,
    #[serde(rename = "lightIds")]
    pub light_ids: Vec<String>,
    #[serde(rename = "sceneIds")]
    pub scene_ids: Vec<String>,
    #[serde(rename = "smartSceneIds")]
    pub smart_scene_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Light {
    pub id: String,
    pub name: String,
    pub on: bool,
    pub brightness: f32,
    pub dimmable: bool,
    pub reachable: bool,
    pub color: bool,
    pub hex: Option<String>,
    pub ct: bool,
    pub mirek: Option<f32>,
    #[serde(rename = "mirekMin")]
    pub mirek_min: f32,
    #[serde(rename = "mirekMax")]
    pub mirek_max: f32,
    pub effects: Vec<String>,
    pub effect: String,
    #[serde(rename = "timedEffects")]
    pub timed_effects: Vec<String>,
    #[serde(rename = "timedEffect")]
    pub timed_effect: String,
    pub product: String,
    /// "safety", "powerfail", "last_on_state" or another preset; None when not supported.
    pub powerup: Option<String>,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            on: false,
            brightness: 100.0,
            dimmable: true,
            reachable: true,
            color: false,
            hex: None,
            ct: false,
            mirek: None,
            mirek_min: 153.0,
            mirek_max: 500.0,
            effects: Vec::new(),
            effect: "no_effect".into(),
            timed_effects: Vec::new(),
            timed_effect: "no_effect".into(),
            product: String::new(),
            powerup: None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Scene {
    pub id: String,
    pub name: String,
    pub group: String,
    pub active: bool,
    /// "inactive", "static" or "dynamic_palette".
    pub mode: String,
    pub speed: f32,
    #[serde(rename = "autoDynamic")]
    pub auto_dynamic: bool,
    /// The scene has a palette, so it can play dynamically.
    pub dynamic: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    pub enabled: bool,
    #[serde(rename = "turnOn")]
    pub turn_on: bool,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Wallpaper {
    pub config: WallpaperConfig,
    pub palette: Vec<String>,
    #[serde(rename = "appliedCount")]
    pub applied_count: u32,
    /// Epoch seconds of the last apply.
    #[serde(rename = "appliedAt")]
    pub applied_at: Option<f64>,
    pub eligible: u32,
}

impl Snapshot {
    /// The lights of a group, in the order the bridge lists them, skipping unknown ids.
    pub fn group_lights<'a>(&'a self, group: &'a Group) -> impl Iterator<Item = &'a Light> + 'a {
        group.light_ids.iter().filter_map(|id| self.lights.get(id))
    }

    /// The mean brightness of the lights of a group that are on (0 when none is on).
    pub fn group_brightness(&self, group: &Group) -> f32 {
        let (sum, n) = self
            .group_lights(group)
            .filter(|l| l.on)
            .fold((0.0, 0u32), |(s, n), l| (s + l.brightness, n + 1));
        if n == 0 { 0.0 } else { sum / n as f32 }
    }

    /// True when any light is on.
    pub fn any_on(&self) -> bool {
        self.lights.values().any(|l| l.on)
    }

    /// Rooms first, then zones; each sorted by name.
    pub fn sorted_groups(&self) -> Vec<&Group> {
        let mut groups: Vec<&Group> = self.groups.iter().collect();
        groups.sort_by(|a, b| {
            (a.kind != "room", a.name.to_lowercase())
                .cmp(&(b.kind != "room", b.name.to_lowercase()))
        });
        groups
    }

    /// The ids of the lights a grouped_light controls (every light for the home group).
    pub fn lights_of(&self, grouped_light: &str) -> Vec<String> {
        if self.home_group.as_deref() == Some(grouped_light) {
            self.lights.keys().cloned().collect()
        } else {
            self.groups
                .iter()
                .filter(|g| g.group == grouped_light)
                .flat_map(|g| g.light_ids.clone())
                .collect()
        }
    }

    /// Show a colour (`hex`) or a white (`mirek`) on lights that are on and can show it,
    /// before the bridge echoes the change.
    pub fn assume_colour(&mut self, ids: &[String], hex: Option<&str>, mirek: Option<f32>) {
        for id in ids {
            let Some(l) = self.lights.get_mut(id) else {
                continue;
            };
            if !l.on {
                continue;
            }
            if let (Some(hex), true) = (hex, l.color) {
                l.hex = Some(hex.to_string());
                l.mirek = None;
            } else if let (Some(m), true) = (mirek, l.ct) {
                let m = m.clamp(l.mirek_min, l.mirek_max);
                l.mirek = Some(m);
                l.hex = Some(crate::colour::mirek_to_hex(m));
            }
        }
    }

    /// Set the power of every light of a group (used before the bridge echoes a change).
    pub fn assume_group_on(&mut self, grouped_light: &str, on: bool) {
        let ids = self.lights_of(grouped_light);
        for id in &ids {
            if let Some(l) = self.lights.get_mut(id) {
                l.on = on;
            }
        }
        for g in &mut self.groups {
            if g.group == grouped_light || self.home_group.as_deref() == Some(grouped_light) {
                g.on = on;
            } else {
                g.on = g
                    .light_ids
                    .iter()
                    .any(|id| self.lights.get(id).is_some_and(|l| l.on));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "bridge": {"name": "Hue Bridge", "host": "192.168.1.2", "id": "x"},
            "homeGroup": "home",
            "groups": [
                {"id": "z1", "kind": "zone", "name": "Desk", "group": "gz", "on": false,
                 "lightIds": ["l2"], "sceneIds": [], "archetype": "x"},
                {"id": "r1", "kind": "room", "name": "Living", "group": "gr", "on": true,
                 "lightIds": ["l1", "l2", "missing"], "sceneIds": ["s1"]}
            ],
            "lights": {
                "l1": {"id": "l1", "name": "Lamp", "on": true, "brightness": 40, "extra": [1]},
                "l2": {"id": "l2", "name": "Strip", "on": false, "brightness": 80, "hex": "#ff0000"}
            },
            "scenes": {"s1": {"id": "s1", "name": "Relax", "group": "r1", "active": true, "speed": 0.5}},
            "automations": []
        }))
        .unwrap()
    }

    #[test]
    fn parses_and_ignores_unknown_fields() {
        let s = fixture();
        assert_eq!(s.home_group.as_deref(), Some("home"));
        assert_eq!(s.lights["l2"].hex.as_deref(), Some("#ff0000"));
        assert!(
            s.lights["l1"].reachable,
            "missing fields take their defaults"
        );
        assert!(s.scenes["s1"].active);
    }

    #[test]
    fn rooms_sort_before_zones() {
        let s = fixture();
        let names: Vec<_> = s.sorted_groups().iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Living", "Desk"]);
    }

    #[test]
    fn group_brightness_counts_only_lights_that_are_on() {
        let s = fixture();
        assert_eq!(s.group_brightness(&s.groups[1]), 40.0);
        assert_eq!(s.group_brightness(&s.groups[0]), 0.0);
        assert_eq!(
            s.group_lights(&s.groups[1]).count(),
            2,
            "unknown ids are skipped"
        );
    }

    #[test]
    fn assumed_power_spreads_to_lights_and_overlapping_groups() {
        let mut s = fixture();
        s.assume_group_on("gz", true);
        assert!(s.lights["l2"].on);
        assert!(s.groups[0].on);
        assert!(s.groups[1].on);
        s.assume_group_on("home", false);
        assert!(!s.any_on());
        assert!(s.groups.iter().all(|g| !g.on));
    }

    #[test]
    fn assumed_colour_respects_power_and_capabilities() {
        let mut s = fixture();
        s.lights.get_mut("l2").unwrap().on = true;
        s.lights.get_mut("l2").unwrap().color = true;
        s.lights.get_mut("l1").unwrap().ct = true;
        let ids = s.lights_of("gr");
        s.assume_colour(&ids, Some("#00ff00"), None);
        assert_eq!(s.lights["l2"].hex.as_deref(), Some("#00ff00"));
        assert_eq!(
            s.lights["l1"].hex, None,
            "a white-only light keeps its look"
        );
        s.assume_colour(&ids, None, Some(900.0));
        assert_eq!(
            s.lights["l1"].mirek,
            Some(500.0),
            "clamped to the range of the light"
        );
        assert!(s.lights["l1"].hex.is_some());
        s.lights.get_mut("l1").unwrap().on = false;
        s.assume_colour(&["l1".to_string()], None, Some(200.0));
        assert_eq!(
            s.lights["l1"].mirek,
            Some(500.0),
            "a light that is off is left alone"
        );
        assert_eq!(s.lights_of("home").len(), 2);
    }

    #[test]
    fn the_other_sections_parse() {
        let s: Snapshot = serde_json::from_value(serde_json::json!({
            "lights": {"l": {"id": "l", "effects": ["candle"], "effect": "candle",
                             "timedEffects": ["sunrise"], "timedEffect": "no_effect"}},
            "smartScenes": {"x": {"id": "x", "name": "Natural light", "group": "r", "active": true}},
            "automations": [{"id": "a", "name": "Evening", "enabled": true, "status": "running", "script": "Schedule"}],
            "entertainment": [{"id": "e", "name": "TV area", "status": "active", "type": "screen", "lights": 2}],
            "devices": [{"id": "d", "name": "Motion", "product": "Hue motion sensor", "reachable": true,
                         "battery": 87, "update": "ready",
                         "readings": [{"label": "Temperature", "value": "21.5 °C"}, {"label": "Lux", "value": 120},
                                      {"bad": 1}],
                         "controls": [{"rtype": "motion", "id": "m", "label": "Motion", "enabled": false}]}]
        }))
        .unwrap();
        assert_eq!(s.lights["l"].effect, "candle");
        assert_eq!(s.lights["l"].timed_effects, ["sunrise"]);
        assert!(s.smart_scenes["x"].active);
        assert_eq!(s.automations[0].script, "Schedule");
        assert_eq!(s.entertainment[0].kind, "screen");
        let d = &s.devices[0];
        assert_eq!(d.battery, Some(87.0));
        assert_eq!(d.readings.len(), 2, "a reading without a label is dropped");
        assert_eq!(d.readings[1].value, "120", "a number value becomes text");
        assert_eq!(d.controls[0].rtype, "motion");
    }

    #[test]
    fn scene_light_and_wallpaper_details_parse() {
        let s: Snapshot = serde_json::from_value(serde_json::json!({
            "lights": {"l": {"id": "l", "product": "Hue Play", "powerup": "safety"}},
            "scenes": {"s": {"id": "s", "name": "Relax", "group": "r", "active": true,
                             "mode": "dynamic_palette", "speed": 0.4, "autoDynamic": true, "dynamic": true}}
        }))
        .unwrap();
        assert_eq!(s.lights["l"].powerup.as_deref(), Some("safety"));
        let sc = &s.scenes["s"];
        assert_eq!(
            (sc.mode.as_str(), sc.auto_dynamic, sc.dynamic),
            ("dynamic_palette", true, true)
        );
        assert!((sc.speed - 0.4).abs() < 1e-6);
        let w: Wallpaper = serde_json::from_value(serde_json::json!({
            "config": {"enabled": true, "targets": ["r"]}, "appliedAt": 1700000000.5, "appliedCount": 3
        }))
        .unwrap();
        assert_eq!(w.applied_at, Some(1700000000.5));
        assert_eq!(w.config.targets, ["r"]);
        let none: Wallpaper =
            serde_json::from_value(serde_json::json!({"appliedAt": null})).unwrap();
        assert_eq!(none.applied_at, None);
    }

    #[test]
    fn an_empty_object_is_an_empty_snapshot() {
        let s: Snapshot = serde_json::from_str("{}").unwrap();
        assert!(s.groups.is_empty() && s.home_group.is_none());
    }
}
