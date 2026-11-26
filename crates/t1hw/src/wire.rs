//! Packet encoding for the T1Bridge Touch Bar hardware IPC v1.
//!
//! Implemented from the public interface description in T1Bridge's
//! `docs/interfaces.md` ("Touch Bar hardware IPC v1"). Every packet is one
//! seqpacket message with a 16-byte little-endian header:
//!
//! | offset | field                                  |
//! |--------|----------------------------------------|
//! | 0      | ASCII `T1HW`                           |
//! | 4      | `u16` protocol major (1)               |
//! | 6      | `u16` message type                     |
//! | 8      | `u32` payload length                   |
//! | 12     | `u32` request id (0 for events)        |

use std::fmt;

pub const MAGIC: [u8; 4] = *b"T1HW";
pub const MAJOR: u16 = 1;
pub const HEADER_LEN: usize = 16;
/// Control packets are limited to 64 KiB.
pub const MAX_PACKET: usize = 64 * 1024;
/// Highest minor version this client understands.
pub const CLIENT_MINOR: u16 = 1;
pub const MAX_DAMAGE: usize = 64;
pub const MAX_CONTACTS: usize = 10;
pub const PIXEL_FORMAT_XRGB8888: u32 = 1;

pub mod msg {
    pub const HELLO: u16 = 0x0001;
    pub const REGISTER_BUFFER: u16 = 0x0002;
    pub const SUBMIT_FRAME: u16 = 0x0003;
    pub const TAP_KEYS: u16 = 0x0004;
    pub const SET_DISPLAY_BRIGHTNESS: u16 = 0x0005;
    pub const SET_KEYBOARD_BACKLIGHT: u16 = 0x0006;
    pub const CANCEL_TOUCH_ID: u16 = 0x0007;
    pub const STEP_DISPLAY_BRIGHTNESS: u16 = 0x0008;
    pub const STEP_KEYBOARD_BACKLIGHT: u16 = 0x0009;
    pub const HELLO_ACK: u16 = 0x8001;
    pub const ACK: u16 = 0x8002;
    pub const ERROR: u16 = 0x8003;
    pub const FRAME_RELEASED: u16 = 0x9001;
    pub const INPUT_FRAME: u16 = 0x9002;
}

/// Feature bits a client may require in `Hello`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Features(pub u64);

impl Features {
    pub const SEALED_MEMFD_FRAMES: Features = Features(0x01);
    pub const INPUT_FRAMES: Features = Features(0x02);
    pub const TYPED_KEY_TAPS: Features = Features(0x04);
    pub const DISPLAY_BRIGHTNESS: Features = Features(0x08);
    pub const KEYBOARD_BACKLIGHT: Features = Features(0x10);
    pub const TOUCH_ID_CANCELLATION: Features = Features(0x20);
    /// The set the service always accepts (`0x27`).
    pub const BASE: Features = Features(0x27);
    pub const ALL: Features = Features(0x3f);

    pub const fn contains(self, other: Features) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn union(self, other: Features) -> Features {
        Features(self.0 | other.0)
    }
    pub const fn without(self, other: Features) -> Features {
        Features(self.0 & !other.0)
    }
}

/// Linux input key codes the service accepts in `TapKeys`.
pub mod key {
    pub const ESC: u16 = 1;
    pub const F1: u16 = 59;
    pub const F10: u16 = 68;
    pub const F11: u16 = 87;
    pub const F12: u16 = 88;

    /// Key code for F1..=F12 (`n` is 1-based).
    pub const fn function(n: u8) -> u16 {
        match n {
            1..=10 => F1 + (n as u16 - 1),
            11 => F11,
            _ => F12,
        }
    }

    pub const fn is_allowed(code: u16) -> bool {
        matches!(code, ESC | F1..=F10 | F11 | F12)
    }
}

/// Typed service error codes. Errors carry no text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    UnsupportedMessage,
    UnsupportedFeature,
    ResourceLimit,
    InvalidBuffer,
    UnknownBuffer,
    BufferBusy,
    ActionDenied,
    DeviceUnavailable,
    Io,
    Internal,
    Other(u32),
}

impl ErrorCode {
    pub fn from_wire(value: u32) -> Self {
        match value {
            1 => Self::UnsupportedMessage,
            2 => Self::UnsupportedFeature,
            3 => Self::ResourceLimit,
            4 => Self::InvalidBuffer,
            5 => Self::UnknownBuffer,
            6 => Self::BufferBusy,
            7 => Self::ActionDenied,
            8 => Self::DeviceUnavailable,
            9 => Self::Io,
            10 => Self::Internal,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::UnsupportedMessage => "unsupported message",
            Self::UnsupportedFeature => "unsupported feature",
            Self::ResourceLimit => "resource limit",
            Self::InvalidBuffer => "invalid buffer",
            Self::UnknownBuffer => "unknown buffer",
            Self::BufferBusy => "buffer busy",
            Self::ActionDenied => "action denied",
            Self::DeviceUnavailable => "device unavailable",
            Self::Io => "I/O failure",
            Self::Internal => "internal failure",
            Self::Other(code) => return write!(f, "error {code}"),
        };
        f.write_str(text)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    pub id: u8,
    pub tip: bool,
    pub in_range: bool,
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputFrame {
    pub monotonic_ns: u64,
    pub fn_pressed: bool,
    pub contacts: Vec<Contact>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelloAck {
    pub minor: u16,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub max_buffers: u32,
}

/// Messages the service sends to the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceMessage {
    HelloAck(HelloAck),
    Ack,
    Error(ErrorCode),
    FrameReleased {
        buffer_id: u32,
        frame_id: u64,
    },
    Input(InputFrame),
    /// A message type this client does not know; ignored.
    Unknown(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub request_id: u32,
    pub message: ServiceMessage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Short,
    Magic,
    Major(u16),
    Length,
    Payload(u16),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short => f.write_str("packet shorter than header"),
            Self::Magic => f.write_str("bad packet magic"),
            Self::Major(major) => write!(f, "unsupported protocol major {major}"),
            Self::Length => f.write_str("payload length mismatch"),
            Self::Payload(kind) => write!(f, "malformed payload for message {kind:#06x}"),
        }
    }
}

impl std::error::Error for WireError {}

fn header(kind: u16, payload_len: usize, request_id: u32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(HEADER_LEN + payload_len);
    packet.extend_from_slice(&MAGIC);
    packet.extend_from_slice(&MAJOR.to_le_bytes());
    packet.extend_from_slice(&kind.to_le_bytes());
    packet.extend_from_slice(&(payload_len as u32).to_le_bytes());
    packet.extend_from_slice(&request_id.to_le_bytes());
    packet
}

pub fn encode_hello(request_id: u32, minor: u16, features: Features) -> Vec<u8> {
    let mut p = header(msg::HELLO, 12, request_id);
    p.extend_from_slice(&minor.to_le_bytes());
    p.extend_from_slice(&0u16.to_le_bytes());
    p.extend_from_slice(&features.0.to_le_bytes());
    p
}

pub fn encode_register_buffer(
    request_id: u32,
    buffer_id: u32,
    stride: u32,
    length: u64,
) -> Vec<u8> {
    let mut p = header(msg::REGISTER_BUFFER, 16, request_id);
    p.extend_from_slice(&buffer_id.to_le_bytes());
    p.extend_from_slice(&stride.to_le_bytes());
    p.extend_from_slice(&length.to_le_bytes());
    p
}

pub fn encode_submit_frame(
    request_id: u32,
    buffer_id: u32,
    frame_id: u64,
    damage: &[Rect],
) -> Vec<u8> {
    let damage = &damage[..damage.len().min(MAX_DAMAGE)];
    let mut p = header(msg::SUBMIT_FRAME, 16 + damage.len() * 16, request_id);
    p.extend_from_slice(&buffer_id.to_le_bytes());
    p.extend_from_slice(&frame_id.to_le_bytes());
    p.extend_from_slice(&(damage.len() as u32).to_le_bytes());
    for rect in damage {
        for value in [rect.x, rect.y, rect.width, rect.height] {
            p.extend_from_slice(&value.to_le_bytes());
        }
    }
    p
}

/// `keys` must hold one to four distinct allowed key codes.
pub fn encode_tap_keys(request_id: u32, keys: &[u16]) -> Vec<u8> {
    let count = keys.len().clamp(1, 4);
    let mut p = header(msg::TAP_KEYS, 12, request_id);
    p.push(count as u8);
    p.extend_from_slice(&[0, 0, 0]);
    for slot in 0..4 {
        let value = keys
            .get(slot)
            .copied()
            .filter(|_| slot < count)
            .unwrap_or(0);
        p.extend_from_slice(&value.to_le_bytes());
    }
    p
}

pub fn encode_percentage(request_id: u32, kind: u16, percent: u8) -> Vec<u8> {
    let mut p = header(kind, 1, request_id);
    p.push(percent.min(100));
    p
}

pub fn encode_step(request_id: u32, kind: u16, up: bool) -> Vec<u8> {
    let mut p = header(kind, 1, request_id);
    p.push(u8::from(up));
    p
}

pub fn encode_cancel_touch_id(request_id: u32) -> Vec<u8> {
    header(msg::CANCEL_TOUCH_ID, 0, request_id)
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(bytes)
}

pub fn decode_service(packet: &[u8]) -> Result<Envelope, WireError> {
    if packet.len() < HEADER_LEN {
        return Err(WireError::Short);
    }
    if packet[..4] != MAGIC {
        return Err(WireError::Magic);
    }
    let major = u16_at(packet, 4);
    if major != MAJOR {
        return Err(WireError::Major(major));
    }
    let kind = u16_at(packet, 6);
    let length = u32_at(packet, 8) as usize;
    let request_id = u32_at(packet, 12);
    let payload = &packet[HEADER_LEN..];
    if payload.len() != length {
        return Err(WireError::Length);
    }
    let bad = || WireError::Payload(kind);
    let message = match kind {
        msg::HELLO_ACK => {
            if payload.len() != 20 {
                return Err(bad());
            }
            ServiceMessage::HelloAck(HelloAck {
                minor: u16_at(payload, 0),
                width: u32_at(payload, 4),
                height: u32_at(payload, 8),
                pixel_format: u32_at(payload, 12),
                max_buffers: u32_at(payload, 16),
            })
        }
        msg::ACK => ServiceMessage::Ack,
        msg::ERROR => {
            if payload.len() != 4 {
                return Err(bad());
            }
            ServiceMessage::Error(ErrorCode::from_wire(u32_at(payload, 0)))
        }
        msg::FRAME_RELEASED => {
            if payload.len() != 12 {
                return Err(bad());
            }
            ServiceMessage::FrameReleased {
                buffer_id: u32_at(payload, 0),
                frame_id: u64_at(payload, 4),
            }
        }
        msg::INPUT_FRAME => {
            if payload.len() < 12 {
                return Err(bad());
            }
            let count = payload[9] as usize;
            if count > MAX_CONTACTS || payload.len() != 12 + count * 12 {
                return Err(bad());
            }
            let contacts = (0..count)
                .map(|i| {
                    let c = &payload[12 + i * 12..24 + i * 12];
                    Contact {
                        id: c[0],
                        tip: c[1] != 0,
                        in_range: c[2] != 0,
                        x: u32_at(c, 4),
                        y: u32_at(c, 8),
                    }
                })
                .collect();
            ServiceMessage::Input(InputFrame {
                monotonic_ns: u64_at(payload, 0),
                fn_pressed: payload[8] != 0,
                contacts,
            })
        }
        other => ServiceMessage::Unknown(other),
    };
    Ok(Envelope {
        request_id,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_layout_matches_spec() {
        let p = encode_hello(7, 1, Features::BASE);
        assert_eq!(&p[..4], b"T1HW");
        assert_eq!(u16_at(&p, 4), 1);
        assert_eq!(u16_at(&p, 6), msg::HELLO);
        assert_eq!(u32_at(&p, 8), 12);
        assert_eq!(u32_at(&p, 12), 7);
        assert_eq!(p.len(), 28);
        assert_eq!(u16_at(&p, 16), 1);
        assert_eq!(u64_at(&p, 20), 0x27);
    }

    #[test]
    fn tap_keys_pads_unused_slots() {
        let p = encode_tap_keys(3, &[key::ESC]);
        assert_eq!(p.len(), HEADER_LEN + 12);
        assert_eq!(p[16], 1);
        assert_eq!(u16_at(&p, 20), 1);
        assert_eq!(u16_at(&p, 22), 0);
    }

    #[test]
    fn function_key_codes() {
        assert_eq!(key::function(1), 59);
        assert_eq!(key::function(10), 68);
        assert_eq!(key::function(11), 87);
        assert_eq!(key::function(12), 88);
        assert!(key::is_allowed(key::ESC));
        assert!(!key::is_allowed(2));
    }

    #[test]
    fn decodes_input_frame() {
        let mut p = header(msg::INPUT_FRAME, 24, 0);
        p.extend_from_slice(&42u64.to_le_bytes());
        p.push(1);
        p.push(1);
        p.extend_from_slice(&[0, 0]);
        p.extend_from_slice(&[3, 1, 1, 0]);
        p.extend_from_slice(&100u32.to_le_bytes());
        p.extend_from_slice(&30u32.to_le_bytes());
        let env = decode_service(&p).unwrap();
        let ServiceMessage::Input(frame) = env.message else {
            panic!()
        };
        assert!(frame.fn_pressed);
        assert_eq!(
            frame.contacts[0],
            Contact {
                id: 3,
                tip: true,
                in_range: true,
                x: 100,
                y: 30
            }
        );
    }

    #[test]
    fn rejects_length_mismatch() {
        let mut p = header(msg::ACK, 4, 1);
        p.extend_from_slice(&[0, 0]);
        assert_eq!(decode_service(&p), Err(WireError::Length));
    }
}
