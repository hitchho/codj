//! A virtual knob for testing `[knob]` without the hardware: a uinput device named
//! `codj-fake-knob` with the keys 191 (up), 192 (down) and 193 (press), so the desktop does not
//! react to it. Point a scratch config at it:
//!
//! ```toml
//! [knob]
//! device = "codj-fake-knob"
//! up = 191
//! down = 192
//! press = 193
//! ```
//!
//! Then type lines on stdin: `u 5` turns five detents up, `d 3` three down, `p` presses;
//! `u 20 10` sends twenty detents 10 ms apart (spinning). Needs a writable /dev/uinput.

use std::io::BufRead;
use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};

fn main() {
    let mut keys = AttributeSet::<KeyCode>::new();
    for c in 191..=193u16 {
        keys.insert(KeyCode::new(c));
    }
    let mut vd = VirtualDevice::builder()
        .and_then(|b| b.name("codj-fake-knob").with_keys(&keys))
        .and_then(|b| b.build())
        .expect("cannot create the virtual device (is /dev/uinput writable?)");
    eprintln!("codj-fake-knob ready: u N [ms], d N [ms], p");
    let mut tap = |code: u16| {
        for value in [1, 0] {
            let _ = vd.emit(&[InputEvent::new(EventType::KEY.0, code, value)]);
        }
    };
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let mut w = line.split_whitespace();
        let (code, n, gap) = match w.next() {
            Some("u") => (191, w.next().and_then(|n| n.parse().ok()).unwrap_or(1), w.next().and_then(|g| g.parse().ok()).unwrap_or(150)),
            Some("d") => (192, w.next().and_then(|n| n.parse().ok()).unwrap_or(1), w.next().and_then(|g| g.parse().ok()).unwrap_or(150)),
            Some("p") => (193, 1, 0),
            _ => continue,
        };
        for _ in 0..n {
            tap(code);
            std::thread::sleep(Duration::from_millis(gap));
        }
    }
}
