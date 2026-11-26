//! Exclusive Wayland keyboard capture via `zwlr_layer_shell_v1` + `libxkbcommon`.
//!
//! While the Touch Bar Wi-Fi password prompt is open, we map an invisible 1x1
//! transparent overlay layer surface with exclusive keyboard interactivity and
//! an empty input region. Every keystroke goes straight to the Touch Bar field
//! (never leaking into whatever desktop window is behind it), and closing the
//! prompt tears down the Wayland connection immediately.

use std::{
    collections::VecDeque,
    ffi::{c_char, c_int, c_void},
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::FileTypeExt,
    },
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use omatouchbar_core::{KeyInput, paths};
use rustix::{
    event::{EventfdFlags, PollFd, PollFlags},
    fs::MemfdFlags,
    mm::{MapFlags, ProtFlags},
    net::{
        AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
        SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketFlags, SocketType,
    },
};

use crate::bus::{Bus, Event};

// ------------------------------------------------------------ libxkbcommon FFI

#[repr(C)]
struct XkbContext {
    _private: [u8; 0],
}
#[repr(C)]
struct XkbKeymap {
    _private: [u8; 0],
}
#[repr(C)]
struct XkbState {
    _private: [u8; 0],
}

const XKB_KEYMAP_FORMAT_TEXT_V1: u32 = 1;
const XKB_STATE_MODS_EFFECTIVE: u32 = 1 << 3;

const XKB_KEY_UP: u32 = 0xff52;
const XKB_KEY_DOWN: u32 = 0xff54;
const XKB_KEY_BACKSPACE: u32 = 0xff08;
const XKB_KEY_RETURN: u32 = 0xff0d;
const XKB_KEY_ESCAPE: u32 = 0xff1b;
const XKB_KEY_KP_ENTER: u32 = 0xff8d;

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_context_new(flags: u32) -> *mut XkbContext;
    fn xkb_context_unref(context: *mut XkbContext);
    fn xkb_keymap_new_from_buffer(
        context: *mut XkbContext,
        buffer: *const c_char,
        length: usize,
        format: u32,
        flags: u32,
    ) -> *mut XkbKeymap;
    fn xkb_keymap_unref(keymap: *mut XkbKeymap);
    fn xkb_keymap_key_repeats(keymap: *mut XkbKeymap, key: u32) -> c_int;
    fn xkb_state_new(keymap: *mut XkbKeymap) -> *mut XkbState;
    fn xkb_state_unref(state: *mut XkbState);
    fn xkb_state_update_mask(
        state: *mut XkbState,
        depressed_mods: u32,
        latched_mods: u32,
        locked_mods: u32,
        depressed_layout: u32,
        latched_layout: u32,
        locked_layout: u32,
    ) -> u32;
    fn xkb_state_key_get_one_sym(state: *mut XkbState, key: u32) -> u32;
    fn xkb_state_key_get_utf8(
        state: *mut XkbState,
        key: u32,
        buffer: *mut c_char,
        size: usize,
    ) -> c_int;
    fn xkb_state_mod_name_is_active(state: *mut XkbState, name: *const c_char, ty: u32) -> c_int;
}

struct Xkb {
    ctx: *mut XkbContext,
    keymap: *mut XkbKeymap,
    state: *mut XkbState,
}

impl Xkb {
    fn new() -> Option<Self> {
        // SAFETY: xkb_context_new(0) creates an independent context.
        let ctx = unsafe { xkb_context_new(0) };
        if ctx.is_null() {
            return None;
        }
        Some(Self {
            ctx,
            keymap: std::ptr::null_mut(),
            state: std::ptr::null_mut(),
        })
    }

    fn load_keymap(&mut self, fd: OwnedFd, size: usize) {
        if size == 0 || size > 4 * 1024 * 1024 {
            return;
        }
        // SAFETY: Wayland keymap fd is mapped read-only + MAP_PRIVATE (required since wl_seat v7).
        let raw = unsafe {
            match rustix::mm::mmap(
                std::ptr::null_mut(),
                size,
                ProtFlags::READ,
                MapFlags::PRIVATE,
                &fd,
                0,
            ) {
                Ok(ptr) => ptr,
                Err(e) => {
                    log::warn!("kbd: mmap keymap: {e}");
                    return;
                }
            }
        };
        let bytes = unsafe { std::slice::from_raw_parts(raw.cast::<u8>(), size) };
        let len = bytes
            .iter()
            .rposition(|&b| b != 0)
            .map(|i| i + 1)
            .unwrap_or(0);
        let keymap = unsafe {
            xkb_keymap_new_from_buffer(
                self.ctx,
                raw.cast::<c_char>(),
                len,
                XKB_KEYMAP_FORMAT_TEXT_V1,
                0,
            )
        };
        unsafe {
            let _ = rustix::mm::munmap(raw, size);
        }
        if keymap.is_null() {
            log::warn!("kbd: xkb_keymap_new_from_buffer failed");
            return;
        }
        let state = unsafe { xkb_state_new(keymap) };
        if state.is_null() {
            unsafe { xkb_keymap_unref(keymap) };
            return;
        }
        unsafe {
            if !self.state.is_null() {
                xkb_state_unref(self.state);
            }
            if !self.keymap.is_null() {
                xkb_keymap_unref(self.keymap);
            }
        }
        self.keymap = keymap;
        self.state = state;
    }

    fn update_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if !self.state.is_null() {
            unsafe {
                xkb_state_update_mask(self.state, depressed, latched, locked, 0, 0, group);
            }
        }
    }

    fn key_repeats(&self, evdev_key: u32) -> bool {
        if self.keymap.is_null() {
            return false;
        }
        unsafe { xkb_keymap_key_repeats(self.keymap, evdev_key + 8) != 0 }
    }

    fn translate(&self, evdev_key: u32) -> Option<KeyInput> {
        if self.state.is_null() {
            return None;
        }
        let xkb_key = evdev_key + 8;
        let sym = unsafe { xkb_state_key_get_one_sym(self.state, xkb_key) };
        let ctrl = unsafe {
            xkb_state_mod_name_is_active(self.state, c"Control".as_ptr(), XKB_STATE_MODS_EFFECTIVE)
                > 0
        };
        let logo = unsafe {
            xkb_state_mod_name_is_active(self.state, c"Mod4".as_ptr(), XKB_STATE_MODS_EFFECTIVE) > 0
        };

        match sym {
            XKB_KEY_UP => return Some(KeyInput::Select(-1)),
            XKB_KEY_DOWN => return Some(KeyInput::Select(1)),
            XKB_KEY_ESCAPE => return Some(KeyInput::Escape),
            XKB_KEY_RETURN | XKB_KEY_KP_ENTER => return Some(KeyInput::Enter),
            XKB_KEY_BACKSPACE => {
                return Some(if ctrl {
                    KeyInput::Clear
                } else {
                    KeyInput::Backspace
                });
            }
            _ => {}
        }

        if ctrl || logo {
            // Handle common editing shortcuts (Ctrl+U clear, Ctrl/Super+V paste).
            let lower = sym | 0x20;
            if ctrl && lower == b'u' as u32 {
                return Some(KeyInput::Clear);
            }
            if lower == b'v' as u32 {
                if let Some(text) = read_clipboard() {
                    return Some(KeyInput::Paste(text));
                }
                return None;
            }
            return None;
        }

        let mut buf = [0u8; 32];
        let n = unsafe {
            xkb_state_key_get_utf8(
                self.state,
                xkb_key,
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len(),
            )
        };
        if n <= 0 {
            return None;
        }
        let text = std::str::from_utf8(&buf[..(n as usize).min(buf.len() - 1)]).ok()?;
        let clean: String = text
            .chars()
            .filter(|c| !c.is_control() && *c != '\u{7f}')
            .collect();
        (!clean.is_empty()).then_some(KeyInput::Text(clean))
    }
}

impl Drop for Xkb {
    fn drop(&mut self) {
        unsafe {
            if !self.state.is_null() {
                xkb_state_unref(self.state);
            }
            if !self.keymap.is_null() {
                xkb_keymap_unref(self.keymap);
            }
            if !self.ctx.is_null() {
                xkb_context_unref(self.ctx);
            }
        }
    }
}

fn read_clipboard() -> Option<String> {
    let out = Command::new("wl-paste")
        .args(["--no-newline"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    (!text.is_empty()).then_some(text)
}

// ------------------------------------------------------ Wayland wire helpers

const ID_DISPLAY: u32 = 1;
const ID_REGISTRY: u32 = 2;
const ID_SYNC_GLOBALS: u32 = 3;
const ID_COMPOSITOR: u32 = 4;
const ID_SHM: u32 = 5;
const ID_SEAT: u32 = 6;
const ID_LAYER_SHELL: u32 = 7;
const ID_SURFACE: u32 = 8;
const ID_REGION: u32 = 9;
const ID_LAYER_SURFACE: u32 = 10;
const ID_SHM_POOL: u32 = 11;
const ID_BUFFER: u32 = 12;
const ID_KEYBOARD: u32 = 13;

const LAYER_OVERLAY: u32 = 3;
const ANCHOR_TOP_LEFT: u32 = 1 | 4;
const KEYBOARD_EXCLUSIVE: u32 = 1;
const SHM_FORMAT_ARGB8888: u32 = 0;

struct WaylandConn {
    fd: OwnedFd,
    rx_buf: Vec<u8>,
    rx_fds: VecDeque<OwnedFd>,
    tx_buf: Vec<u8>,
}

impl WaylandConn {
    fn connect() -> io::Result<Self> {
        let path = wayland_socket_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Wayland display socket not found")
        })?;
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::connect(&fd, &SocketAddrUnix::new(&path)?)?;
        let flags = rustix::fs::fcntl_getfl(&fd)?;
        rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK)?;
        Ok(Self {
            fd,
            rx_buf: Vec::with_capacity(4096),
            rx_fds: VecDeque::new(),
            tx_buf: Vec::with_capacity(512),
        })
    }

    fn push_msg(&mut self, object: u32, opcode: u16, args: &[u8]) {
        let size = (8 + args.len()) as u32;
        self.tx_buf.extend_from_slice(&object.to_le_bytes());
        self.tx_buf
            .extend_from_slice(&((size << 16) | opcode as u32).to_le_bytes());
        self.tx_buf.extend_from_slice(args);
    }

    fn send_msg_with_fd(
        &mut self,
        object: u32,
        opcode: u16,
        args: &[u8],
        fd: &OwnedFd,
    ) -> io::Result<()> {
        self.flush()?;
        let mut packet = Vec::with_capacity(8 + args.len());
        let size = (8 + args.len()) as u32;
        packet.extend_from_slice(&object.to_le_bytes());
        packet.extend_from_slice(&((size << 16) | opcode as u32).to_le_bytes());
        packet.extend_from_slice(args);

        let iov = [IoSlice::new(&packet)];
        let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        let fds = [fd.as_fd()];
        if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
            return Err(io::Error::other("ancillary buffer full"));
        }
        let sent = rustix::net::sendmsg(&self.fd, &iov, &mut control, SendFlags::NOSIGNAL)?;
        if sent != packet.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short wayland send",
            ));
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        while !self.tx_buf.is_empty() {
            let iov = [IoSlice::new(&self.tx_buf)];
            let mut control = SendAncillaryBuffer::new(&mut []);
            match rustix::net::sendmsg(&self.fd, &iov, &mut control, SendFlags::NOSIGNAL) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "wayland socket closed",
                    ));
                }
                Ok(n) => {
                    self.tx_buf.drain(..n);
                }
                Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {
                    let mut fds = [PollFd::new(&self.fd, PollFlags::OUT)];
                    let ts = rustix::time::Timespec {
                        tv_sec: 1,
                        tv_nsec: 0,
                    };
                    let _ = rustix::event::poll(&mut fds, Some(&ts));
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn read_more(&mut self) -> io::Result<bool> {
        let mut buf = [0u8; 4096];
        let mut iov = [IoSliceMut::new(&mut buf)];
        let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(8))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        match rustix::net::recvmsg(&self.fd, &mut iov, &mut control, RecvFlags::CMSG_CLOEXEC) {
            Ok(res) => {
                for msg in control.drain() {
                    if let RecvAncillaryMessage::ScmRights(fds) = msg {
                        for fd in fds {
                            self.rx_fds.push_back(fd);
                        }
                    }
                }
                if res.bytes == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "wayland disconnected",
                    ));
                }
                self.rx_buf.extend_from_slice(&buf[..res.bytes]);
                Ok(true)
            }
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn next_message(&mut self) -> Option<(u32, u16, Vec<u8>)> {
        if self.rx_buf.len() < 8 {
            return None;
        }
        let object = u32::from_le_bytes(self.rx_buf[0..4].try_into().ok()?);
        let word = u32::from_le_bytes(self.rx_buf[4..8].try_into().ok()?);
        let size = (word >> 16) as usize;
        let opcode = (word & 0xffff) as u16;
        if size < 8 || self.rx_buf.len() < size {
            return None;
        }
        let payload = self.rx_buf[8..size].to_vec();
        self.rx_buf.drain(..size);
        Some((object, opcode, payload))
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = (bytes.len() + 1) as u32;
    put_u32(out, len);
    out.extend_from_slice(bytes);
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn read_u32(b: &[u8], off: &mut usize) -> Option<u32> {
    if *off + 4 > b.len() {
        return None;
    }
    let v = u32::from_le_bytes(b[*off..*off + 4].try_into().ok()?);
    *off += 4;
    Some(v)
}

fn read_i32(b: &[u8], off: &mut usize) -> Option<i32> {
    read_u32(b, off).map(|v| v as i32)
}

fn read_str(b: &[u8], off: &mut usize) -> Option<String> {
    let len = read_u32(b, off)? as usize;
    if len == 0 || *off + len > b.len() {
        return None;
    }
    let slice = &b[*off..*off + len - 1];
    let padded = (len + 3) & !3;
    *off = (*off + padded).min(b.len());
    Some(String::from_utf8_lossy(slice).to_string())
}

fn wayland_socket_path() -> Option<PathBuf> {
    let runtime = paths::runtime_dir();
    if let Ok(display) = std::env::var("WAYLAND_DISPLAY")
        && !display.is_empty()
    {
        let p = PathBuf::from(&display);
        let full = if p.is_absolute() {
            p
        } else {
            runtime.join(display)
        };
        if full.exists() {
            return Some(full);
        }
    }
    // Fallback when WAYLAND_DISPLAY wasn't exported into the service env.
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&runtime)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            name.starts_with("wayland-")
                && !name.ends_with(".lock")
                && std::fs::symlink_metadata(p)
                    .map(|m| m.file_type().is_socket())
                    .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

fn make_transparent_pixel_fd() -> io::Result<OwnedFd> {
    let fd = rustix::fs::memfd_create("omatouchbar-kbd", MemfdFlags::CLOEXEC)?;
    rustix::fs::ftruncate(&fd, 4)?;
    // Zero-initialized 4 bytes = 1x1 fully transparent ARGB8888 pixel.
    let ptr = unsafe {
        rustix::mm::mmap(
            std::ptr::null_mut(),
            4,
            ProtFlags::READ | ProtFlags::WRITE,
            MapFlags::SHARED,
            &fd,
            0,
        )?
    };
    unsafe {
        std::ptr::write_bytes(ptr.cast::<u8>(), 0, 4);
        let _ = rustix::mm::munmap(ptr.cast::<c_void>(), 4);
    }
    Ok(fd)
}

// ------------------------------------------------------ Public grab manager

pub struct KeyboardGrab {
    active: bool,
    stop_fd: Option<Arc<OwnedFd>>,
    thread: Option<JoinHandle<()>>,
}

impl KeyboardGrab {
    pub fn new() -> Self {
        Self {
            active: false,
            stop_fd: None,
            thread: None,
        }
    }

    pub fn set_active(&mut self, want: bool, bus: &Bus) {
        if want == self.active {
            return;
        }
        if want {
            self.start(bus.clone());
        } else {
            self.stop();
        }
    }

    fn start(&mut self, bus: Bus) {
        self.stop();
        let Ok(stop_fd) = rustix::event::eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)
        else {
            return;
        };
        let stop_fd = Arc::new(stop_fd);
        let thread_stop = stop_fd.clone();
        self.active = true;
        self.stop_fd = Some(stop_fd);
        self.thread = thread::Builder::new()
            .name("wayland-kbd".into())
            .spawn(move || {
                if let Err(e) = run_grab(bus, thread_stop, || {
                    log::debug!("kbd: exclusive keyboard focus active")
                }) {
                    log::warn!("kbd grab ended: {e}");
                }
            })
            .ok();
    }

    pub fn stop(&mut self) {
        self.active = false;
        if let Some(fd) = self.stop_fd.take() {
            let _ = rustix::io::write(&*fd, &1u64.to_ne_bytes());
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for KeyboardGrab {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_grab(bus: Bus, stop_fd: Arc<OwnedFd>, on_focus: impl Fn()) -> io::Result<()> {
    let mut wl = WaylandConn::connect()?;
    let mut xkb = Xkb::new().ok_or_else(|| io::Error::other("xkb_context_new failed"))?;

    // Request registry and sync globals.
    let mut a = Vec::new();
    put_u32(&mut a, ID_REGISTRY);
    wl.push_msg(ID_DISPLAY, 1, &a);
    a.clear();
    put_u32(&mut a, ID_SYNC_GLOBALS);
    wl.push_msg(ID_DISPLAY, 0, &a);
    wl.flush()?;

    // The registry's announcement order is compositor-defined. Client object
    // IDs, however, must be allocated consecutively on the wire.
    let bindings = [
        ("wl_compositor", ID_COMPOSITOR, 4),
        ("wl_shm", ID_SHM, 1),
        ("wl_seat", ID_SEAT, 7),
        ("zwlr_layer_shell_v1", ID_LAYER_SHELL, 4),
    ];
    let mut globals = [None; 4];
    let mut globals_done = false;
    let mut surface_created = false;
    let mut keyboard_bound = false;
    let mut buffer_ready = false;

    let mut repeat_rate: i32 = 25;
    let mut repeat_delay: i32 = 400;
    let mut repeat_key: Option<(u32, Instant)> = None;

    loop {
        let timeout_ms = match repeat_key {
            Some((_, at)) => at
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(250) as i64,
            None => 250,
        };
        let ts = rustix::time::Timespec {
            tv_sec: timeout_ms / 1000,
            tv_nsec: (timeout_ms % 1000) * 1_000_000,
        };
        let mut pfds = [
            PollFd::new(&stop_fd, PollFlags::IN),
            PollFd::new(&wl.fd, PollFlags::IN),
        ];
        match rustix::event::poll(&mut pfds, Some(&ts)) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
        if !pfds[0].revents().is_empty() {
            return Ok(());
        }
        if !pfds[1].revents().is_empty() {
            // Dispatch wl_display.error before a following EOF can mask it.
            wl.read_more()?;
        }

        while let Some((obj, op, payload)) = wl.next_message() {
            match (obj, op) {
                (ID_DISPLAY, 0) => {
                    // wl_display.error
                    let mut off = 0;
                    let oid = read_u32(&payload, &mut off).unwrap_or(0);
                    let code = read_u32(&payload, &mut off).unwrap_or(0);
                    let msg = read_str(&payload, &mut off).unwrap_or_default();
                    return Err(io::Error::other(format!(
                        "wayland error obj={oid} code={code}: {msg}"
                    )));
                }
                (ID_REGISTRY, 0) if !globals_done => {
                    // wl_registry.global(name, interface, version)
                    let mut off = 0;
                    let Some(name) = read_u32(&payload, &mut off) else {
                        continue;
                    };
                    let Some(iface) = read_str(&payload, &mut off) else {
                        continue;
                    };
                    let Some(ver) = read_u32(&payload, &mut off) else {
                        continue;
                    };
                    if let Some(index) = bindings
                        .iter()
                        .position(|(interface, _, _)| *interface == iface)
                    {
                        globals[index].get_or_insert((name, ver));
                    }
                }
                (ID_SYNC_GLOBALS, 0) if !globals_done => {
                    globals_done = true;
                    for ((interface, id, max_version), global) in bindings.into_iter().zip(globals)
                    {
                        let (name, version) = global.ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::Unsupported,
                                format!("missing Wayland global {interface}"),
                            )
                        })?;
                        let mut args = Vec::new();
                        put_u32(&mut args, name);
                        put_str(&mut args, interface);
                        put_u32(&mut args, version.min(max_version));
                        put_u32(&mut args, id);
                        wl.push_msg(ID_REGISTRY, 0, &args);
                    }
                    // Create invisible 1x1 transparent overlay layer surface with exclusive keyboard.
                    let mut b = Vec::new();
                    put_u32(&mut b, ID_SURFACE);
                    wl.push_msg(ID_COMPOSITOR, 0, &b);

                    // Empty input region so pointer clicks pass right through.
                    b.clear();
                    put_u32(&mut b, ID_REGION);
                    wl.push_msg(ID_COMPOSITOR, 1, &b);
                    b.clear();
                    put_u32(&mut b, ID_REGION);
                    wl.push_msg(ID_SURFACE, 5, &b);
                    wl.push_msg(ID_REGION, 0, &[]);

                    // zwlr_layer_shell_v1.get_layer_surface
                    b.clear();
                    put_u32(&mut b, ID_LAYER_SURFACE);
                    put_u32(&mut b, ID_SURFACE);
                    put_u32(&mut b, 0); // output = null (focused output)
                    put_u32(&mut b, LAYER_OVERLAY);
                    put_str(&mut b, "omatouchbar-password");
                    wl.push_msg(ID_LAYER_SHELL, 0, &b);

                    // Configure layer surface: 1x1, top-left, ignore exclusive zone, EXCLUSIVE keyboard.
                    b.clear();
                    put_u32(&mut b, 1);
                    put_u32(&mut b, 1);
                    wl.push_msg(ID_LAYER_SURFACE, 0, &b);

                    b.clear();
                    put_u32(&mut b, ANCHOR_TOP_LEFT);
                    wl.push_msg(ID_LAYER_SURFACE, 1, &b);

                    b.clear();
                    put_i32(&mut b, -1);
                    wl.push_msg(ID_LAYER_SURFACE, 2, &b);

                    b.clear();
                    put_u32(&mut b, KEYBOARD_EXCLUSIVE);
                    wl.push_msg(ID_LAYER_SURFACE, 4, &b);

                    // Initial null commit required before configure.
                    wl.push_msg(ID_SURFACE, 6, &[]);

                    // Create 1x1 transparent ARGB8888 buffer.
                    let shm_fd = make_transparent_pixel_fd()?;
                    b.clear();
                    put_u32(&mut b, ID_SHM_POOL);
                    put_i32(&mut b, 4);
                    wl.send_msg_with_fd(ID_SHM, 0, &b, &shm_fd)?;

                    b.clear();
                    put_u32(&mut b, ID_BUFFER);
                    put_i32(&mut b, 0);
                    put_i32(&mut b, 1);
                    put_i32(&mut b, 1);
                    put_i32(&mut b, 4);
                    put_u32(&mut b, SHM_FORMAT_ARGB8888);
                    wl.push_msg(ID_SHM_POOL, 0, &b);
                    wl.push_msg(ID_SHM_POOL, 1, &[]);
                    wl.flush()?;

                    surface_created = true;
                    buffer_ready = true;
                    log::debug!("kbd: overlay layer surface created");
                }
                (ID_SEAT, 0) => {
                    // wl_seat.capabilities
                    let mut off = 0;
                    let caps = read_u32(&payload, &mut off).unwrap_or(0);
                    if (caps & 2) != 0 && !keyboard_bound {
                        let mut b = Vec::new();
                        put_u32(&mut b, ID_KEYBOARD);
                        wl.push_msg(ID_SEAT, 1, &b);
                        wl.flush()?;
                        keyboard_bound = true;
                    }
                }
                (ID_LAYER_SURFACE, 0) => {
                    // zwlr_layer_surface_v1.configure(serial, width, height)
                    let mut off = 0;
                    if let Some(serial) = read_u32(&payload, &mut off)
                        && surface_created
                        && buffer_ready
                    {
                        let mut b = Vec::new();
                        put_u32(&mut b, serial);
                        wl.push_msg(ID_LAYER_SURFACE, 6, &b);

                        b.clear();
                        put_u32(&mut b, ID_BUFFER);
                        put_i32(&mut b, 0);
                        put_i32(&mut b, 0);
                        wl.push_msg(ID_SURFACE, 1, &b);

                        b.clear();
                        put_i32(&mut b, 0);
                        put_i32(&mut b, 0);
                        put_i32(&mut b, 1);
                        put_i32(&mut b, 1);
                        wl.push_msg(ID_SURFACE, 2, &b);

                        wl.push_msg(ID_SURFACE, 6, &[]);
                        wl.flush()?;
                    }
                }
                (ID_LAYER_SURFACE, 1) => {
                    // zwlr_layer_surface_v1.closed
                    return Ok(());
                }
                (ID_KEYBOARD, 0) => {
                    // wl_keyboard.keymap(format, size) + fd in SCM_RIGHTS
                    let mut off = 0;
                    let format = read_u32(&payload, &mut off).unwrap_or(0);
                    let size = read_u32(&payload, &mut off).unwrap_or(0) as usize;
                    if let Some(fd) = wl.rx_fds.pop_front()
                        && format == XKB_KEYMAP_FORMAT_TEXT_V1
                    {
                        xkb.load_keymap(fd, size);
                    }
                }
                (ID_KEYBOARD, 1) => {
                    // wl_keyboard.enter(serial, surface, keys).
                    let mut off = 4;
                    if read_u32(&payload, &mut off) == Some(ID_SURFACE) {
                        on_focus();
                    }
                }
                (ID_KEYBOARD, 2) => {
                    // wl_keyboard.leave
                    repeat_key = None;
                }
                (ID_KEYBOARD, 3) => {
                    // wl_keyboard.key(serial, time, key, state)
                    let mut off = 0;
                    let _serial = read_u32(&payload, &mut off);
                    let _time = read_u32(&payload, &mut off);
                    let Some(key) = read_u32(&payload, &mut off) else {
                        continue;
                    };
                    let Some(state) = read_u32(&payload, &mut off) else {
                        continue;
                    };
                    if state == 1 {
                        if let Some(input) = xkb.translate(key) {
                            let repeatable =
                                matches!(input, KeyInput::Text(_) | KeyInput::Backspace)
                                    && xkb.key_repeats(key)
                                    && repeat_rate > 0;
                            bus.send(Event::KeyInput(input));
                            if repeatable {
                                repeat_key = Some((
                                    key,
                                    Instant::now()
                                        + Duration::from_millis(repeat_delay.max(50) as u64),
                                ));
                            } else {
                                repeat_key = None;
                            }
                        }
                    } else if state == 0 && repeat_key.is_some_and(|(k, _)| k == key) {
                        repeat_key = None;
                    }
                }
                (ID_KEYBOARD, 4) => {
                    // wl_keyboard.modifiers(serial, depressed, latched, locked, group)
                    let mut off = 0;
                    let _serial = read_u32(&payload, &mut off);
                    let depressed = read_u32(&payload, &mut off).unwrap_or(0);
                    let latched = read_u32(&payload, &mut off).unwrap_or(0);
                    let locked = read_u32(&payload, &mut off).unwrap_or(0);
                    let group = read_u32(&payload, &mut off).unwrap_or(0);
                    xkb.update_modifiers(depressed, latched, locked, group);
                }
                (ID_KEYBOARD, 5) => {
                    // wl_keyboard.repeat_info(rate, delay)
                    let mut off = 0;
                    repeat_rate = read_i32(&payload, &mut off).unwrap_or(25);
                    repeat_delay = read_i32(&payload, &mut off).unwrap_or(400);
                }
                _ => {}
            }
        }

        // Software key repeat using the compositor's repeat_info.
        if let Some((key, at)) = repeat_key
            && Instant::now() >= at
        {
            if let Some(input) = xkb.translate(key) {
                bus.send(Event::KeyInput(input));
            }
            let period = Duration::from_millis((1000 / repeat_rate.max(1)) as u64);
            repeat_key = Some((key, Instant::now() + period));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a running Wayland compositor with layer-shell keyboard focus"]
    fn compositor_focuses_the_text_entry_layer() {
        let stop = Arc::new(rustix::event::eventfd(0, EventfdFlags::CLOEXEC).unwrap());
        let thread_stop = stop.clone();
        let (bus, _rx) = Bus::new();
        let (ready, focused) = std::sync::mpsc::channel();
        let grab = thread::spawn(move || {
            run_grab(bus, thread_stop, || {
                let _ = ready.send(());
            })
        });
        let acquired = focused.recv_timeout(Duration::from_secs(2));
        rustix::io::write(&*stop, &1u64.to_ne_bytes()).unwrap();
        grab.join().unwrap().unwrap();
        acquired.expect("the compositor did not give the Touch Bar keyboard focus");
    }
}
