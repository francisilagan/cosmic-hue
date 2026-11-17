//! Colour helpers for the controls: hue/saturation to hex and back, and the
//! display colour of a white temperature. The bridge does the real gamut
//! mapping; these only pick what to send and what to show.

/// `#rrggbb` for a hue in degrees (0..360) and a saturation in percent, at full value.
pub fn hs_to_hex(hue: f32, sat: f32) -> String {
    let h = hue.rem_euclid(360.0) / 60.0;
    let s = (sat / 100.0).clamp(0.0, 1.0);
    let c = s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = 1.0 - c;
    to_hex(r + m, g + m, b + m)
}

/// (hue in degrees, saturation in percent) of a `#rrggbb` colour.
pub fn hex_to_hs(hex: &str) -> Option<(f32, f32)> {
    let (r, g, b) = parse(hex)?;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let sat = if max <= 0.0 { 0.0 } else { d / max * 100.0 };
    let hue = if d <= f32::EPSILON {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Some((hue, sat))
}

/// The display colour of a white at `mirek` (1e6 / kelvin), after Tanner Helland's fit.
pub fn mirek_to_hex(mirek: f32) -> String {
    let t = (1.0e6 / mirek.max(1.0) / 100.0).clamp(10.0, 400.0);
    let r = if t <= 66.0 {
        255.0
    } else {
        329.699 * (t - 60.0).powf(-0.133_204_76)
    };
    let g = if t <= 66.0 {
        99.470_8 * t.ln() - 161.119_57
    } else {
        288.122_16 * (t - 60.0).powf(-0.075_514_85)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    to_hex(r / 255.0, g / 255.0, b / 255.0)
}

/// Kelvin, rounded to 100, for a label.
pub fn kelvin(mirek: f32) -> u32 {
    ((1.0e6 / mirek.max(1.0) / 100.0).round() * 100.0) as u32
}

fn parse(hex: &str) -> Option<(f32, f32, f32)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some((
        ((n >> 16) & 0xff) as f32 / 255.0,
        ((n >> 8) & 0xff) as f32 / 255.0,
        (n & 0xff) as f32 / 255.0,
    ))
}

fn to_hex(r: f32, g: f32, b: f32) -> String {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primaries_and_white() {
        assert_eq!(hs_to_hex(0.0, 100.0), "#ff0000");
        assert_eq!(hs_to_hex(120.0, 100.0), "#00ff00");
        assert_eq!(hs_to_hex(240.0, 100.0), "#0000ff");
        assert_eq!(hs_to_hex(360.0, 100.0), "#ff0000");
        assert_eq!(hs_to_hex(200.0, 0.0), "#ffffff");
        assert_eq!(hs_to_hex(30.0, 50.0), "#ffbf80");
    }

    #[test]
    fn hex_round_trips_through_hue_and_saturation() {
        for hex in ["#ff0000", "#ff8000", "#40ff80", "#8040ff", "#ff659e"] {
            let (h, s) = hex_to_hs(hex).unwrap();
            let back = hs_to_hex(h, s);
            let (h2, s2) = hex_to_hs(&back).unwrap();
            assert!(
                (h - h2).abs() < 1.5 && (s - s2).abs() < 1.5,
                "{hex} -> {back}"
            );
        }
        assert_eq!(hex_to_hs("#ffffff"), Some((0.0, 0.0)));
        assert_eq!(hex_to_hs("#000000"), Some((0.0, 0.0)));
        assert_eq!(hex_to_hs("red"), None);
        assert_eq!(hex_to_hs("#12345"), None);
    }

    #[test]
    fn warm_whites_are_redder_than_cool_whites() {
        let warm = parse(&mirek_to_hex(454.0)).unwrap(); // 2200 K
        let cool = parse(&mirek_to_hex(153.0)).unwrap(); // 6500 K
        assert!(warm.2 < cool.2, "warm has less blue");
        assert!(warm.0 >= cool.0, "warm has as much or more red");
        assert_eq!(kelvin(153.0), 6500);
        assert_eq!(kelvin(500.0), 2000);
    }
}
