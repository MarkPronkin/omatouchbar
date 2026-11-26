//! Linear RGBA colors with the small set of operations the renderer needs.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Rgba = Rgba::rgb(1.0, 1.0, 1.0);
    pub const TRANSPARENT: Rgba = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const fn rgb(r: f32, g: f32, b: f32) -> Rgba {
        Rgba { r, g, b, a: 1.0 }
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` is optional).
    pub fn hex(text: &str) -> Option<Rgba> {
        let hex = text.trim().trim_start_matches('#');
        let channel = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| v as f32 / 255.0);
        match hex.len() {
            3 => {
                let mut c = hex
                    .chars()
                    .map(|ch| ch.to_digit(16).map(|v| (v * 17) as f32 / 255.0));
                Some(Rgba::rgb(c.next()??, c.next()??, c.next()??))
            }
            6 => Some(Rgba::rgb(
                channel(&hex[0..2])?,
                channel(&hex[2..4])?,
                channel(&hex[4..6])?,
            )),
            8 => Some(Rgba {
                r: channel(&hex[0..2])?,
                g: channel(&hex[2..4])?,
                b: channel(&hex[4..6])?,
                a: channel(&hex[6..8])?,
            }),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b))
    }

    pub fn alpha(self, a: f32) -> Rgba {
        Rgba { a, ..self }
    }

    pub fn fade(self, factor: f32) -> Rgba {
        Rgba {
            a: self.a * factor,
            ..self
        }
    }

    pub fn mix(self, other: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        Rgba {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    pub fn lighten(self, t: f32) -> Rgba {
        self.mix(Rgba::WHITE.alpha(self.a), t)
    }

    pub fn darken(self, t: f32) -> Rgba {
        self.mix(Rgba::BLACK.alpha(self.a), t)
    }

    /// Relative luminance (sRGB approximation).
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    /// Composites `self` over an opaque `below`.
    pub fn over(self, below: Rgba) -> Rgba {
        Rgba {
            r: self.r * self.a + below.r * (1.0 - self.a),
            g: self.g * self.a + below.g * (1.0 - self.a),
            b: self.b * self.a + below.b * (1.0 - self.a),
            a: 1.0,
        }
    }

    pub fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
            self.a.clamp(0.0, 1.0),
        )
        .unwrap_or(tiny_skia::Color::BLACK)
    }

    /// Premultiplied 8-bit channels.
    pub fn premul8(self) -> [u8; 4] {
        let a = self.a.clamp(0.0, 1.0);
        let c = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
        [c(self.r), c(self.g), c(self.b), (a * 255.0 + 0.5) as u8]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Rgba::hex("#fff"), Some(Rgba::WHITE));
        assert_eq!(Rgba::hex("000000"), Some(Rgba::BLACK));
        assert_eq!(Rgba::hex("#8bcbb1").unwrap().to_hex(), "#8bcbb1");
        assert!(Rgba::hex("#12").is_none());
        assert!((Rgba::hex("#ffffff80").unwrap().a - 0.502).abs() < 0.01);
    }
}
