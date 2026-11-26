//! Unix seqpacket transport and sealed memfd frame memory.

use std::{
    ffi::c_void,
    fs,
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::Path,
    ptr::NonNull,
    time::Instant,
};

use rustix::{
    event::{PollFd, PollFlags},
    fs::{MemfdFlags, SealFlags},
    mm::{MapFlags, ProtFlags},
    net::{
        AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
        SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketFlags,
        SocketType,
    },
};

use crate::wire::MAX_PACKET;

pub const SOCKET_PATH: &str = "/run/t1bridge/touchbar.sock";

/// A connected, non-blocking `SOCK_SEQPACKET` socket to the root service.
pub struct SeqPacket {
    fd: OwnedFd,
}

impl SeqPacket {
    /// Connects to the hardware service and verifies that both the socket
    /// file and the connected peer belong to root.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.file_type().is_socket() || meta.uid() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "endpoint is not a root-owned socket",
            ));
        }
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::connect(&fd, &SocketAddrUnix::new(path)?)?;
        let cred = rustix::net::sockopt::socket_peercred(&fd)?;
        if cred.uid.as_raw() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "hardware peer is not root",
            ));
        }
        set_nonblocking(&fd)?;
        Ok(Self { fd })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Sends one packet, optionally carrying exactly one descriptor.
    /// Returns `WouldBlock` when the socket buffer is full.
    pub fn send(&self, packet: &[u8], descriptor: Option<BorrowedFd<'_>>) -> io::Result<()> {
        let iov = [IoSlice::new(packet)];
        let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        let fds;
        if let Some(fd) = descriptor {
            fds = [fd];
            if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
                return Err(io::Error::other("ancillary buffer too small"));
            }
        }
        let sent = rustix::net::sendmsg(&self.fd, &iov, &mut control, SendFlags::NOSIGNAL)?;
        if sent != packet.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short seqpacket send",
            ));
        }
        Ok(())
    }

    /// Sends, waiting for socket space until `deadline`.
    pub fn send_until(
        &self,
        packet: &[u8],
        descriptor: Option<BorrowedFd<'_>>,
        deadline: Instant,
    ) -> io::Result<()> {
        loop {
            match self.send(packet, descriptor) {
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::Interrupted =>
                {
                    wait(self.fd(), PollFlags::OUT, deadline)?;
                }
                other => return other,
            }
        }
    }

    /// Receives one packet into `buffer` (which must hold `MAX_PACKET` bytes).
    /// Returns `WouldBlock` when nothing is queued and `UnexpectedEof` on close.
    pub fn receive(&self, buffer: &mut [u8]) -> io::Result<usize> {
        debug_assert!(buffer.len() >= MAX_PACKET);
        let mut iov = [IoSliceMut::new(buffer)];
        let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(4))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let result = rustix::net::recvmsg(
            &self.fd,
            &mut iov,
            &mut control,
            RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
        )?;
        // The service never sends descriptors; close any that arrive.
        let mut unexpected = false;
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(fds) = message {
                for fd in fds {
                    drop(fd);
                    unexpected = true;
                }
            }
        }
        if result.bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "hardware connection closed",
            ));
        }
        if result
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
            || result.bytes > MAX_PACKET
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated packet",
            ));
        }
        if unexpected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected descriptor",
            ));
        }
        Ok(result.bytes)
    }

    pub fn receive_until(&self, buffer: &mut [u8], deadline: Instant) -> io::Result<usize> {
        loop {
            match self.receive(buffer) {
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::Interrupted =>
                {
                    wait(self.fd(), PollFlags::IN, deadline)?;
                }
                other => return other,
            }
        }
    }
}

fn set_nonblocking(fd: &OwnedFd) -> io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}

fn wait(fd: BorrowedFd<'_>, events: PollFlags, deadline: Instant) -> io::Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "hardware service timed out",
            ));
        }
        let timeout = rustix::time::Timespec {
            tv_sec: remaining.as_secs() as _,
            tv_nsec: remaining.subsec_nanos() as _,
        };
        let mut fds = [PollFd::from_borrowed_fd(fd, events)];
        match rustix::event::poll(&mut fds, Some(&timeout)) {
            Ok(0) => continue,
            Ok(_) => {
                let revents = fds[0].revents();
                if revents.intersects(events) {
                    return Ok(());
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "hardware connection closed",
                ));
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// One sealed, shared, writable frame buffer.
///
/// The memfd is sealed against shrinking, growing and further seal changes,
/// as the service requires. The client keeps a writable mapping; the root
/// side maps it read-only and snapshots submitted pixels.
pub struct FrameMemory {
    fd: OwnedFd,
    ptr: NonNull<u8>,
    len: usize,
}

// The mapping is owned exclusively by this value.
unsafe impl Send for FrameMemory {}

impl FrameMemory {
    pub fn new(len: usize) -> io::Result<Self> {
        if len == 0 || len > 32 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size out of range",
            ));
        }
        let fd = rustix::fs::memfd_create(
            "omatouchbar-frame",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )?;
        rustix::fs::ftruncate(&fd, len as u64)?;
        // SAFETY: fresh shared mapping of a memfd of exactly `len` bytes,
        // unmapped exactly once in Drop.
        let raw = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                &fd,
                0,
            )?
        };
        let ptr =
            NonNull::new(raw.cast::<u8>()).ok_or_else(|| io::Error::other("mmap returned null"))?;
        let memory = Self { fd, ptr, len };
        rustix::fs::fcntl_add_seals(
            &memory.fd,
            SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
        )?;
        Ok(memory)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the mapping is `len` bytes, alive for `self`, and uniquely
        // borrowed. The caller must not write while the service owns the
        // buffer (between SubmitFrame and FrameReleased).
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// Views the buffer as native-endian 32-bit pixels.
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        // SAFETY: mmap returns page-aligned memory and len is a multiple of 4.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr().cast::<u32>(), self.len / 4) }
    }
}

impl Drop for FrameMemory {
    fn drop(&mut self) {
        // SAFETY: ptr/len came from one successful mmap.
        unsafe {
            let _ = rustix::mm::munmap(self.ptr.as_ptr().cast::<c_void>(), self.len);
        }
    }
}
