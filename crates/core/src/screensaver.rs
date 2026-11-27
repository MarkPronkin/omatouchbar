//! The Touch Bar half of Omarchy's screensaver.
//!
//! Omarchy's screensaver runs `ttfx` (its Rust port of TerminalTextEffects)
//! on the branding text in a fullscreen terminal, one random effect after
//! another. The renderer runs the same binary on the same text, sized to the
//! strip, and draws what it prints: every frame is the canvas's rows of
//! characters, colored with 24-bit foreground escapes. This module holds the
//! parts that don't touch processes: how the strip divides into terminal
//! cells, the frame parser, and drawing a frame.
//!
//! Cells keep a terminal's 1:2 shape and are as tall as the text allows, so
//! the logo fills the strip's height the way it sits in the middle of the
//! desktop. Block elements (most of the logo) are drawn as exact rectangles;
//! anything else is the font's glyph boxed down to the cell.

use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc, sync::Arc};

use crate::{canvas::Canvas, paths, text::Fonts};

/// The window class Omarchy's screensaver terminals use.
pub const WINDOW_CLASS: &str = "org.omarchy.screensaver";

/// Frames per second Omarchy runs `ttfx` at. Effects advance per frame, so
/// this is also their speed.
pub const FRAME_RATE: u32 = 120;

/// One character cell of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: [u8; 3],
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: [0, 0, 0],
    };
}

/// One frame of the effect: `rows` × `cols` cells, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Grid {
        Grid {
            cols,
            rows,
            cells: vec![Cell::BLANK; cols * rows],
        }
    }

    pub fn get(&self, col: usize, row: usize) -> Cell {
        self.cells[row * self.cols + col]
    }

    /// The text the frame shows, one line per row (tests and logs).
    pub fn text(&self) -> String {
        let mut out = String::new();
        for row in self.cells.chunks(self.cols.max(1)) {
            let line: String = row.iter().map(|c| c.ch).collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

/// How the strip divides into terminal cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub cols: u32,
    pub rows: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    /// Offset of the cell grid, centering it on the strip.
    pub x0: u32,
    pub y0: u32,
}

impl Geometry {
    /// The tallest cells that fit `text_rows` lines into `height` (cells
    /// keep a terminal's 1:2 shape), across the whole strip. `None` when
    /// the text is too tall to draw legibly.
    pub fn fit(text_rows: usize, width: u32, height: u32) -> Option<Geometry> {
        let text_rows = text_rows.max(1) as u32;
        let cell_h = (height / text_rows).min(12);
        if cell_h < 4 {
            return None;
        }
        let cell_w = ((cell_h as f32) * 0.5).round().max(2.0) as u32;
        let cols = width / cell_w;
        let rows = height / cell_h;
        Some(Geometry {
            cols,
            rows,
            cell_w,
            cell_h,
            x0: (width - cols * cell_w) / 2,
            y0: (height - rows * cell_h) / 2,
        })
    }
}

/// The text Omarchy's screensaver animates, ready for the strip: the user's
/// branding (`omarchy branding screensaver`), else Omarchy's logo, else
/// the word itself. Trailing blank lines and spaces are dropped so the line
/// count is the art's real height.
pub fn branding(width: u32, height: u32) -> (String, Geometry) {
    let candidates: [PathBuf; 2] = [
        paths::config_home().join("omarchy/branding/screensaver.txt"),
        paths::omarchy_root().join("logo.txt"),
    ];
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let text = tidy(&text);
        if text.is_empty() {
            continue;
        }
        match Geometry::fit(text.lines().count(), width, height) {
            Some(geometry) => return (text, geometry),
            None => log::info!(
                "screensaver: {} is too tall for the Touch Bar",
                path.display()
            ),
        }
    }
    let text = "omarchy".to_string();
    let geometry = Geometry::fit(1, width, height).expect("one line fits");
    (text, geometry)
}

fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim_end())
        .skip_while(|l| l.is_empty())
        .collect();
    let end = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(0, |i| i + 1);
    lines[..end].join("\n")
}

// ----------------------------------------------------------------- parser

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Ground,
    Escape,
    Csi,
    /// An OSC or other string sequence, skipped up to BEL or ST.
    String,
    StringEscape,
}

/// Turns `ttfx`'s terminal output back into frames.
///
/// `ttfx` prints a blank canvas, saves the cursor below it, and then draws
/// every frame by restoring the cursor, moving up to the canvas's top row
/// and printing each row, so a cursor restore ends a frame. This keeps a
/// small terminal's state (cursor, foreground color, UTF-8 decoding) across
/// arbitrary chunks of the stream.
pub struct Parser {
    grid: Grid,
    row: i32,
    col: i32,
    saved: (i32, i32),
    fg: Option<[u8; 3]>,
    state: State,
    params: Vec<u16>,
    param: Option<u16>,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
    drawn: bool,
}

/// Foreground for text printed without a color (the terminal's own).
const DEFAULT_FG: [u8; 3] = [0xe6, 0xe6, 0xe6];

impl Parser {
    pub fn new(cols: usize, rows: usize) -> Parser {
        Parser {
            grid: Grid::new(cols, rows),
            row: 0,
            col: 0,
            saved: (0, 0),
            fg: None,
            state: State::Ground,
            params: Vec::new(),
            param: None,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
            drawn: false,
        }
    }

    /// Feeds a chunk of output; `frame` sees each frame it completes.
    pub fn feed(&mut self, bytes: &[u8], frame: &mut impl FnMut(&Grid)) {
        for &b in bytes {
            match self.state {
                State::Ground => self.ground(b),
                State::Escape => self.escape(b, frame),
                State::Csi => self.csi(b),
                State::String => {
                    if b == 0x07 {
                        self.state = State::Ground;
                    } else if b == 0x1b {
                        self.state = State::StringEscape;
                    }
                }
                State::StringEscape => {
                    self.state = if b == b'\\' {
                        State::Ground
                    } else {
                        State::String
                    };
                }
            }
        }
    }

    /// Ends the stream: the last frame isn't followed by a cursor restore.
    pub fn finish(&mut self, frame: &mut impl FnMut(&Grid)) {
        if self.drawn {
            self.drawn = false;
            frame(&self.grid);
        }
    }

    fn ground(&mut self, b: u8) {
        if self.utf8_need > 0 {
            if b & 0xc0 == 0x80 {
                self.utf8[self.utf8_len] = b;
                self.utf8_len += 1;
                if self.utf8_len == self.utf8_need {
                    self.utf8_need = 0;
                    let ch = std::str::from_utf8(&self.utf8[..self.utf8_len])
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('\u{FFFD}');
                    self.print(ch);
                }
                return;
            }
            // A broken sequence: drop it and read this byte afresh.
            self.utf8_need = 0;
            self.print('\u{FFFD}');
        }
        match b {
            0x1b => self.state = State::Escape,
            b'\n' => {
                self.row += 1;
                self.col = 0;
            }
            b'\r' => self.col = 0,
            0x08 => self.col = (self.col - 1).max(0),
            0x00..=0x1f | 0x7f => {}
            0x20..=0x7e => self.print(b as char),
            _ => {
                let need = match b {
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    _ => 0,
                };
                if need == 0 {
                    self.print('\u{FFFD}');
                } else {
                    self.utf8[0] = b;
                    self.utf8_len = 1;
                    self.utf8_need = need;
                }
            }
        }
    }

    fn escape(&mut self, b: u8, frame: &mut impl FnMut(&Grid)) {
        self.state = State::Ground;
        match b {
            b'[' => {
                self.params.clear();
                self.param = None;
                self.state = State::Csi;
            }
            b']' | b'P' | b'X' | b'^' | b'_' => self.state = State::String,
            b'7' => self.saved = (self.row, self.col),
            b'8' => {
                (self.row, self.col) = self.saved;
                if self.drawn {
                    self.drawn = false;
                    frame(&self.grid);
                }
            }
            b'c' => {
                self.grid.cells.fill(Cell::BLANK);
                (self.row, self.col) = (0, 0);
            }
            _ => {}
        }
    }

    fn csi(&mut self, b: u8) {
        match b {
            b'0'..=b'9' => {
                let digit = (b - b'0') as u16;
                self.param = Some(
                    self.param
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(digit),
                );
            }
            b';' | b':' => self.params.push(self.param.take().unwrap_or(0)),
            0x20..=0x3f => {}
            0x40..=0x7e => {
                if let Some(p) = self.param.take() {
                    self.params.push(p);
                }
                self.state = State::Ground;
                self.command(b);
            }
            _ => self.state = State::Ground,
        }
    }

    fn command(&mut self, b: u8) {
        let n = |params: &[u16], i: usize| {
            params.get(i).copied().filter(|&v| v > 0).unwrap_or(1) as i32
        };
        match b {
            b'm' => self.sgr(),
            b'A' => self.row -= n(&self.params, 0),
            b'B' => self.row += n(&self.params, 0),
            b'C' => self.col += n(&self.params, 0),
            b'D' => self.col = (self.col - n(&self.params, 0)).max(0),
            b'E' => {
                self.row += n(&self.params, 0);
                self.col = 0;
            }
            b'F' => {
                self.row -= n(&self.params, 0);
                self.col = 0;
            }
            b'G' => self.col = n(&self.params, 0) - 1,
            b'H' | b'f' => {
                self.row = n(&self.params, 0) - 1;
                self.col = n(&self.params, 1) - 1;
            }
            b'J' if self.params.first() == Some(&2) => self.grid.cells.fill(Cell::BLANK),
            _ => {}
        }
    }

    fn sgr(&mut self) {
        if self.params.is_empty() {
            self.fg = None;
            return;
        }
        let p = std::mem::take(&mut self.params);
        let mut i = 0;
        while i < p.len() {
            match p[i] {
                0 | 39 => self.fg = None,
                code @ 30..=37 => self.fg = Some(ansi16(code as u8 - 30)),
                code @ 90..=97 => self.fg = Some(ansi16(code as u8 - 90 + 8)),
                38 if p.get(i + 1) == Some(&2) && i + 4 < p.len() => {
                    self.fg = Some([p[i + 2], p[i + 3], p[i + 4]].map(|v| v.min(255) as u8));
                    i += 4;
                }
                38 if p.get(i + 1) == Some(&5) && i + 2 < p.len() => {
                    self.fg = Some(ansi256(p[i + 2].min(255) as u8));
                    i += 2;
                }
                // Background colors are parsed past but not drawn: the
                // screensaver's terminal is black and ttfx never sets one.
                48 if p.get(i + 1) == Some(&2) => i += 4,
                48 if p.get(i + 1) == Some(&5) => i += 2,
                _ => {}
            }
            i += 1;
        }
        self.params = p;
        self.params.clear();
    }

    fn print(&mut self, ch: char) {
        if self.row >= 0
            && (self.row as usize) < self.grid.rows
            && self.col >= 0
            && (self.col as usize) < self.grid.cols
        {
            let i = self.row as usize * self.grid.cols + self.col as usize;
            self.grid.cells[i] = if ch == ' ' {
                Cell::BLANK
            } else {
                Cell {
                    ch,
                    fg: self.fg.unwrap_or(DEFAULT_FG),
                }
            };
        }
        self.col += 1;
        self.drawn = true;
    }
}

fn ansi16(i: u8) -> [u8; 3] {
    const COLORS: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    COLORS[i as usize & 15]
}

fn ansi256(i: u8) -> [u8; 3] {
    match i {
        0..=15 => ansi16(i),
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [level(i / 36), level((i / 6) % 6), level(i % 6)]
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            [v, v, v]
        }
    }
}

// ---------------------------------------------------------------- drawing

/// Coverage masks for each character at one cell size.
pub struct Glyphs {
    cell_w: u32,
    cell_h: u32,
    masks: RefCell<HashMap<char, Option<Rc<[u8]>>>>,
}

impl Glyphs {
    pub fn new(geometry: &Geometry) -> Glyphs {
        Glyphs {
            cell_w: geometry.cell_w,
            cell_h: geometry.cell_h,
            masks: RefCell::new(HashMap::new()),
        }
    }

    fn mask(&self, ch: char, fonts: &Fonts) -> Option<Rc<[u8]>> {
        if let Some(mask) = self.masks.borrow().get(&ch) {
            return mask.clone();
        }
        let (w, h) = (self.cell_w, self.cell_h);
        let coverage = block(ch, w, h).unwrap_or_else(|| {
            // Small glyphs come out faint once boxed down to a handful of
            // pixels; lift their coverage so they read as lit dots.
            let mut c = fonts.cell_coverage(ch, w, h);
            for v in &mut c {
                let f = *v as f32 / 255.0;
                *v = ((1.0 - (1.0 - f).powi(2)) * 255.0 + 0.5) as u8;
            }
            c
        });
        let mask: Option<Rc<[u8]>> = coverage.iter().any(|&v| v > 0).then(|| coverage.into());
        self.masks.borrow_mut().insert(ch, mask.clone());
        mask
    }
}

/// Exact coverage for block elements and the light box-drawing lines
/// `ttfx` uses, which a terminal draws edge to edge.
fn block(ch: char, w: u32, h: u32) -> Option<Vec<u8>> {
    let mut rects: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut shade = 1.0;
    let code = ch as u32;
    match code {
        0x2580 => rects.push((0.0, 0.0, 1.0, 0.5)),
        0x2581..=0x2588 => rects.push((0.0, 1.0 - (code - 0x2580) as f32 / 8.0, 1.0, 1.0)),
        0x2589..=0x258f => rects.push((0.0, 0.0, (0x2590 - code) as f32 / 8.0, 1.0)),
        0x2590 => rects.push((0.5, 0.0, 1.0, 1.0)),
        0x2591..=0x2593 => {
            rects.push((0.0, 0.0, 1.0, 1.0));
            shade = (code - 0x2590) as f32 * 0.25;
        }
        0x2594 => rects.push((0.0, 0.0, 1.0, 0.125)),
        0x2595 => rects.push((0.875, 0.0, 1.0, 1.0)),
        0x2596..=0x259f => {
            // Quadrants: upper left, upper right, lower left, lower right.
            const Q: [u8; 10] = [
                0b0010, 0b0001, 0b1000, 0b1011, 0b1001, 0b1110, 0b1101, 0b0100, 0b0110, 0b0111,
            ];
            let bits = Q[(code - 0x2596) as usize];
            let quads = [
                (0.0, 0.0, 0.5, 0.5),
                (0.5, 0.0, 1.0, 0.5),
                (0.0, 0.5, 0.5, 1.0),
                (0.5, 0.5, 1.0, 1.0),
            ];
            for (i, q) in quads.iter().enumerate() {
                if bits & (0b1000 >> i) != 0 {
                    rects.push(*q);
                }
            }
        }
        // Light and heavy horizontal and vertical lines, pixel-snapped so a
        // one-pixel stroke stays crisp.
        0x2500..=0x2503 => {
            let heavy = code & 1 == 1;
            let mut mask = vec![0u8; (w * h) as usize];
            if code < 0x2502 {
                let t = (if heavy { 2 } else { 1 }).min(h);
                let y0 = (h - t) / 2;
                for y in y0..y0 + t {
                    mask[(y * w) as usize..((y + 1) * w) as usize].fill(255);
                }
            } else {
                let t = (if heavy { 2 } else { 1 }).min(w);
                let x0 = (w - t) / 2;
                for y in 0..h {
                    for x in x0..x0 + t {
                        mask[(y * w + x) as usize] = 255;
                    }
                }
            }
            return Some(mask);
        }
        _ => return None,
    }
    let (fw, fh) = (w as f32, h as f32);
    let mut mask = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut cover = 0.0;
            for &(x0, y0, x1, y1) in &rects {
                let ox = ((x1 * fw).min(x as f32 + 1.0) - (x0 * fw).max(x as f32)).max(0.0);
                let oy = ((y1 * fh).min(y as f32 + 1.0) - (y0 * fh).max(y as f32)).max(0.0);
                cover += ox * oy;
            }
            mask[(y * w + x) as usize] = (cover.min(1.0) * shade * 255.0 + 0.5) as u8;
        }
    }
    Some(mask)
}

/// Draws a frame onto the strip at `opacity`.
pub fn draw(
    canvas: &mut Canvas,
    grid: &Grid,
    geometry: &Geometry,
    glyphs: &Glyphs,
    fonts: &Fonts,
    opacity: f32,
) {
    let cols = grid.cols.min(geometry.cols as usize);
    let rows = grid.rows.min(geometry.rows as usize);
    for row in 0..rows {
        for col in 0..cols {
            let cell = grid.get(col, row);
            if cell.ch == ' ' {
                continue;
            }
            let Some(mask) = glyphs.mask(cell.ch, fonts) else {
                continue;
            };
            canvas.blend_mask(
                geometry.x0 + col as u32 * geometry.cell_w,
                geometry.y0 + row as u32 * geometry.cell_h,
                geometry.cell_w,
                geometry.cell_h,
                &mask,
                cell.fg,
                opacity,
            );
        }
    }
}

/// The screensaver as the scene shows it.
pub(crate) struct Saver {
    pub geometry: Geometry,
    pub glyphs: Glyphs,
    pub frame: Option<Arc<Grid>>,
    /// A frame arrived since the last update.
    pub fresh: bool,
}

impl Saver {
    pub fn new(geometry: Geometry) -> Saver {
        Saver {
            glyphs: Glyphs::new(&geometry),
            geometry,
            frame: None,
            fresh: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What ttfx prints for a 4×2 canvas: the blank canvas, then each frame
    /// from a saved cursor below it.
    fn stream(frames: &[[&str; 2]]) -> Vec<u8> {
        let mut out = b"\x1b[?25l    \n    \n\x1b7".to_vec();
        for frame in frames {
            out.extend_from_slice(b"\x1b8\x1b7\x1b[2A");
            for (i, row) in frame.iter().enumerate() {
                for ch in row.chars() {
                    if ch == ' ' {
                        out.push(b' ');
                    } else {
                        out.extend_from_slice(b"\x1b[38;2;10;200;255m");
                        out.extend_from_slice(ch.to_string().as_bytes());
                        out.extend_from_slice(b"\x1b[0m");
                    }
                }
                if i == 0 {
                    out.push(b'\n');
                }
            }
        }
        out
    }

    #[test]
    fn parses_frames_across_chunk_boundaries() {
        let bytes = stream(&[["█▀  ", "    "], ["█▀▄ ", " ░ █"]]);
        for chunk in [1, 3, 7, bytes.len()] {
            let mut parser = Parser::new(4, 2);
            let mut frames = Vec::new();
            for part in bytes.chunks(chunk) {
                parser.feed(part, &mut |g| frames.push(g.text()));
            }
            parser.finish(&mut |g| frames.push(g.text()));
            // The blank canvas, then both frames (the last one at the end).
            assert_eq!(
                frames,
                vec![
                    "\n\n".to_string(),
                    "█▀\n\n".to_string(),
                    "█▀▄\n ░ █\n".to_string()
                ],
                "chunk size {chunk}"
            );
        }
        let mut parser = Parser::new(4, 2);
        let mut last = None;
        parser.feed(&bytes, &mut |_| {});
        parser.finish(&mut |g| last = Some(g.clone()));
        let grid = last.unwrap();
        assert_eq!(grid.get(1, 1).ch, '░');
        assert_eq!(grid.get(1, 1).fg, [10, 200, 255]);
        assert_eq!(grid.get(0, 1), Cell::BLANK);
    }

    #[test]
    fn real_ttfx_output_round_trips() {
        let Ok(output) = std::process::Command::new("ttfx")
            .args([
                "--seed",
                "1",
                "--frame-rate",
                "0",
                "--canvas-width",
                "40",
                "--canvas-height",
                "3",
                "--ignore-terminal-dimensions",
                "--anchor-canvas",
                "c",
                "--anchor-text",
                "c",
                "--no-eol",
                "--no-restore-cursor",
                "wipe",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all("omarchy\n▄▀█".as_bytes())?;
                child.wait_with_output()
            })
        else {
            return; // ttfx isn't installed here.
        };
        let mut parser = Parser::new(40, 3);
        let mut count = 0;
        let mut last = None;
        parser.feed(&output.stdout, &mut |_| count += 1);
        parser.finish(&mut |g| last = Some(g.text()));
        assert!(count > 5, "{count} frames");
        let last = last.expect("final frame");
        assert!(last.contains("omarchy"), "{last}");
        assert!(last.contains("▄▀█"), "{last}");
    }

    #[test]
    fn fits_the_logo_to_the_strip() {
        let g = Geometry::fit(10, 2170, 60).unwrap();
        assert_eq!((g.cell_w, g.cell_h, g.cols, g.rows), (3, 6, 723, 10));
        assert_eq!((g.x0, g.y0), (0, 0));
        let g = Geometry::fit(7, 2170, 60).unwrap();
        assert_eq!((g.cell_w, g.cell_h, g.rows, g.y0), (4, 8, 7, 2));
        assert_eq!(Geometry::fit(1, 2170, 60).unwrap().cell_h, 12);
        assert!(Geometry::fit(30, 2170, 60).is_none());
    }

    #[test]
    fn tidies_art() {
        assert_eq!(tidy("\n\n  ab  \n c\n\n  \n"), "  ab\n c");
        assert_eq!(tidy("\n \n"), "");
    }

    #[test]
    fn blocks_cover_their_part_of_the_cell() {
        let sum = |m: Vec<u8>| m.iter().map(|&v| v as u32).sum::<u32>();
        let full = sum(block('█', 3, 6).unwrap());
        assert_eq!(full, 255 * 18);
        assert_eq!(sum(block('▀', 3, 6).unwrap()), full / 2);
        assert_eq!(sum(block('▄', 3, 6).unwrap()), full / 2);
        let top = block('▀', 3, 6).unwrap();
        assert!(top[..9].iter().all(|&v| v == 255) && top[9..].iter().all(|&v| v == 0));
        // ▌ covers a cell and a half of three: the middle column half lit.
        let left = block('▌', 3, 6).unwrap();
        assert_eq!(&left[..3], &[255, 128, 0]);
        assert_eq!(sum(block('▙', 4, 8).unwrap()), 255 * 24);
        assert!(block('a', 3, 6).is_none());
    }
}
