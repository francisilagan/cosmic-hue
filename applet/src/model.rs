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

    /// Set the power of every light of a group (used before the bridge echoes a change).
    pub fn assume_group_on(&mut self, grouped_light: &str, on: bool) {
        let ids: Vec<String> = if self.home_group.as_deref() == Some(grouped_light) {
            self.lights.keys().cloned().collect()
        } else {
            self.groups
                .iter()
                .filter(|g| g.group == grouped_light)
                .flat_map(|g| g.light_ids.clone())
                .collect()
        };
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
    fn an_empty_object_is_an_empty_snapshot() {
        let s: Snapshot = serde_json::from_str("{}").unwrap();
        assert!(s.groups.is_empty() && s.home_group.is_none());
    }
}
