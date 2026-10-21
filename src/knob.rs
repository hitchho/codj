//! A hardware knob (a rotary encoder that shows up as volume keys) for DJ mode: turning it
//! edits the selected control, pressing it resets the control.
//!
//! A background thread reads one evdev device and forwards only the three configured key codes
//! (up, down, press) to the UI loop. Every other event is dropped on the spot: nothing else is
//! logged, stored or forwarded, because the device may well be a keyboard.
//!
//! The device can be grabbed (EVIOCGRAB) while DJ mode is open so the system volume does not move
//! meanwhile. The grab belongs to the open file, so it ends when [`Knob`] is dropped (leaving DJ
//! mode) and, should codj die, when the kernel closes the file. A device that also types letters
//! is never grabbed: that would swallow the keyboard.

use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use evdev::raw_stream::RawDevice;
use evdev::{EventType, KeyCode};

use crate::parser::KnobSettings;

pub const KEY_VOLUMEUP: u16 = 115;
pub const KEY_VOLUMEDOWN: u16 = 114;
pub const KEY_MUTE: u16 = 113;

/// What the knob did
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnobAction {
    Up,
    Down,
    Press,
}

/// The configured key codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Codes {
    pub up: u16,
    pub down: u16,
    pub press: u16,
}

impl Codes {
    pub fn from_settings(s: &KnobSettings) -> Codes {
        Codes {
            up: s.up.unwrap_or(KEY_VOLUMEUP),
            down: s.down.unwrap_or(KEY_VOLUMEDOWN),
            press: s.press.unwrap_or(KEY_MUTE),
        }
    }

    /// An evdev key event as an action. `value`: 1 press, 2 auto-repeat, 0 release. Turning acts
    /// on press and on auto-repeat (a held volume key keeps turning); the press button acts once,
    /// on the press only; releases and unknown codes are nothing.
    pub fn map(&self, code: u16, value: i32) -> Option<KnobAction> {
        match (code, value) {
            (c, 1 | 2) if c == self.up => Some(KnobAction::Up),
            (c, 1 | 2) if c == self.down => Some(KnobAction::Down),
            (c, 1) if c == self.press => Some(KnobAction::Press),
            _ => None,
        }
    }
}

/// A message from the reader thread
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// the knob turned or was pressed, stamped when it was read
    Action(KnobAction, Instant),
    /// something the user should know once (no device, no permission, unplugged)
    Note(String),
}

/// The running reader. Dropping it stops the thread, which releases the grab.
pub struct Knob {
    rx: Receiver<Msg>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Knob {
    /// Starts reading when the config names a device; None when the knob is off
    pub fn start(settings: &KnobSettings) -> Option<Knob> {
        let needle = settings.device.as_deref().map(str::trim).filter(|d| !d.is_empty())?.to_string();
        let codes = Codes::from_settings(settings);
        let grab = settings.grab.unwrap_or(true);
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || run(&needle, codes, grab, &tx, &stop))
        };
        Some(Knob { rx, stop, thread: Some(thread) })
    }

    /// Everything that arrived since the last call
    pub fn drain(&self) -> Vec<Msg> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Knob {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// How often the thread looks at the stop flag while the device is quiet
const POLL_MS: i32 = 40;

fn run(needle: &str, codes: Codes, grab: bool, tx: &Sender<Msg>, stop: &AtomicBool) {
    let mut dev = match find_device(needle, &codes) {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(Msg::Note(format!("knob: {e}")));
            return;
        }
    };
    let mut grabbed = false;
    if grab {
        let types_letters = dev.supported_keys().is_some_and(|k| k.contains(KeyCode::KEY_A));
        if types_letters {
            let _ = tx.send(Msg::Note(
                "knob: that device types letters too, so it is not grabbed (system volume still moves)".into(),
            ));
        } else if let Err(e) = dev.grab() {
            let _ = tx.send(Msg::Note(format!("knob: could not grab the device ({e}); system volume still moves")));
        } else {
            grabbed = true;
        }
    }
    let fd = dev.as_raw_fd();
    while !stop.load(Ordering::SeqCst) {
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd for an open descriptor that `dev` keeps alive
        let n = unsafe { libc::poll(&mut p, 1, POLL_MS) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if n == 0 {
            continue;
        }
        if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            let _ = tx.send(Msg::Note("knob: the device went away".into()));
            break;
        }
        match dev.fetch_events() {
            Ok(events) => {
                for ev in events {
                    // everything but the configured codes is dropped here, unexamined
                    if ev.event_type() != EventType::KEY {
                        continue;
                    }
                    if let Some(action) = codes.map(ev.code(), ev.value()) {
                        if tx.send(Msg::Action(action, Instant::now())).is_err() {
                            return;
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => {
                let _ = tx.send(Msg::Note("knob: the device went away".into()));
                break;
            }
        }
    }
    if grabbed {
        let _ = dev.ungrab();
    }
}

/// The first input device whose name contains `needle` (case-insensitive) and that has one of
/// the configured keys: one product often exposes several nodes (keyboard, consumer control)
fn find_device(needle: &str, codes: &Codes) -> Result<RawDevice, String> {
    let mut paths: Vec<_> = std::fs::read_dir("/dev/input")
        .map_err(|e| format!("cannot read /dev/input ({e})"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")))
        .collect();
    paths.sort();
    let needle_lc = needle.to_lowercase();
    let mut denied = false;
    let mut named = None;
    for path in paths {
        let dev = match RawDevice::open(&path) {
            Ok(d) => d,
            Err(e) => {
                denied |= e.kind() == std::io::ErrorKind::PermissionDenied;
                continue;
            }
        };
        let name = dev.name().unwrap_or("").to_string();
        if !name.to_lowercase().contains(&needle_lc) {
            continue;
        }
        named = Some(name);
        let has = |c: u16| dev.supported_keys().is_some_and(|k| k.contains(KeyCode::new(c)));
        if has(codes.up) || has(codes.down) || has(codes.press) {
            return Ok(dev);
        }
    }
    Err(match (named, denied) {
        (Some(n), _) => format!("\"{n}\" has none of the configured keys (up {}, down {}, press {})", codes.up, codes.down, codes.press),
        (None, true) => format!("no readable input device matches \"{needle}\" (permission denied on /dev/input: are you in the input group?)"),
        (None, false) => format!("no input device matches \"{needle}\""),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Codes = Codes { up: KEY_VOLUMEUP, down: KEY_VOLUMEDOWN, press: KEY_MUTE };

    #[test]
    fn defaults_are_the_volume_keys() {
        assert_eq!(Codes::from_settings(&KnobSettings::default()), C);
        let custom = KnobSettings { up: Some(1), down: Some(2), press: Some(3), ..Default::default() };
        assert_eq!(Codes::from_settings(&custom), Codes { up: 1, down: 2, press: 3 });
    }

    #[test]
    fn turning_acts_on_press_and_repeat_not_on_release() {
        assert_eq!(C.map(115, 1), Some(KnobAction::Up));
        assert_eq!(C.map(115, 2), Some(KnobAction::Up));
        assert_eq!(C.map(115, 0), None);
        assert_eq!(C.map(114, 1), Some(KnobAction::Down));
        assert_eq!(C.map(114, 2), Some(KnobAction::Down));
        assert_eq!(C.map(114, 0), None);
    }

    #[test]
    fn press_acts_once() {
        assert_eq!(C.map(113, 1), Some(KnobAction::Press));
        assert_eq!(C.map(113, 2), None, "key repeat of the button is not another press");
        assert_eq!(C.map(113, 0), None);
    }

    #[test]
    fn other_keys_are_nothing() {
        // letters, enter, anything not configured: never an action, whatever the value
        for code in [1u16, 28, 30, 31, 57, 100, 112, 116, 164, 0x110] {
            for value in [0, 1, 2, 3, -1] {
                assert_eq!(C.map(code, value), None, "{code} {value}");
            }
        }
        // odd values of a configured code
        assert_eq!(C.map(115, 3), None);
        assert_eq!(C.map(115, -1), None);
    }

    #[test]
    fn remapped_codes_replace_the_defaults() {
        let c = Codes { up: 200, down: 201, press: 202 };
        assert_eq!(c.map(200, 1), Some(KnobAction::Up));
        assert_eq!(c.map(115, 1), None);
        assert_eq!(c.map(202, 1), Some(KnobAction::Press));
    }

    #[test]
    fn off_without_a_device_name() {
        assert!(Knob::start(&KnobSettings::default()).is_none());
        assert!(Knob::start(&KnobSettings { device: Some("  ".into()), ..Default::default() }).is_none());
    }

    #[test]
    fn a_missing_device_gives_one_note_and_ends() {
        let s = KnobSettings { device: Some("no such knob xyzzy 42".into()), ..Default::default() };
        let knob = Knob::start(&s).unwrap();
        let mut got = Vec::new();
        for _ in 0..100 {
            got.extend(knob.drain());
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(matches!(&got[0], Msg::Note(n) if n.starts_with("knob: ")), "{got:?}");
    }

    /// Needs a writable /dev/uinput; skipped (passes) without one. The virtual device only has
    /// F20..F23, so nothing it emits can reach a real application even if the grab failed.
    #[test]
    fn a_virtual_device_end_to_end() {
        use evdev::uinput::VirtualDevice;
        use evdev::{AttributeSet, InputEvent};
        if std::fs::OpenOptions::new().write(true).open("/dev/uinput").is_err() {
            eprintln!("skipped: /dev/uinput is not writable");
            return;
        }
        const NAME: &str = "codj-test-knob-7731";
        let mut keys = AttributeSet::<KeyCode>::new();
        for c in 190..=193u16 {
            keys.insert(KeyCode::new(c));
        }
        let Ok(mut vd) = VirtualDevice::builder().and_then(|b| b.name(NAME).with_keys(&keys)).and_then(|b| b.build()) else {
            eprintln!("skipped: cannot create a virtual device");
            return;
        };
        let codes = Codes { up: 191, down: 192, press: 193 };
        // wait for the node to show up
        let mut found = None;
        for _ in 0..100 {
            if let Ok(d) = find_device(NAME, &codes) {
                found = Some(d);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let Some(probe) = found else {
            eprintln!("skipped: the virtual device did not appear in /dev/input");
            return;
        };
        drop(probe);
        let settings =
            KnobSettings { device: Some(NAME.into()), up: Some(191), down: Some(192), press: Some(193), grab: Some(true) };
        let knob = Knob::start(&settings).unwrap();
        // the thread has the device grabbed once a second open cannot grab it
        let mut busy = false;
        for _ in 0..100 {
            if let Ok(mut other) = find_device(NAME, &codes) {
                if other.grab().is_err() {
                    busy = true;
                    break;
                }
                let _ = other.ungrab();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(busy, "the knob never grabbed the device");
        let ev = |code, value| InputEvent::new(EventType::KEY.0, code, value);
        vd.emit(&[ev(191, 1)]).unwrap();
        vd.emit(&[ev(191, 0)]).unwrap();
        vd.emit(&[ev(192, 1)]).unwrap();
        // not configured: dropped
        vd.emit(&[ev(190, 1)]).unwrap();
        vd.emit(&[ev(190, 0)]).unwrap();
        vd.emit(&[ev(193, 1)]).unwrap();
        vd.emit(&[ev(193, 2)]).unwrap();
        vd.emit(&[ev(193, 0)]).unwrap();
        let mut got = Vec::new();
        for _ in 0..100 {
            got.extend(knob.drain());
            if got.len() >= 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(200));
        got.extend(knob.drain());
        let actions: Vec<_> = got
            .iter()
            .map(|m| match m {
                Msg::Action(a, _) => Some(*a),
                Msg::Note(_) => None,
            })
            .collect();
        assert_eq!(actions, vec![Some(KnobAction::Up), Some(KnobAction::Down), Some(KnobAction::Press)], "{got:?}");
        // leaving DJ mode: the device is free again
        drop(knob);
        let mut after = find_device(NAME, &codes).unwrap();
        assert!(after.grab().is_ok(), "the grab was not released");
    }
}
