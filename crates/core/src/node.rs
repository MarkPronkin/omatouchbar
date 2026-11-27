//! Declarative description of what the Touch Bar should show.
//!
//! The scene rebuilds a list of [`Node`]s whenever something changes; the
//! animator diffs them by [`Key`] and springs every visual property from
//! where it is to where the new list says it should be.

use crate::{
    canvas::VAlign,
    color::Rgba,
    scene::{Cmd, LevelFeedback, Slider},
    text::Role,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(pub u64);

impl Key {
    pub const fn new(name: &str) -> Key {
        Key(0xcbf2_9ce4_8422_2325).with(name)
    }

    /// Derives a child key (FNV-1a over the name).
    pub const fn with(self, name: &str) -> Key {
        let bytes = name.as_bytes();
        let mut hash = self.0 ^ 0x2f;
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            i += 1;
        }
        Key(hash)
    }

    pub const fn index(self, n: i64) -> Key {
        let mut hash = self.0 ^ 0x51;
        let bytes = n.to_le_bytes();
        let mut i = 0;
        while i < 8 {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            i += 1;
        }
        Key(hash)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }

    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }

    pub fn contains(&self, x: f32, y: f32, pad: f32) -> bool {
        x >= self.x - pad
            && x < self.x + self.w + pad
            && y >= self.y - pad
            && y < self.y + self.h + pad
    }

    pub fn offset(self, dx: f32) -> Rect {
        Rect {
            x: self.x + dx,
            ..self
        }
    }
}

/// How a node arrives or leaves: start/end offset, scale and delay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    /// Seconds to wait before moving (entrances only).
    pub delay: f32,
}

impl Motion {
    pub const FADE: Motion = Motion {
        dx: 0.0,
        dy: 0.0,
        scale: 0.96,
        delay: 0.0,
    };

    pub const fn slide(dx: f32) -> Motion {
        Motion {
            dx,
            dy: 0.0,
            scale: 0.97,
            delay: 0.0,
        }
    }

    pub const fn rise(dy: f32) -> Motion {
        Motion {
            dx: 0.0,
            dy,
            scale: 0.98,
            delay: 0.0,
        }
    }

    pub fn delayed(self, delay: f32) -> Motion {
        Motion { delay, ..self }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub color: Rgba,
    /// Color at full emphasis (selected/active).
    pub emph: Option<Rgba>,
    pub radius: f32,
    pub border: Option<Rgba>,
    /// Subtle top-to-bottom sheen.
    pub sheen: bool,
}

impl Fill {
    pub fn solid(color: Rgba, radius: f32) -> Fill {
        Fill {
            color,
            emph: None,
            radius,
            border: None,
            sheen: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    ClockTrack {
        value: f32,
        daylight: Vec<[f32; 2]>,
        scrub: bool,
        color: Rgba,
        sun: Rgba,
        track: Rgba,
    },
    IconImage {
        icon: crate::state::IconImage,
        size: f32,
        dx: f32,
    },
    Text {
        text: String,
        size: f32,
        role: Role,
        color: Rgba,
        emph: Option<Rgba>,
        anchor: Anchor,
        dx: f32,
        dy: f32,
        valign: VAlign,
    },
    /// Text over the node's `Progress` fill: `on_fill` where the fill
    /// covers it and `color` elsewhere, split at the fill's animated edge,
    /// so a value near that edge never fades into the fill or the track.
    SplitText {
        text: String,
        size: f32,
        role: Role,
        color: Rgba,
        on_fill: Rgba,
        anchor: Anchor,
        dx: f32,
    },
    /// A horizontal fill from the left edge (sliders, meters).
    Progress {
        value: f32,
        color: Rgba,
        track: Option<Rgba>,
        inset: f32,
        radius: f32,
    },
    /// A level read across a row of keys (Keyboard mode's brightness,
    /// keyboard light and volume): this key covers `span` of the row, and
    /// tints from its left up to wherever `value` falls in it, so the row's
    /// keys fill as one meter with a bright edge at the value. Springs like
    /// `Progress`; `shown` fades it in and out.
    Level {
        value: f32,
        span: (f32, f32),
        color: Rgba,
        shown: bool,
    },
    Dot {
        anchor: Anchor,
        dx: f32,
        dy: f32,
        r: f32,
        color: Rgba,
    },
    /// Blinking text caret inside an input field (drawn using live scene time).
    Cursor { dx: f32, height: f32, color: Rgba },
    /// Sweeping progress shimmer inside a busy surface (e.g. connecting to Wi-Fi).
    Shimmer { color: Rgba, radius: f32 },
    /// The Touch ID overlay effect (drawn from the scene's live prompt
    /// timeline; see `touchfx`).
    TouchFx,
    /// Omarchy's screensaver: the effect's latest frame (see `screensaver`).
    Screensaver,
}

impl Part {
    pub fn label(text: impl Into<String>, size: f32, color: Rgba, anchor: Anchor, dx: f32) -> Part {
        Part::Text {
            text: text.into(),
            size,
            role: Role::Text,
            color,
            emph: None,
            anchor,
            dx,
            dy: 0.0,
            valign: VAlign::Cap,
        }
    }

    pub fn icon(text: impl Into<String>, size: f32, color: Rgba, anchor: Anchor, dx: f32) -> Part {
        Part::Text {
            text: text.into(),
            size,
            role: Role::Icon,
            color,
            emph: None,
            anchor,
            dx,
            dy: 0.0,
            valign: VAlign::Ink,
        }
    }

    /// A slider's icon or label, two-toned across its fill (see `SplitText`).
    pub fn split(
        text: impl Into<String>,
        size: f32,
        role: Role,
        (color, on_fill): (Rgba, Rgba),
        anchor: Anchor,
        dx: f32,
    ) -> Part {
        Part::SplitText {
            text: text.into(),
            size,
            role,
            color,
            on_fill,
            anchor,
            dx,
        }
    }

    pub fn with_emph(mut self, color: Rgba) -> Part {
        if let Part::Text { emph, .. } = &mut self {
            *emph = Some(color);
        }
        self
    }

    pub fn with_role(mut self, new_role: Role) -> Part {
        if let Part::Text { role, valign, .. } = &mut self {
            *role = new_role;
            if new_role == Role::Logo {
                *valign = VAlign::Ink;
            }
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    WorldDrag,
    /// Fires on release inside the node.
    Tap,
    /// Tap to run `cmd`, or hold / drag horizontally to morph into an inline slider.
    TapSlide(Slider),
    /// Fires immediately on touch-down (keys).
    Down,
    /// Fires on touch-down, then repeats while held.
    Repeat,
    /// Continuous horizontal value while dragging.
    Slider,
    /// Fires on touch-down. Sliding in from a sibling also plays this cue.
    Scrub(LevelFeedback),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub cmd: Cmd,
    pub kind: HitKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub key: Key,
    pub rect: Rect,
    pub opacity: f32,
    pub emph: f32,
    pub fill: Option<Fill>,
    pub parts: Vec<Part>,
    pub hit: Option<Hit>,
    pub enter: Option<Motion>,
    pub exit: Option<Motion>,
    /// Scroll group (drawn with that group's offset and clip).
    pub group: Option<u8>,
    pub ripple: bool,
    /// Progress values follow immediately instead of springing (dragging).
    pub snap_progress: bool,
    /// Fill color springs toward a highlight while pressed.
    pub press_glow: bool,
}

impl Node {
    pub fn new(key: Key, rect: Rect) -> Node {
        Node {
            key,
            rect,
            opacity: 1.0,
            emph: 0.0,
            fill: None,
            parts: Vec::new(),
            hit: None,
            enter: None,
            exit: None,
            group: None,
            ripple: true,
            snap_progress: false,
            press_glow: true,
        }
    }

    pub fn fill(mut self, fill: Fill) -> Node {
        self.fill = Some(fill);
        self
    }

    pub fn part(mut self, part: Part) -> Node {
        self.parts.push(part);
        self
    }

    pub fn tap(mut self, cmd: Cmd) -> Node {
        self.hit = Some(Hit {
            cmd,
            kind: HitKind::Tap,
        });
        self
    }

    pub fn hit(mut self, cmd: Cmd, kind: HitKind) -> Node {
        self.hit = Some(Hit { cmd, kind });
        self
    }

    pub fn emph(mut self, emph: f32) -> Node {
        self.emph = emph;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Node {
        self.opacity = opacity;
        self
    }

    pub fn enter(mut self, motion: Motion) -> Node {
        self.enter = Some(motion);
        self
    }

    pub fn exit(mut self, motion: Motion) -> Node {
        self.exit = Some(motion);
        self
    }

    pub fn group(mut self, group: u8) -> Node {
        self.group = Some(group);
        self
    }

    pub fn no_ripple(mut self) -> Node {
        self.ripple = false;
        self
    }

    pub fn shift(&mut self, dx: f32) {
        self.rect.x += dx;
    }
}
