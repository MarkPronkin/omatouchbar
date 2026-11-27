//! Font loading, per-character fallback and a glyph cache.
//!
//! Fonts are found through fontconfig (`fc-match`) at runtime, so the
//! Touch Bar uses the same monospace and Nerd Font icons as the Omarchy bar.
//! Characters none of the loaded fonts cover (Japanese, Chinese, Korean,
//! Arabic, emoji, …) are resolved on first use by asking fontconfig for a
//! font with that character, which also honors Omarchy's CJK preference
//! (`omarchy font cjk`). Color emoji (CBDT/sbix PNG strikes) are drawn in
//! color.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
};

use ab_glyph::{Font, FontVec, GlyphId, GlyphImageFormat, PxScale};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Text,
    Bold,
    Icon,
    Logo,
}

const ROLES: usize = 4;

fn role_index(role: Role) -> usize {
    match role {
        Role::Text => 0,
        Role::Bold => 1,
        Role::Icon => 2,
        Role::Logo => 3,
    }
}

struct Face {
    font: FontVec,
    /// px-scale per em px (`height_unscaled / units_per_em`).
    scale_per_em: f32,
    upem: f32,
    /// Cap height as a fraction of the em.
    cap: f32,
}

impl Face {
    fn load(path: &Path, index: u32) -> Option<Face> {
        let data = std::fs::read(path).ok()?;
        let font = FontVec::try_from_vec_and_index(data, index).ok()?;
        let upem = font.units_per_em().unwrap_or(1000.0);
        let scale_per_em = font.height_unscaled() / upem;
        let mut face = Face {
            font,
            scale_per_em,
            upem,
            cap: 0.7,
        };
        let h = face.font.glyph_id('H');
        if h.0 != 0
            && let Some(outline) = face.font.outline(h)
        {
            let height = outline.bounds.max.y - outline.bounds.min.y;
            if height > 0.0 {
                face.cap = height / upem;
            }
        }
        Some(face)
    }

    fn px(&self, em: f32) -> PxScale {
        PxScale::from(em * self.scale_per_em)
    }

    fn advance(&self, id: GlyphId, em: f32) -> f32 {
        self.font.h_advance_unscaled(id) * em / self.upem
    }
}

/// One rasterized glyph: coverage (or color pixels) plus its offset from
/// the pen position.
pub struct Bitmap {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
    /// Premultiplied RGBA for color glyphs (emoji); drawn as-is.
    pub rgba: Option<Vec<u8>>,
}

impl Bitmap {
    fn empty() -> Bitmap {
        Bitmap {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            coverage: Vec::new(),
            rgba: None,
        }
    }
}

/// A glyph placed along a run, relative to the run's origin.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    face: u16,
    id: GlyphId,
    pub x: f32,
}

pub struct Run {
    pub glyphs: Vec<Placed>,
    pub width: f32,
    pub size: f32,
    role: Role,
}

type Key = (u16, u16, u16, u8);

/// A two-generation glyph cache: when the current generation fills up it
/// becomes the previous one, and glyphs still in use are promoted back on
/// their next lookup. Overflow never empties the cache at once, so a long
/// animation can't trigger a frame that re-rasterizes everything.
#[derive(Default)]
struct GlyphCache {
    current: HashMap<Key, Rc<Bitmap>>,
    previous: HashMap<Key, Rc<Bitmap>>,
}

impl GlyphCache {
    const GENERATION: usize = 4096;

    fn get(&mut self, key: &Key) -> Option<Rc<Bitmap>> {
        if let Some(bitmap) = self.current.get(key) {
            return Some(bitmap.clone());
        }
        let bitmap = self.previous.remove(key)?;
        self.insert(*key, bitmap.clone());
        Some(bitmap)
    }

    fn insert(&mut self, key: Key, bitmap: Rc<Bitmap>) {
        if self.current.len() >= Self::GENERATION {
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.insert(key, bitmap);
    }

    fn len(&self) -> usize {
        self.current.len() + self.previous.len()
    }
}

pub struct Fonts {
    /// fontconfig family used for text fallback lookups.
    family: String,
    faces: RefCell<Vec<Rc<Face>>>,
    chains: RefCell<[Vec<u16>; ROLES]>,
    loaded: RefCell<HashMap<(PathBuf, u32), u16>>,
    /// Characters already looked up in fontconfig: (char, bold) → face.
    discovered: RefCell<HashMap<(char, bool), Option<u16>>>,
    /// Whether runtime fontconfig fallback is allowed.
    dynamic: bool,
    cache: RefCell<GlyphCache>,
    measure_cache: RefCell<HashMap<(String, u16, Role), f32>>,
}

/// Resolves a fontconfig pattern to a font file and collection index.
pub fn find_font_indexed(pattern: &str) -> Option<(PathBuf, u32)> {
    let output = Command::new("fc-match")
        .args(["-f", "%{file}\n%{index}", pattern])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let mut lines = text.lines();
    let path = PathBuf::from(lines.next()?.trim());
    let index = lines
        .next()
        .and_then(|i| i.trim().parse().ok())
        .unwrap_or(0);
    path.is_file().then_some((path, index))
}

/// Resolves a fontconfig pattern to a font file.
pub fn find_font(pattern: &str) -> Option<PathBuf> {
    find_font_indexed(pattern).map(|(path, _)| path)
}

/// Characters that only modify their neighbors and draw nothing on their own.
fn invisible(ch: char) -> bool {
    matches!(ch, '\u{200B}'..='\u{200F}' | '\u{FE00}'..='\u{FE0F}' | '\u{2060}' | '\u{E0020}'..='\u{E007F}')
}

impl Fonts {
    /// Loads the system fonts used by the Omarchy bar.
    pub fn system(family: Option<&str>) -> Fonts {
        let family = family
            .filter(|f| !f.is_empty())
            .unwrap_or("monospace")
            .to_string();
        let text = find_font_indexed(&format!("{family}:style=Regular"));
        let bold = find_font_indexed(&format!("{family}:style=Bold"));
        let icons = find_font_indexed("Symbols Nerd Font")
            .filter(|(p, _)| p.to_string_lossy().to_lowercase().contains("nerd"))
            .or_else(|| find_font_indexed("JetBrainsMono Nerd Font"));
        let logo =
            find_font_indexed("omarchy").filter(|(p, _)| p.to_string_lossy().contains("omarchy"));
        let sans = find_font_indexed("sans-serif");
        let mut fonts = Fonts::empty(family, true);
        let text = fonts.add(text);
        let bold = fonts.add(bold).or(text);
        let icons = fonts.add(icons);
        let logo = fonts.add(logo);
        let sans = fonts.add(sans);
        fonts.set_chains(text, bold, icons, logo, sans);
        fonts
    }

    /// Fixed fonts without runtime fallback (tests and tools).
    pub fn from_files(
        text: Option<&Path>,
        bold: Option<&Path>,
        icons: Option<&Path>,
        logo: Option<&Path>,
        fallback: Option<&Path>,
    ) -> Fonts {
        let mut fonts = Fonts::empty("monospace".into(), false);
        let entry = |p: Option<&Path>| p.map(|p| (p.to_path_buf(), 0));
        let text = fonts.add(entry(text));
        let bold = fonts.add(entry(bold)).or(text);
        let icons = fonts.add(entry(icons));
        let logo = fonts.add(entry(logo));
        let sans = fonts.add(entry(fallback));
        fonts.set_chains(text, bold, icons, logo, sans);
        fonts
    }

    fn empty(family: String, dynamic: bool) -> Fonts {
        Fonts {
            family,
            faces: RefCell::new(Vec::new()),
            chains: RefCell::new(Default::default()),
            loaded: RefCell::new(HashMap::new()),
            discovered: RefCell::new(HashMap::new()),
            dynamic,
            cache: RefCell::new(GlyphCache::default()),
            measure_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Loads (or reuses) a face; returns its index.
    fn add(&self, entry: Option<(PathBuf, u32)>) -> Option<u16> {
        let (path, index) = entry?;
        if let Some(face) = self.loaded.borrow().get(&(path.clone(), index)) {
            return Some(*face);
        }
        let face = Face::load(&path, index)?;
        let mut faces = self.faces.borrow_mut();
        faces.push(Rc::new(face));
        let id = (faces.len() - 1) as u16;
        self.loaded.borrow_mut().insert((path, index), id);
        Some(id)
    }

    fn set_chains(
        &mut self,
        text: Option<u16>,
        bold: Option<u16>,
        icons: Option<u16>,
        logo: Option<u16>,
        sans: Option<u16>,
    ) {
        let chain = |items: &[Option<u16>]| {
            let mut out: Vec<u16> = Vec::new();
            for item in items.iter().flatten() {
                if !out.contains(item) {
                    out.push(*item);
                }
            }
            out
        };
        *self.chains.borrow_mut() = [
            chain(&[text, icons, sans, logo]),
            chain(&[bold, text, icons, sans, logo]),
            chain(&[icons, text, sans, logo]),
            chain(&[logo, icons, text, sans]),
        ];
    }

    pub fn is_empty(&self) -> bool {
        self.faces.borrow().is_empty()
    }

    fn face(&self, index: u16) -> Rc<Face> {
        self.faces.borrow()[index as usize].clone()
    }

    fn lookup(&self, role: Role, ch: char) -> Option<(u16, GlyphId)> {
        {
            let chains = self.chains.borrow();
            let faces = self.faces.borrow();
            for &face in &chains[role_index(role)] {
                let id = faces[face as usize].font.glyph_id(ch);
                if id.0 != 0 {
                    return Some((face, id));
                }
            }
        }
        if let Some(face) = self.discover(role, ch) {
            let id = self.faces.borrow()[face as usize].font.glyph_id(ch);
            return Some((face, id));
        }
        // Nothing covers it: draw nothing, but keep the text's metrics.
        let chains = self.chains.borrow();
        let face = *chains[role_index(role)].first()?;
        Some((face, self.faces.borrow()[face as usize].font.glyph_id(ch)))
    }

    /// Asks fontconfig for a font that has `ch` and adds it to the chains.
    fn discover(&self, role: Role, ch: char) -> Option<u16> {
        if !self.dynamic || ch.is_control() || ch.is_whitespace() {
            return None;
        }
        let bold = role == Role::Bold;
        if let Some(found) = self.discovered.borrow().get(&(ch, bold)) {
            return *found;
        }
        let family = match role {
            Role::Text | Role::Bold => self.family.as_str(),
            Role::Icon | Role::Logo => "sans-serif",
        };
        let weight = if bold { ":weight=bold" } else { "" };
        let pattern = format!("{family}{weight}:charset={:x}", ch as u32);
        let found = find_font_indexed(&pattern)
            .and_then(|entry| self.add(Some(entry)))
            .filter(|&face| self.faces.borrow()[face as usize].font.glyph_id(ch).0 != 0);
        if let Some(face) = found {
            let mut chains = self.chains.borrow_mut();
            // Bold lookups extend the bold chain; others extend the rest.
            let targets: &[usize] = if bold { &[1] } else { &[0, 2, 3] };
            for &target in targets {
                if !chains[target].contains(&face) {
                    chains[target].push(face);
                }
            }
            // A bold-less script still renders in bold runs.
            if !bold && !chains[1].contains(&face) {
                chains[1].push(face);
            }
            log::debug!("font fallback for U+{:04X}: {pattern}", ch as u32);
        }
        self.discovered.borrow_mut().insert((ch, bold), found);
        found
    }

    /// Lays out a single line of text.
    pub fn layout(&self, text: &str, size: f32, role: Role) -> Run {
        let mut glyphs = Vec::with_capacity(text.len());
        let mut x = 0.0f32;
        let mut previous: Option<(u16, GlyphId)> = None;
        for ch in text.chars() {
            if ch == '\n' || invisible(ch) {
                continue;
            }
            let Some((face, id)) = self.lookup(role, ch) else {
                continue;
            };
            let f = self.face(face);
            if let Some((pf, pid)) = previous
                && pf == face
            {
                x += f.font.kern_unscaled(pid, id) * size / f.upem;
            }
            glyphs.push(Placed { face, id, x });
            x += f.advance(id, size);
            previous = Some((face, id));
        }
        Run {
            glyphs,
            width: x,
            size,
            role,
        }
    }

    pub fn measure(&self, text: &str, size: f32, role: Role) -> f32 {
        let key = (text.to_string(), (size * 4.0) as u16, role);
        if let Some(width) = self.measure_cache.borrow().get(&key) {
            return *width;
        }
        let width = self.layout(text, size, role).width;
        let mut cache = self.measure_cache.borrow_mut();
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(key, width);
        width
    }

    /// Cap height in px for the role's primary face.
    pub fn cap_height(&self, role: Role, size: f32) -> f32 {
        let first = self.chains.borrow()[role_index(role)].first().copied();
        first.map(|f| self.face(f).cap * size).unwrap_or(size * 0.7)
    }

    /// Rasterizes (or fetches) one glyph at a quarter-pixel horizontal phase.
    pub fn bitmap(&self, placed: &Placed, size: f32, phase: u8) -> Option<Rc<Bitmap>> {
        let size_q = (size * 4.0).round().clamp(1.0, 65535.0) as u16;
        let key = (placed.face, placed.id.0, size_q, phase);
        if let Some(bitmap) = self.cache.borrow_mut().get(&key) {
            return (bitmap.width > 0).then_some(bitmap);
        }
        let face = self.face(placed.face);
        let size = size_q as f32 / 4.0;
        let glyph = placed
            .id
            .with_scale_and_position(face.px(size), ab_glyph::point(phase as f32 / 4.0, 0.0));
        // Empty glyphs (spaces) are cached too, so they never miss again.
        let bitmap = match face.font.outline_glyph(glyph) {
            Some(outlined) => {
                let bounds = outlined.px_bounds();
                let width = bounds.width().max(0.0) as u32;
                let height = bounds.height().max(0.0) as u32;
                let mut coverage = vec![0u8; (width * height) as usize];
                outlined.draw(|x, y, c| {
                    if x < width && y < height {
                        coverage[(y * width + x) as usize] =
                            (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    }
                });
                Bitmap {
                    left: bounds.min.x.floor() as i32,
                    top: bounds.min.y.floor() as i32,
                    width,
                    height,
                    coverage,
                    rgba: None,
                }
            }
            None => color_glyph(&face, placed.id, size).unwrap_or_else(Bitmap::empty),
        };
        let bitmap = Rc::new(bitmap);
        self.cache.borrow_mut().insert(key, bitmap.clone());
        (bitmap.width > 0).then_some(bitmap)
    }

    /// Coverage (`w`×`h`, row-major) of `ch` drawn the way a terminal draws
    /// it in a `w`×`h` cell: the text font's line height fills the cell and
    /// the glyph is centered on its advance. Rendered four times larger and
    /// boxed down, so even a cell a few pixels across keeps the glyph's
    /// shape. Empty for characters no font has an outline for.
    pub fn cell_coverage(&self, ch: char, w: u32, h: u32) -> Vec<u8> {
        const K: u32 = 4;
        let mut out = vec![0u8; (w * h) as usize];
        let Some((face, id)) = self.lookup(Role::Text, ch) else {
            return out;
        };
        let f = self.face(face);
        let (big_w, big_h) = (w * K, h * K);
        let line = f.font.height_unscaled().max(1.0);
        let px = big_h as f32 / line;
        let advance = f.font.h_advance_unscaled(id) * px;
        let baseline = f.font.ascent_unscaled() * px;
        let glyph = id.with_scale_and_position(
            PxScale::from(big_h as f32),
            ab_glyph::point((big_w as f32 - advance) / 2.0, baseline),
        );
        let Some(outlined) = f.font.outline_glyph(glyph) else {
            return out;
        };
        let bounds = outlined.px_bounds();
        let mut sums = vec![0f32; (w * h) as usize];
        outlined.draw(|x, y, c| {
            let x = x as i32 + bounds.min.x.floor() as i32;
            let y = y as i32 + bounds.min.y.floor() as i32;
            if x >= 0 && y >= 0 && (x as u32) < big_w && (y as u32) < big_h {
                sums[((y as u32 / K) * w + x as u32 / K) as usize] += c.clamp(0.0, 1.0);
            }
        });
        for (o, s) in out.iter_mut().zip(sums) {
            *o = (s / (K * K) as f32 * 255.0 + 0.5).min(255.0) as u8;
        }
        out
    }

    /// Cached glyph count (for diagnostics).
    pub fn cached_glyphs(&self) -> usize {
        self.cache.borrow().len()
    }

    /// Vertical extent (top, bottom) of a run's ink relative to its baseline.
    pub fn ink_extent(&self, run: &Run) -> Option<(i32, i32)> {
        let mut top = i32::MAX;
        let mut bottom = i32::MIN;
        for g in &run.glyphs {
            if let Some(b) = self.bitmap(g, run.size, 0) {
                top = top.min(b.top);
                bottom = bottom.max(b.top + b.height as i32);
            }
        }
        (top <= bottom).then_some((top, bottom))
    }

    /// Horizontal ink extent `(left, right)` relative to the pen origin.
    /// Icon glyphs often overhang their advance, so chips size and place
    /// icons by ink rather than advance.
    pub fn ink_hbounds(&self, text: &str, size: f32, role: Role) -> (f32, f32) {
        let run = self.layout(text, size, role);
        let mut left = f32::MAX;
        let mut right = f32::MIN;
        for g in &run.glyphs {
            if let Some(b) = self.bitmap(g, size, 0) {
                left = left.min(g.x + b.left as f32);
                right = right.max(g.x + b.left as f32 + b.width as f32);
            }
        }
        if left > right {
            (0.0, run.width)
        } else {
            (left, right)
        }
    }

    pub fn role_of(run: &Run) -> Role {
        run.role
    }
}

/// Decodes and scales a color bitmap glyph (Noto Color Emoji and friends).
fn color_glyph(face: &Face, id: GlyphId, size: f32) -> Option<Bitmap> {
    let image = face
        .font
        .glyph_raster_image2(id, size.round().max(1.0) as u16)?;
    let source = match image.format {
        GlyphImageFormat::Png => tiny_skia::Pixmap::decode_png(image.data).ok()?,
        _ => return None,
    };
    // Emoji sit on the baseline at about the em size.
    let target = (size * 1.08).round().max(1.0);
    let scale = target / source.height().max(1) as f32;
    let width = ((source.width() as f32) * scale).round().max(1.0) as u32;
    let height = target as u32;
    let mut scaled = tiny_skia::Pixmap::new(width, height)?;
    let paint = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..Default::default()
    };
    scaled.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &paint,
        tiny_skia::Transform::from_scale(scale, scale),
        None,
    );
    Some(Bitmap {
        left: ((face.advance(id, size) - width as f32) / 2.0).round() as i32,
        top: -((height as f32) * 0.86).round() as i32,
        width,
        height,
        coverage: Vec::new(),
        rgba: Some(scaled.data().to_vec()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_cjk_fonts_when_installed() {
        if find_font("monospace").is_none() {
            return;
        }
        let fonts = Fonts::system(None);
        let has_cjk = find_font_indexed("sans-serif:charset=3042")
            .is_some_and(|(p, _)| p.to_string_lossy().to_lowercase().contains("cjk"));
        if !has_cjk {
            return;
        }
        let run = fonts.layout("夜に駆ける", 22.0, Role::Text);
        assert_eq!(run.glyphs.len(), 5);
        for g in &run.glyphs {
            assert!(fonts.bitmap(g, 22.0, 0).is_some(), "CJK glyph not rendered");
        }
    }
}
