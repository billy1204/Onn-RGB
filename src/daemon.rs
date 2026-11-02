//! `onn-rgb daemon`: tray icon + Fn-combo hotkeys.
//!
//! Threads, all blocking (no timers, no polling):
//! - main: `poll()` on the keyboard's evdev nodes, the uinput devices (LED
//!   feedback), a kernel uevent socket (hotplug/resume), a control pipe and a
//!   signal pipe. The only timeouts are one-shot: the Fn window and a short
//!   delay after a hotplug event.
//! - hid worker: owns all HID I/O (so the input loop never blocks on the
//!   keyboard) and caches the current settings.
//! - tray (ksni, current-thread tokio): idle on D-Bus.

use crate::actions::{self, Action};
use crate::config::{self, Config};
use crate::evdev::{self, Device, Event, Uinput, EV_KEY, EV_LED, EV_SYN};
use crate::fnkeys::{Engine, Origin};
use crate::{hotkeys, Keyboard, Mode, Settings};
use std::fs::File;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const PASSTHROUGH_NAME: &str = "onn-rgb passthrough";

fn log(msg: impl AsRef<str>) {
    eprintln!("onn-rgb: {}", msg.as_ref());
}

/// Commands for the HID worker.
pub enum Cmd {
    Act(Action),
    SetMode(Mode),
    SetBrightness(u8),
    SetSpeed(u8),
    Refresh,
}

/// Wakes the input loop from other threads (tray).
#[derive(Clone)]
pub struct Ctl {
    block_combo: Arc<AtomicBool>,
    notices: Arc<AtomicBool>,
    wake_fd: RawFd,
}

impl Ctl {
    pub fn set_notices(&self, on: bool) {
        self.notices.store(on, Ordering::SeqCst);
    }

    pub fn set_block_combo_keys(&self, on: bool) {
        self.block_combo.store(on, Ordering::SeqCst);
        unsafe { libc::write(self.wake_fd, b"c".as_ptr().cast(), 1) };
    }
}

/// What the tray shows.
#[derive(Default)]
pub struct Shown {
    pub settings: Option<Settings>,
    pub error: Option<String>,
    pub status: Option<String>,
}

type Notify = Arc<dyn Fn(Box<dyn FnOnce(&mut Shown) + Send>) + Send + Sync>;

fn pipe() -> (RawFd, RawFd) {
    let mut fds = [0; 2];
    unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
    (fds[0], fds[1])
}

fn drain(fd: RawFd) {
    let mut b = [0u8; 64];
    while unsafe { libc::read(fd, b.as_mut_ptr().cast(), b.len()) } > 0 {}
}

static SIG_FD: AtomicI32 = AtomicI32::new(-1);
extern "C" fn on_signal(_: libc::c_int) {
    let fd = SIG_FD.load(Ordering::Relaxed);
    unsafe { libc::write(fd, b"s".as_ptr().cast(), 1) };
}

/// Single-instance lock in $XDG_RUNTIME_DIR, held until the process exits.
fn lock() -> Result<File, String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let p = dir.join("onn-rgb.lock");
    let f = File::create(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(format!("another onn-rgb daemon is already running ({} is locked)", p.display()));
    }
    Ok(f)
}

fn desktop_notify(msg: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "onn-rgb", "-i", "input-keyboard", "onn Keyboard Lighting", msg])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn kind_of(cmd: &Cmd) -> Option<crate::notify::Kind> {
    use crate::notify::Kind;
    Some(match cmd {
        Cmd::Refresh => return None,
        Cmd::SetMode(_) => Kind::Mode,
        Cmd::SetBrightness(_) => Kind::Brightness,
        Cmd::SetSpeed(_) => Kind::Speed,
        Cmd::Act(a) => match a {
            Action::BrightnessUp | Action::BrightnessDown => Kind::Brightness,
            Action::SpeedUp | Action::SpeedDown => Kind::Speed,
            _ => Kind::Mode,
        },
    })
}

fn worker(rx: Receiver<Cmd>, notify: Notify, notices: Arc<AtomicBool>) {
    let _ = &notices;
    #[cfg(feature = "tray")]
    let mut notifier = crate::notify::Notifier::new();
    let mut cached: Option<Settings> = None;
    for cmd in rx {
        let kind = kind_of(&cmd);
        let r = Keyboard::open().and_then(|mut kb| {
            let cur = match (&cmd, cached) {
                (Cmd::Refresh, _) | (_, None) => kb.settings()?,
                (_, Some(s)) => s,
            };
            match cmd {
                Cmd::Refresh => Ok(cur),
                Cmd::Act(a) => actions::apply_from(&mut kb, cur, a),
                Cmd::SetMode(m) => {
                    if m == cur.mode {
                        return Ok(cur);
                    }
                    if cur.mode != Mode::Off {
                        actions::remember_mode(cur.mode);
                    }
                    kb.set_mode(m).map(|_| Settings { mode: m, ..cur })
                }
                Cmd::SetBrightness(b) if b == cur.brightness => Ok(cur),
                Cmd::SetBrightness(b) => kb.set_brightness(b).map(|_| Settings { brightness: b, ..cur }),
                Cmd::SetSpeed(v) if v == cur.speed => Ok(cur),
                Cmd::SetSpeed(v) => kb.set_speed(v).map(|_| Settings { speed: v, ..cur }),
            }
        });
        match r {
            Ok(s) => {
                if cached != Some(s) {
                    log(format!("lighting: {}, brightness {}, speed {}", s.mode, s.brightness, s.speed));
                }
                cached = Some(s);
                #[cfg(feature = "tray")]
                if let Some(k) = kind {
                    if notices.load(Ordering::SeqCst) {
                        notifier.show(&crate::notify::text(k, s));
                    }
                }
                let _ = kind;
                notify(Box::new(move |t| {
                    t.settings = Some(s);
                    t.error = None;
                }));
            }
            Err(e) => {
                log(format!("lighting: {e}"));
                cached = None;
                let e = e.to_string();
                notify(Box::new(move |t| t.error = Some(e)));
            }
        }
    }
}

struct Node {
    path: PathBuf,
    origin: Origin,
    dev: Device,
    ui: Option<Uinput>,
}

struct Input {
    cfg: Config,
    engine: Engine,
    nodes: Vec<Node>,
    block_combo: Arc<AtomicBool>,
    tx: Sender<Cmd>,
    notify: Notify,
    uinput_warned: bool,
    perm_warned: bool,
    rescan_at: Option<Instant>,
    rescan_tries: u32,
    last_status: String,
}

impl Input {
    fn want_grab(&self, o: Origin) -> bool {
        self.cfg.grab && (o == Origin::FnNode || self.block_combo.load(Ordering::SeqCst))
    }

    /// Grab a node and mirror it through uinput. Returns false to retry later.
    fn grab(&mut self, i: usize) -> bool {
        let n = &mut self.nodes[i];
        if n.dev.is_grabbed() {
            return true;
        }
        if n.dev.any_key_down() {
            return false; // avoid stuck keys: wait until everything is released
        }
        match Uinput::mirror(&n.dev, PASSTHROUGH_NAME) {
            Ok(ui) => match n.dev.grab(true) {
                Ok(()) => {
                    n.ui = Some(ui);
                    log(format!("grabbed {} (re-emitting via \"{PASSTHROUGH_NAME}\")", n.path.display()));
                }
                Err(e) => log(format!("can't grab {}: {e}; running ungrabbed", n.path.display())),
            },
            Err(e) => {
                if !self.uinput_warned {
                    self.uinput_warned = true;
                    let msg = format!(
                        "uinput not available ({e}); running ungrabbed, so Fn also changes the volume. \
                         Install udev/72-onn-uinput.rules (see README)."
                    );
                    log(&msg);
                    desktop_notify(&msg);
                }
            }
        }
        true
    }

    fn ungrab(&mut self, i: usize) {
        let n = &mut self.nodes[i];
        if n.dev.is_grabbed() {
            let _ = n.dev.grab(false);
            log(format!("released {}", n.path.display()));
        }
        n.ui = None;
    }

    /// (Re)open the keyboard's event nodes. Returns true if a retry is needed.
    fn scan(&mut self) -> bool {
        let mut retry = false;
        let found = hotkeys::event_nodes();
        self.nodes.retain(|n| found.iter().any(|(p, _)| *p == n.path));
        for (path, name) in found {
            if self.nodes.iter().any(|n| n.path == path) {
                continue;
            }
            let dev = match Device::open(&path) {
                Ok(d) => d,
                Err(e) => {
                    retry = true;
                    if !self.perm_warned && e.kind() == std::io::ErrorKind::PermissionDenied {
                        self.perm_warned = true;
                        log(format!("can't read {} ({name}): {e}. Install udev/71-onn-keyboard-input.rules (see README)", path.display()));
                    }
                    continue;
                }
            };
            let origin = if dev.event_types().contains(&EV_LED) {
                Origin::Main
            } else if dev.has_key(self.cfg.fn_cfg.key) {
                Origin::FnNode
            } else {
                continue; // mouse interface etc.
            };
            log(format!("listening on {} ({name}, {:?})", path.display(), origin));
            self.nodes.push(Node { path, origin, dev, ui: None });
            let i = self.nodes.len() - 1;
            if self.want_grab(origin) && !self.grab(i) {
                retry = true;
            }
        }
        self.update_status();
        retry
    }

    fn update_status(&mut self) {
        let fn_node = self.nodes.iter().find(|n| n.origin == Origin::FnNode);
        let s = match fn_node {
            None if self.perm_warned => "Fn combos: no permission (see README)".to_string(),
            None => "Fn combos: keyboard not found".to_string(),
            Some(n) if n.dev.is_grabbed() => "Fn combos active".to_string(),
            Some(_) if self.cfg.grab => "Fn combos active (ungrabbed: Fn also changes volume)".to_string(),
            Some(_) => "Fn combos active (ungrabbed)".to_string(),
        };
        if s != self.last_status {
            log(&s);
            self.last_status = s.clone();
            (self.notify)(Box::new(move |t| t.status = Some(s)));
        }
    }

    fn apply_block_combo(&mut self) -> bool {
        let mut retry = false;
        for i in 0..self.nodes.len() {
            if self.nodes[i].origin != Origin::Main {
                continue;
            }
            if self.want_grab(Origin::Main) {
                retry |= !self.grab(i);
            } else {
                self.ungrab(i);
            }
        }
        retry
    }

    /// Handle readable node `i`. Returns false if the node went away.
    fn read_node(&mut self, i: usize, now: Instant) -> bool {
        let mut evs = Vec::new();
        let r = evdev::read_events(self.nodes[i].dev.fd(), &mut evs);
        let origin = self.nodes[i].origin;
        let mut out: Vec<Event> = Vec::with_capacity(evs.len());
        for ev in evs {
            if ev.ty == EV_KEY {
                let o = self.engine.key(origin, ev.code, ev.value, now);
                if let Some(a) = o.action {
                    let _ = self.tx.send(Cmd::Act(a));
                }
                if !o.forward {
                    continue;
                }
            }
            out.push(ev);
        }
        if let Some(ui) = &self.nodes[i].ui {
            if out.iter().any(|e| e.ty != EV_SYN) {
                if let Err(e) = evdev::write_events(ui.fd(), &out) {
                    log(format!("uinput write failed ({e}); releasing grab"));
                    self.ungrab(i);
                }
            }
        }
        match r {
            Ok(()) => true,
            Err(e) => {
                log(format!("{} closed ({e})", self.nodes[i].path.display()));
                false
            }
        }
    }

    /// LED changes (Caps/Num Lock) sent to the virtual device -> real keyboard.
    fn read_uinput(&mut self, i: usize) {
        let n = &self.nodes[i];
        let Some(ui) = &n.ui else { return };
        let mut evs = Vec::new();
        let _ = evdev::read_events(ui.fd(), &mut evs);
        let mut w: Vec<Event> = evs.into_iter().filter(|e| e.ty == EV_LED).collect();
        if !w.is_empty() {
            w.push(Event { ty: EV_SYN, code: 0, value: 0 });
            let _ = evdev::write_events(n.dev.fd(), &w);
        }
    }
}

pub fn run() -> ! {
    // Release builds abort on panic; make debug builds abort too. Either way
    // the kernel closes our fds, which drops grabs and virtual devices.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        hook(info);
        std::process::abort();
    }));

    let _lock = lock().unwrap_or_else(|e| {
        log(e);
        std::process::exit(1)
    });
    let (cfg, warnings) = config::load();
    for w in warnings {
        log(format!("config: {w}"));
    }
    if let Some(p) = config::path() {
        log(format!("config: {}", p.display()));
    }
    log(format!(
        "fn: {}, window {} ms, grab {}, block-combo-keys {}, keys: {}",
        hotkeys::key_name(cfg.fn_cfg.key),
        cfg.fn_cfg.window.as_millis(),
        cfg.grab,
        cfg.block_combo_keys,
        config::describe_fn(&cfg.fn_cfg)
    ));

    let (sig_r, sig_w) = pipe();
    SIG_FD.store(sig_w, Ordering::SeqCst);
    for s in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        unsafe { libc::signal(s, on_signal as *const () as libc::sighandler_t) };
    }
    let (ctl_r, ctl_w) = pipe();
    let block_combo = Arc::new(AtomicBool::new(cfg.block_combo_keys));
    let notices = Arc::new(AtomicBool::new(cfg.notices));
    let ctl = Ctl { block_combo: block_combo.clone(), notices: notices.clone(), wake_fd: ctl_w };

    let (tx, rx) = mpsc::channel::<Cmd>();
    #[allow(unused_mut)]
    let mut notify: Notify = Arc::new(|_| {});

    #[cfg(feature = "tray")]
    if cfg.tray_enabled {
        use ksni::blocking::TrayMethods;
        let tray = crate::tray::OnnTray {
            settings: None,
            error: None,
            status: String::new(),
            block_combo_keys: cfg.block_combo_keys,
            notices: cfg.notices,
            tx: tx.clone(),
            ctl: ctl.clone(),
        };
        match tray.spawn() {
            Ok(handle) => {
                log("tray: registered StatusNotifierItem");
                notify = Arc::new(move |f| {
                    handle.update(move |t| {
                        let mut s = Shown { settings: t.settings, error: t.error.clone(), status: Some(t.status.clone()) };
                        f(&mut s);
                        t.settings = s.settings;
                        t.error = s.error;
                        t.status = s.status.unwrap_or_default();
                    });
                });
            }
            Err(e) => log(format!(
                "tray: not available ({e}). Your desktop needs a StatusNotifierItem host \
                 (on GNOME: the AppIndicator extension). Hotkeys still work."
            )),
        }
    }
    let _ = &ctl;

    {
        let notify = notify.clone();
        thread::Builder::new().name("hid".into()).spawn(move || worker(rx, notify, notices)).unwrap();
    }
    let _ = tx.send(Cmd::Refresh);

    let uevents = evdev::uevent_socket().map_err(|e| log(format!("hotplug: no uevent socket ({e})"))).ok();
    let mut inp = Input {
        engine: Engine::new(cfg.fn_cfg.clone(), cfg.bindings.clone()),
        cfg,
        nodes: Vec::new(),
        block_combo,
        tx,
        notify,
        uinput_warned: false,
        perm_warned: false,
        rescan_at: None,
        rescan_tries: 0,
        last_status: String::new(),
    };
    if !inp.cfg.hotkeys_enabled {
        log("hotkeys disabled in config");
    } else if inp.scan() {
        inp.rescan_at = Some(Instant::now() + Duration::from_millis(500));
    }

    loop {
        let now = Instant::now();
        let deadline = [inp.engine.deadline(), inp.rescan_at].into_iter().flatten().min();
        let timeout = deadline.map_or(-1, |d| d.saturating_duration_since(now).as_millis() as i32 + 1);

        let mut fds = vec![
            libc::pollfd { fd: sig_r, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: ctl_r, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: uevents.as_ref().map_or(-1, |f| f.as_raw_fd()), events: libc::POLLIN, revents: 0 },
        ];
        // Fn node first so its events are handled before the arrow's.
        let mut order: Vec<usize> = (0..inp.nodes.len()).collect();
        order.sort_by_key(|i| inp.nodes[*i].origin != Origin::FnNode);
        for &i in &order {
            fds.push(libc::pollfd { fd: inp.nodes[i].dev.fd(), events: libc::POLLIN, revents: 0 });
            fds.push(libc::pollfd { fd: inp.nodes[i].ui.as_ref().map_or(-1, |u| u.fd()), events: libc::POLLIN, revents: 0 });
        }
        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
        if r < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            log(format!("poll: {}", std::io::Error::last_os_error()));
            break;
        }
        let now = Instant::now();
        if fds[0].revents != 0 {
            log("exiting; releasing keyboard");
            break;
        }
        if fds[1].revents != 0 {
            drain(ctl_r);
            if inp.apply_block_combo() {
                inp.rescan_at = Some(now + Duration::from_millis(200));
            }
        }
        if fds[2].revents != 0 && evdev::uevent_relevant(fds[2].fd) {
            // Give udev a moment to create nodes and apply permissions.
            inp.rescan_at = Some(now + Duration::from_millis(300));
            inp.rescan_tries = 0;
            inp.perm_warned = false;
        }
        let mut gone = Vec::new();
        for (k, &i) in order.iter().enumerate() {
            let (dev_ev, ui_ev) = (fds[3 + 2 * k].revents, fds[4 + 2 * k].revents);
            if dev_ev & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 && dev_ev & libc::POLLIN == 0 {
                gone.push(i);
                continue;
            }
            if dev_ev & libc::POLLIN != 0 && !inp.read_node(i, now) {
                gone.push(i);
            }
            if ui_ev & libc::POLLIN != 0 {
                inp.read_uinput(i);
            }
        }
        if !gone.is_empty() {
            gone.sort_unstable();
            for i in gone.into_iter().rev() {
                inp.nodes.remove(i);
            }
            inp.update_status();
            inp.rescan_at = Some(now + Duration::from_millis(300));
        }
        inp.engine.expire(now);
        if inp.rescan_at.is_some_and(|d| now >= d) {
            inp.rescan_at = None;
            if inp.cfg.hotkeys_enabled {
                let mut retry = inp.scan();
                for i in 0..inp.nodes.len() {
                    if inp.want_grab(inp.nodes[i].origin) {
                        retry |= !inp.grab(i);
                    }
                }
                inp.update_status();
                if retry && inp.rescan_tries < 10 {
                    inp.rescan_tries += 1;
                    inp.rescan_at = Some(now + Duration::from_millis(500));
                }
            }
        }
    }
    // Dropping the nodes ungrabs them and destroys the virtual devices.
    drop(inp);
    std::process::exit(0)
}
