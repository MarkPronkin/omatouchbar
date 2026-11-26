//! Springs and the keyed node animator.

use std::collections::HashMap;

use crate::node::{Key, Motion, Node, Part, Rect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringParams {
    pub stiffness: f32,
    pub damping: f32,
}

impl SpringParams {
    /// `response` is the period in seconds; `damping_ratio` 1.0 is
    /// critically damped, lower values overshoot.
    pub fn new(response: f32, damping_ratio: f32) -> SpringParams {
        let response = response.max(0.02);
        let omega = std::f32::consts::TAU / response;
        SpringParams {
            stiffness: omega * omega,
            damping: 2.0 * damping_ratio * omega,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
}

impl Spring {
    pub fn new(value: f32) -> Spring {
        Spring {
            value,
            velocity: 0.0,
            target: value,
        }
    }

    pub fn snap(&mut self, value: f32) {
        *self = Spring::new(value);
    }

    pub fn step(&mut self, dt: f32, p: SpringParams, epsilon: f32) -> bool {
        if self.settled(epsilon) {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        let mut remaining = dt;
        while remaining > 0.0 {
            let h = remaining.min(1.0 / 240.0);
            let force = -p.stiffness * (self.value - self.target) - p.damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            remaining -= h;
        }
        if self.settled(epsilon) {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        true
    }

    pub fn settled(&self, epsilon: f32) -> bool {
        (self.value - self.target).abs() < epsilon && self.velocity.abs() < epsilon * 12.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feel {
    pub enabled: bool,
    pub speed: f32,
    pub bounce: f32,
}

impl Default for Feel {
    fn default() -> Self {
        Feel {
            enabled: true,
            speed: 1.0,
            bounce: 0.3,
        }
    }
}

struct Params {
    position: SpringParams,
    fade: SpringParams,
    press: SpringParams,
    emph: SpringParams,
    progress: SpringParams,
}

impl Params {
    fn from(feel: Feel) -> Params {
        let zeta = 1.0 - feel.bounce.clamp(0.0, 1.0) * 0.55;
        let s = feel.speed.clamp(0.25, 3.0);
        Params {
            position: SpringParams::new(0.42 / s, zeta),
            fade: SpringParams::new(0.26 / s, 1.0),
            press: SpringParams::new(0.11, 1.0),
            emph: SpringParams::new(0.28 / s, 1.0),
            progress: SpringParams::new(0.22 / s, (zeta + 0.25).min(1.0)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ripple {
    pub x: f32,
    pub y: f32,
    pub age: f32,
    pub released: bool,
    pub fade: f32,
}

impl Ripple {
    pub const GROW: f32 = 0.42;
}

pub struct Animated {
    pub node: Node,
    pub x: Spring,
    pub y: Spring,
    pub w: Spring,
    pub h: Spring,
    pub opacity: Spring,
    pub scale: Spring,
    pub emph: Spring,
    pub press: Spring,
    pub progress: Spring,
    pub ripples: Vec<Ripple>,
    pub exiting: bool,
    delay: f32,
}

impl Animated {
    pub fn rect(&self) -> Rect {
        Rect::new(
            self.x.value,
            self.y.value,
            self.w.value.max(0.0),
            self.h.value.max(0.0),
        )
    }

    fn progress_target(node: &Node) -> Option<f32> {
        node.parts.iter().find_map(|p| match p {
            Part::Progress { value, .. } => Some(*value),
            _ => None,
        })
    }
}

pub struct Animator {
    pub items: Vec<Animated>,
    index: HashMap<Key, usize>,
    feel: Feel,
    params: Params,
    /// Default entrance/exit for nodes that don't specify their own.
    pub enter: Motion,
    pub exit: Motion,
}

impl Default for Animator {
    fn default() -> Self {
        Animator::new(Feel::default())
    }
}

impl Animator {
    pub fn new(feel: Feel) -> Animator {
        Animator {
            items: Vec::new(),
            index: HashMap::new(),
            feel,
            params: Params::from(feel),
            enter: Motion::FADE,
            exit: Motion::FADE,
        }
    }

    pub fn set_feel(&mut self, feel: Feel) {
        self.feel = feel;
        self.params = Params::from(feel);
    }

    pub fn enabled(&self) -> bool {
        self.feel.enabled
    }

    pub fn get(&self, key: Key) -> Option<&Animated> {
        self.index
            .get(&key)
            .map(|&i| &self.items[i])
            .filter(|a| !a.exiting)
    }

    pub fn get_mut(&mut self, key: Key) -> Option<&mut Animated> {
        match self.index.get(&key) {
            Some(&i) if !self.items[i].exiting => Some(&mut self.items[i]),
            _ => None,
        }
    }

    /// Replaces the target node list.
    pub fn sync(&mut self, nodes: Vec<Node>) {
        let mut old: HashMap<Key, Animated> =
            self.items.drain(..).map(|a| (a.node.key, a)).collect();
        let mut live = Vec::with_capacity(nodes.len());
        let animate = self.feel.enabled;
        for node in nodes {
            let r = node.rect;
            let animated = match old.remove(&node.key) {
                Some(mut a) => {
                    if a.exiting {
                        a.exiting = false;
                        a.delay = if a.opacity.value < 0.25 {
                            node.enter.unwrap_or(self.enter).delay
                        } else {
                            0.0
                        };
                    }
                    a.x.target = r.x;
                    a.y.target = r.y;
                    a.w.target = r.w;
                    a.h.target = r.h;
                    a.opacity.target = node.opacity;
                    a.scale.target = 1.0;
                    a.emph.target = node.emph;
                    if let Some(p) = Animated::progress_target(&node) {
                        if node.snap_progress || !animate {
                            a.progress.snap(p);
                        } else {
                            a.progress.target = p;
                        }
                    }
                    a.node = node;
                    a
                }
                None => {
                    let m = node.enter.unwrap_or(self.enter);
                    let progress = Animated::progress_target(&node).unwrap_or(0.0);
                    let mut a = Animated {
                        x: Spring::new(r.x + m.dx),
                        y: Spring::new(r.y + m.dy),
                        w: Spring::new(r.w),
                        h: Spring::new(r.h),
                        opacity: Spring::new(0.0),
                        scale: Spring::new(m.scale),
                        emph: Spring::new(node.emph),
                        press: Spring::new(0.0),
                        progress: Spring::new(progress),
                        ripples: Vec::new(),
                        exiting: false,
                        delay: m.delay,
                        node,
                    };
                    a.x.target = r.x;
                    a.y.target = r.y;
                    a.opacity.target = a.node.opacity;
                    a.scale.target = 1.0;
                    if !animate {
                        a.x.snap(r.x);
                        a.y.snap(r.y);
                        a.opacity.snap(a.node.opacity);
                        a.scale.snap(1.0);
                        a.delay = 0.0;
                    }
                    a
                }
            };
            live.push(animated);
        }
        // Leftovers animate out underneath the live nodes.
        let mut exiting: Vec<Animated> = old
            .into_values()
            .filter(|_| animate)
            .map(|mut a| {
                if !a.exiting {
                    let m = a.node.exit.unwrap_or(self.exit);
                    a.exiting = true;
                    a.delay = 0.0;
                    a.opacity.target = 0.0;
                    a.x.target = a.x.value + m.dx;
                    a.y.target = a.y.value + m.dy;
                    a.scale.target = m.scale;
                    a.press.target = 0.0;
                }
                a
            })
            .collect();
        exiting.sort_by_key(|a| a.node.key);
        self.items = exiting;
        self.items.extend(live);
        self.index = self
            .items
            .iter()
            .enumerate()
            .map(|(i, a)| (a.node.key, i))
            .collect();
    }

    /// Advances every spring. Returns whether anything is still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        let p = &self.params;
        let mut moving = false;
        let enabled = self.feel.enabled;
        for a in &mut self.items {
            for ripple in &mut a.ripples {
                ripple.age += dt;
                if ripple.released {
                    ripple.fade += dt;
                }
            }
            a.ripples.retain(|r| r.fade < 0.32 && r.age < 3.0);
            if !a.ripples.is_empty() {
                moving = true;
            }
            moving |= a.press.step(dt, p.press, 0.002);
            if !enabled {
                a.x.snap(a.x.target);
                a.y.snap(a.y.target);
                a.w.snap(a.w.target);
                a.h.snap(a.h.target);
                a.opacity.snap(a.opacity.target);
                a.scale.snap(a.scale.target);
                a.emph.snap(a.emph.target);
                a.progress.snap(a.progress.target);
                continue;
            }
            if a.delay > 0.0 {
                a.delay -= dt;
                moving = true;
                continue;
            }
            moving |= a.x.step(dt, p.position, 0.05);
            moving |= a.y.step(dt, p.position, 0.05);
            moving |= a.w.step(dt, p.position, 0.05);
            moving |= a.h.step(dt, p.position, 0.05);
            moving |= a.scale.step(dt, p.position, 0.0005);
            moving |= a.opacity.step(dt, p.fade, 0.002);
            moving |= a.emph.step(dt, p.emph, 0.002);
            moving |= a.progress.step(dt, p.progress, 0.0005);
        }
        let before = self.items.len();
        self.items
            .retain(|a| !(a.exiting && (a.opacity.value < 0.01 || !enabled)));
        if self.items.len() != before {
            self.index = self
                .items
                .iter()
                .enumerate()
                .map(|(i, a)| (a.node.key, i))
                .collect();
        }
        moving
    }

    pub fn press(&mut self, key: Key, down: bool, at: Option<(f32, f32)>, ripple: bool) {
        // Releases must reach nodes that are already animating out.
        let index = self.index.get(&key).copied();
        let node = index
            .map(|i| &mut self.items[i])
            .filter(|a| !down || !a.exiting);
        if let Some(a) = node {
            a.press.target = if down { 1.0 } else { 0.0 };
            if down && ripple {
                if let Some((x, y)) = at {
                    a.ripples.push(Ripple {
                        x: x - a.x.value,
                        y: y - a.y.value,
                        age: 0.0,
                        released: false,
                        fade: 0.0,
                    });
                }
            } else if !down {
                for r in &mut a.ripples {
                    r.released = true;
                }
            }
        }
    }

    pub fn clear_ripples(&mut self, key: Key) {
        if let Some(&i) = self.index.get(&key) {
            self.items[i].ripples.clear();
        }
    }

    /// Topmost live, interactive node under a point (`offsets` maps scroll
    /// groups to their current offset).
    pub fn hit_test(
        &self,
        x: f32,
        y: f32,
        offsets: &dyn Fn(Option<u8>) -> f32,
        pad: f32,
    ) -> Option<Key> {
        let mut best: Option<(Key, f32)> = None;
        for a in self.items.iter().rev() {
            if a.exiting || a.node.hit.is_none() || a.opacity.target < 0.5 {
                continue;
            }
            let rect = a.rect().offset(-offsets(a.node.group));
            if rect.contains(x, y, 0.0) {
                return Some(a.node.key);
            }
            if rect.contains(x, y, pad) {
                let distance = (rect.cx() - x).abs();
                if best.is_none_or(|(_, d)| distance < d) {
                    best = Some((a.node.key, distance));
                }
            }
        }
        best.map(|(k, _)| k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_settles_on_target() {
        let mut s = Spring::new(0.0);
        s.target = 100.0;
        let p = SpringParams::new(0.4, 0.8);
        let mut t = 0.0;
        while s.step(1.0 / 60.0, p, 0.05) {
            t += 1.0 / 60.0;
            assert!(t < 3.0, "spring never settled");
        }
        assert_eq!(s.value, 100.0);
    }
}
