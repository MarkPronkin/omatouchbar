//! High-level renderer connection: negotiation, buffers, frames and actions.

use std::{
    collections::HashMap,
    io,
    os::fd::BorrowedFd,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use crate::{
    transport::{FrameMemory, SeqPacket},
    wire::{
        self, Envelope, ErrorCode, Features, HelloAck, InputFrame, MAX_PACKET, Rect, ServiceMessage,
    },
};

const SETUP_TIMEOUT: Duration = Duration::from_secs(3);
const SEND_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub enum ConnectError {
    Io(io::Error),
    Rejected(ErrorCode),
    Protocol(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Rejected(code) => write!(f, "service rejected connection: {code}"),
            Self::Protocol(text) => write!(f, "protocol error: {text}"),
        }
    }
}

impl std::error::Error for ConnectError {}

impl From<io::Error> for ConnectError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BufferState {
    Free,
    /// SubmitFrame sent; waiting for Ack.
    Submitted,
    /// Owned by the service until FrameReleased.
    Owned,
}

struct Buffer {
    id: u32,
    memory: FrameMemory,
    state: BufferState,
    submitted: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
enum Pending {
    Submit(usize),
    Action(&'static str),
}

/// Events delivered to the renderer after setup.
#[derive(Clone, Debug)]
pub enum Event {
    Input(InputFrame),
    /// A buffer became free again; `latency` is submit-to-release time.
    BufferFree {
        latency: Option<Duration>,
    },
    /// A typed action was rejected.
    ActionFailed {
        action: &'static str,
        code: ErrorCode,
    },
}

pub struct Client {
    socket: SeqPacket,
    next_request: u32,
    pub info: HelloAck,
    pub features: Features,
    buffers: Vec<Buffer>,
    frame_id: u64,
    pending: HashMap<u32, Pending>,
    scratch: Vec<u8>,
    max_in_flight: usize,
}

fn hello(path: &Path, features: Features) -> Result<(SeqPacket, HelloAck), ConnectError> {
    let socket = SeqPacket::connect(path)?;
    let deadline = Instant::now() + SETUP_TIMEOUT;
    socket.send_until(
        &wire::encode_hello(1, wire::CLIENT_MINOR, features),
        None,
        deadline,
    )?;
    let mut buffer = vec![0u8; MAX_PACKET];
    loop {
        let length = socket.receive_until(&mut buffer, deadline)?;
        let envelope = wire::decode_service(&buffer[..length])
            .map_err(|e| ConnectError::Protocol(e.to_string()))?;
        match envelope.message {
            ServiceMessage::HelloAck(ack) if envelope.request_id == 1 => {
                if ack.pixel_format != wire::PIXEL_FORMAT_XRGB8888
                    || ack.width == 0
                    || ack.height == 0
                {
                    return Err(ConnectError::Protocol(format!(
                        "unsupported display {ack:?}"
                    )));
                }
                return Ok((socket, ack));
            }
            ServiceMessage::Error(code) => return Err(ConnectError::Rejected(code)),
            _ => continue,
        }
    }
}

impl Client {
    /// Connects to the service, negotiating as many of `wanted`'s optional
    /// brightness features as the service accepts.
    pub fn connect(path: &Path, wanted: Features) -> Result<Self, ConnectError> {
        let wanted = wanted.union(Features::BASE);
        let (socket, info, features) = match hello(path, wanted) {
            Ok((socket, info)) => (socket, info, wanted),
            Err(ConnectError::Rejected(ErrorCode::UnsupportedFeature)) => {
                // Probe each optional bit on its own short-lived connection,
                // then require the accepted set on the final connection.
                let mut accepted = Features::BASE;
                for bit in [Features::DISPLAY_BRIGHTNESS, Features::KEYBOARD_BACKLIGHT] {
                    if !wanted.contains(bit) {
                        continue;
                    }
                    if hello(path, Features::BASE.union(bit)).is_ok() {
                        accepted = accepted.union(bit);
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                let mut last = None;
                let mut result = None;
                for _ in 0..20 {
                    match hello(path, accepted) {
                        Ok((socket, info)) => {
                            result = Some((socket, info));
                            break;
                        }
                        Err(e) => {
                            last = Some(e);
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
                let (socket, info) = result.ok_or_else(|| last.unwrap())?;
                (socket, info, accepted)
            }
            Err(e) => return Err(e),
        };

        let mut client = Client {
            socket,
            next_request: 2,
            info,
            features,
            buffers: Vec::new(),
            frame_id: 0,
            pending: HashMap::new(),
            scratch: vec![0u8; MAX_PACKET],
            max_in_flight: 2,
        };
        client.register_buffers()?;
        Ok(client)
    }

    pub fn width(&self) -> u32 {
        self.info.width
    }

    pub fn height(&self) -> u32 {
        self.info.height
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.socket.fd()
    }

    fn request_id(&mut self) -> u32 {
        loop {
            let id = self.next_request;
            self.next_request = self.next_request.wrapping_add(1).max(1);
            if id != 0 && !self.pending.contains_key(&id) {
                return id;
            }
        }
    }

    fn register_buffers(&mut self) -> Result<(), ConnectError> {
        let stride = self.info.width * 4;
        let length = u64::from(stride) * u64::from(self.info.height);
        let count = self.info.max_buffers.clamp(1, 3);
        for index in 0..count {
            let memory = FrameMemory::new(length as usize)?;
            let buffer_id = index + 1;
            let request = self.request_id();
            let deadline = Instant::now() + SETUP_TIMEOUT;
            self.socket.send_until(
                &wire::encode_register_buffer(request, buffer_id, stride, length),
                Some(memory.fd()),
                deadline,
            )?;
            loop {
                let len = self.socket.receive_until(&mut self.scratch, deadline)?;
                let envelope = wire::decode_service(&self.scratch[..len])
                    .map_err(|e| ConnectError::Protocol(e.to_string()))?;
                if envelope.request_id != request {
                    continue;
                }
                match envelope.message {
                    ServiceMessage::Ack => break,
                    ServiceMessage::Error(code) => return Err(ConnectError::Rejected(code)),
                    _ => continue,
                }
            }
            self.buffers.push(Buffer {
                id: buffer_id,
                memory,
                state: BufferState::Free,
                submitted: None,
            });
        }
        Ok(())
    }

    /// Index of a buffer the client may write, if any, keeping at most
    /// `max_in_flight` frames queued at the service.
    pub fn free_buffer(&self) -> Option<usize> {
        let in_flight = self
            .buffers
            .iter()
            .filter(|b| b.state != BufferState::Free)
            .count();
        if in_flight >= self.max_in_flight {
            return None;
        }
        self.buffers
            .iter()
            .position(|b| b.state == BufferState::Free)
    }

    /// Frames allowed at the service at once (1–3). Fewer means lower
    /// latency; more means higher throughput.
    pub fn set_max_in_flight(&mut self, frames: usize) {
        self.max_in_flight = frames.clamp(1, 3);
    }

    pub fn frames_in_flight(&self) -> usize {
        self.buffers
            .iter()
            .filter(|b| b.state != BufferState::Free)
            .count()
    }

    pub fn buffer_pixels(&mut self, index: usize) -> &mut [u32] {
        self.buffers[index].memory.pixels_mut()
    }

    /// Submits a written buffer. An empty damage list means the whole frame.
    pub fn submit(&mut self, index: usize, damage: &[Rect]) -> io::Result<()> {
        let request = self.request_id();
        self.frame_id += 1;
        let buffer = &mut self.buffers[index];
        let packet = wire::encode_submit_frame(request, buffer.id, self.frame_id, damage);
        self.socket
            .send_until(&packet, None, Instant::now() + SEND_TIMEOUT)?;
        buffer.state = BufferState::Submitted;
        buffer.submitted = Some(Instant::now());
        self.pending.insert(request, Pending::Submit(index));
        Ok(())
    }

    fn action(&mut self, name: &'static str, build: impl FnOnce(u32) -> Vec<u8>) -> io::Result<()> {
        let request = self.request_id();
        let packet = build(request);
        self.socket
            .send_until(&packet, None, Instant::now() + SEND_TIMEOUT)?;
        self.pending.insert(request, Pending::Action(name));
        Ok(())
    }

    /// Presses and releases up to four allowed keys (Esc, F1–F12).
    pub fn tap_keys(&mut self, keys: &[u16]) -> io::Result<()> {
        if keys.is_empty() || keys.len() > 4 || !keys.iter().all(|k| wire::key::is_allowed(*k)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "key not allowed",
            ));
        }
        let keys = keys.to_vec();
        self.action("tap-keys", move |id| wire::encode_tap_keys(id, &keys))
    }

    pub fn has(&self, feature: Features) -> bool {
        self.features.contains(feature)
    }

    pub fn set_display_brightness(&mut self, percent: u8) -> io::Result<()> {
        self.action("display-brightness", |id| {
            wire::encode_percentage(id, wire::msg::SET_DISPLAY_BRIGHTNESS, percent)
        })
    }

    pub fn set_keyboard_backlight(&mut self, percent: u8) -> io::Result<()> {
        self.action("keyboard-backlight", |id| {
            wire::encode_percentage(id, wire::msg::SET_KEYBOARD_BACKLIGHT, percent)
        })
    }

    pub fn step_display_brightness(&mut self, up: bool) -> io::Result<()> {
        self.action("display-step", |id| {
            wire::encode_step(id, wire::msg::STEP_DISPLAY_BRIGHTNESS, up)
        })
    }

    pub fn step_keyboard_backlight(&mut self, up: bool) -> io::Result<()> {
        self.action("keyboard-step", |id| {
            wire::encode_step(id, wire::msg::STEP_KEYBOARD_BACKLIGHT, up)
        })
    }

    pub fn cancel_touch_id(&mut self) -> io::Result<()> {
        self.action("cancel-touch-id", wire::encode_cancel_touch_id)
    }

    /// Drains every queued packet. Returns `UnexpectedEof` when the service
    /// closed the connection.
    pub fn read_events(&mut self, out: &mut Vec<Event>) -> io::Result<()> {
        loop {
            let length = match self.socket.receive(&mut self.scratch) {
                Ok(length) => length,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            let envelope = match wire::decode_service(&self.scratch[..length]) {
                Ok(envelope) => envelope,
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            };
            self.handle(envelope, out);
        }
    }

    fn handle(&mut self, envelope: Envelope, out: &mut Vec<Event>) {
        match envelope.message {
            ServiceMessage::Input(frame) => out.push(Event::Input(frame)),
            ServiceMessage::FrameReleased { buffer_id, .. } => {
                if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                    buffer.state = BufferState::Free;
                    let latency = buffer.submitted.take().map(|t| t.elapsed());
                    out.push(Event::BufferFree { latency });
                }
            }
            ServiceMessage::Ack => match self.pending.remove(&envelope.request_id) {
                Some(Pending::Submit(index)) => {
                    if self.buffers[index].state == BufferState::Submitted {
                        self.buffers[index].state = BufferState::Owned;
                    }
                }
                Some(Pending::Action(_)) | None => {}
            },
            ServiceMessage::Error(code) => match self.pending.remove(&envelope.request_id) {
                Some(Pending::Submit(index)) => {
                    self.buffers[index].state = BufferState::Free;
                    self.buffers[index].submitted = None;
                    out.push(Event::BufferFree { latency: None });
                }
                Some(Pending::Action(action)) => out.push(Event::ActionFailed { action, code }),
                None => {}
            },
            ServiceMessage::HelloAck(_) | ServiceMessage::Unknown(_) => {}
        }
    }
}
