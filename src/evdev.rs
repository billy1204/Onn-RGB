//! Thin evdev / uinput / netlink helpers (Linux ioctls via libc).
//!
//! Safety net for grabbing: an `EVIOCGRAB` belongs to the open file, so the
//! kernel drops it the moment the fd closes, including when the process
//! exits, aborts on panic or is killed. A grabbed keyboard can therefore
//! never stay dead after onn-rgb stops.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;
pub const EV_MSC: u16 = 0x04;
pub const EV_LED: u16 = 0x11;
pub const KEY_A: u16 = 30;

const EVIOCGRAB: u64 = 0x4004_4590;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const UI_DEV_SETUP: u64 = 0x405c_5503;
const UI_ABS_SETUP: u64 = 0x401c_5504;
const UI_SET_EVBIT: u64 = 0x4004_5564;

fn ioc_read(nr: u64, len: u64) -> u64 {
    (2 << 30) | (len << 16) | (0x45 << 8) | nr
}
fn ui_set_bit(ev: u16) -> Option<u64> {
    Some(match ev {
        EV_KEY => 0x4004_5565,
        EV_REL => 0x4004_5566,
        EV_ABS => 0x4004_5567,
        EV_MSC => 0x4004_5568,
        EV_LED => 0x4004_5569,
        _ => return None,
    })
}
fn max_code(ev: u16) -> usize {
    match ev {
        EV_KEY => 0x2ff,
        EV_REL => 0x0f,
        EV_ABS => 0x3f,
        EV_MSC => 0x07,
        EV_LED => 0x0f,
        _ => 0x1f,
    }
}

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

/// One `struct input_event` (x86_64 / 64-bit time layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub ty: u16,
    pub code: u16,
    pub value: i32,
}

impl Event {
    pub fn decode(b: &[u8]) -> Event {
        Event {
            ty: u16::from_ne_bytes([b[16], b[17]]),
            code: u16::from_ne_bytes([b[18], b[19]]),
            value: i32::from_ne_bytes([b[20], b[21], b[22], b[23]]),
        }
    }
    fn encode(&self) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[16..18].copy_from_slice(&self.ty.to_ne_bytes());
        b[18..20].copy_from_slice(&self.code.to_ne_bytes());
        b[20..24].copy_from_slice(&self.value.to_ne_bytes());
        b
    }
}

/// Read all currently queued events (fd must be non-blocking).
pub fn read_events(fd: RawFd, out: &mut Vec<Event>) -> io::Result<()> {
    let mut buf = [0u8; 24 * 64];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::WouldBlock { Ok(()) } else { Err(e) };
        }
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        for c in buf[..n as usize].chunks_exact(24) {
            out.push(Event::decode(c));
        }
    }
}

pub fn write_events(fd: RawFd, evs: &[Event]) -> io::Result<()> {
    let bytes: Vec<u8> = evs.iter().flat_map(|e| e.encode()).collect();
    let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// An open evdev node.
pub struct Device {
    pub file: File,
    grabbed: bool,
}

impl Device {
    pub fn open(path: &Path) -> io::Result<Device> {
        let open = |w: bool| {
            OpenOptions::new().read(true).write(w).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(path)
        };
        // Write access lets us forward LED changes (Caps Lock) when grabbed.
        let file = open(true).or_else(|_| open(false))?;
        Ok(Device { file, grabbed: false })
    }
    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    fn bits(&self, ev: u16, nbits: usize) -> Vec<u16> {
        let mut b = vec![0u8; nbits / 8 + 1];
        let r = unsafe { libc::ioctl(self.fd(), ioc_read(0x20 + ev as u64, b.len() as u64) as _, b.as_mut_ptr()) };
        if r < 0 {
            return Vec::new();
        }
        (0..nbits as u16).filter(|i| b[*i as usize / 8] >> (i % 8) & 1 == 1).collect()
    }
    pub fn event_types(&self) -> Vec<u16> {
        self.bits(0, 0x1f)
    }
    pub fn codes(&self, ev: u16) -> Vec<u16> {
        self.bits(ev, max_code(ev) + 1)
    }
    pub fn has_key(&self, code: u16) -> bool {
        self.codes(EV_KEY).contains(&code)
    }
    /// True if any key on this device is currently held down.
    pub fn any_key_down(&self) -> bool {
        let mut b = [0u8; 0x300 / 8];
        let r = unsafe { libc::ioctl(self.fd(), ioc_read(0x18, b.len() as u64) as _, b.as_mut_ptr()) };
        r >= 0 && b.iter().any(|x| *x != 0)
    }
    fn absinfo(&self, code: u16) -> [i32; 6] {
        let mut a = [0i32; 6];
        unsafe { libc::ioctl(self.fd(), ioc_read(0x40 + code as u64, 24) as _, a.as_mut_ptr()) };
        a
    }
    pub fn grab(&mut self, on: bool) -> io::Result<()> {
        cvt(unsafe { libc::ioctl(self.fd(), EVIOCGRAB as _, on as libc::c_int) })?;
        self.grabbed = on;
        Ok(())
    }
    pub fn is_grabbed(&self) -> bool {
        self.grabbed
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if self.grabbed {
            let _ = self.grab(false);
        }
    }
}

/// A uinput virtual device that mirrors a real device's capabilities.
pub struct Uinput {
    pub file: File,
}

impl Uinput {
    pub fn mirror(src: &Device, name: &str) -> io::Result<Uinput> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open("/dev/uinput")?;
        let fd = file.as_raw_fd();
        // No EV_REP: the real device's repeats are forwarded as-is.
        for ev in src.event_types() {
            let Some(set) = ui_set_bit(ev) else { continue };
            cvt(unsafe { libc::ioctl(fd, UI_SET_EVBIT as _, ev as libc::c_int) })?;
            for code in src.codes(ev) {
                cvt(unsafe { libc::ioctl(fd, set as _, code as libc::c_int) })?;
                if ev == EV_ABS {
                    let mut s = [0u8; 28];
                    s[0..2].copy_from_slice(&code.to_ne_bytes());
                    for (i, v) in src.absinfo(code).iter().enumerate() {
                        s[4 + i * 4..8 + i * 4].copy_from_slice(&v.to_ne_bytes());
                    }
                    cvt(unsafe { libc::ioctl(fd, UI_ABS_SETUP as _, s.as_ptr()) })?;
                }
            }
        }
        let mut setup = [0u8; 92];
        setup[0..2].copy_from_slice(&0x06u16.to_ne_bytes()); // BUS_VIRTUAL; vendor/product 0
        let n = name.as_bytes();
        setup[8..8 + n.len().min(79)].copy_from_slice(&n[..n.len().min(79)]);
        cvt(unsafe { libc::ioctl(fd, UI_DEV_SETUP as _, setup.as_ptr()) })?;
        cvt(unsafe { libc::ioctl(fd, UI_DEV_CREATE as _) })?;
        Ok(Uinput { file })
    }
    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl Drop for Uinput {
    fn drop(&mut self) {
        unsafe { libc::ioctl(self.fd(), UI_DEV_DESTROY as _) };
    }
}

/// Kernel uevent socket (hotplug notifications; no polling).
pub fn uevent_socket() -> io::Result<File> {
    unsafe {
        let fd = cvt(libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            libc::NETLINK_KOBJECT_UEVENT,
        ))?;
        let mut addr: libc::sockaddr_nl = std::mem::zeroed();
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_groups = 1;
        if libc::bind(fd, (&addr as *const libc::sockaddr_nl).cast(), std::mem::size_of_val(&addr) as u32) < 0 {
            let e = io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(std::os::fd::FromRawFd::from_raw_fd(fd))
    }
}

/// Drain the uevent socket; true if any input/hidraw device came or went.
pub fn uevent_relevant(fd: RawFd) -> bool {
    let mut relevant = false;
    let mut buf = [0u8; 8192];
    loop {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n <= 0 {
            return relevant;
        }
        let msg = &buf[..n as usize];
        if msg.split(|b| *b == 0).any(|f| f == b"SUBSYSTEM=input" || f == b"SUBSYSTEM=hidraw") {
            relevant = true;
        }
    }
}
