//! Keyboard side of DJ mode: key -> binding, hold/tap handling and continuous ramps.
//!
//! Everything here is pure (time is passed in), so it is tested without a terminal or a daemon.
//!
//! Two modes, chosen by whether the terminal reports key releases (kitty keyboard protocol):
//! - **release events**: a press acts at once, holding continues (nudge, cue preview, ramps) and
//!   the release ends it.
//! - **no release events** (plain terminals, tmux): the same keys degrade to taps. Cue toggles
//!   the preview, nudge is a short timed bend that auto-repeat keeps alive, and ramps become one
//!   step per key event (auto-repeat then sweeps).

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::hints::{self, Accel, Hints, Target, Typed};
use crate::dj_types::{DeckInfo, MixerInfo};

/// Deck index: 0 = A, 1 = B
pub type Deck = u8;

/// A tap/hold must last this long before a ramp starts (a shorter press is one step)
pub const HOLD_DELAY: Duration = Duration::from_millis(220);
/// Seconds for a held EQ / filter / fader / crossfader key to sweep its whole range
pub const FULL_SWEEP_S: f64 = 1.5;
/// Never send more often than this per control (about 30 Hz)
pub const SEND_GAP: Duration = Duration::from_millis(30);
/// With release events: a held key that stops repeating for this long counts as released
const STALE: Duration = Duration::from_secs(3);
/// Without release events: a nudge tap bends the tempo for this long
pub const NUDGE_TAP: Duration = Duration::from_millis(300);
/// Without release events: key events closer than this are auto-repeat of one key
const REPEAT_GAP: Duration = Duration::from_millis(120);
/// A just-moved control ignores polled values for this long (the poll may still be stale)
const SETTLE: Duration = Duration::from_millis(500);
/// Beats for beat jump and for the loop key
/// Tap step of the playback rate (0.1 %) and its hold speed, 1 %/s growing to 4 %/s
const RATE_STEP: f64 = 0.001;
const RATE_MIN: f64 = 0.5;
const RATE_MAX: f64 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ctl {
    /// deck, band 0 = low, 1 = mid, 2 = high
    Eq(Deck, u8),
    Filter(Deck),
    Fader(Deck),
    Crossfader,
}

/// A continuously adjusted value
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot {
    Rate(Deck),
    Ctl(Ctl),
    /// trim in dB
    Gain(Deck),
    Master,
    CueMix,
    /// how far a synced tempo may stray (0.16 = 16 %)
    SyncRange,
}

impl Slot {
    fn range(self) -> (f64, f64) {
        match self {
            Slot::Rate(_) => (RATE_MIN, RATE_MAX),
            Slot::Ctl(Ctl::Fader(_)) => (0.0, 1.0),
            Slot::Ctl(_) => (-1.0, 1.0),
            Slot::Gain(_) => (-24.0, 12.0),
            Slot::Master => (0.0, 1.5),
            Slot::CueMix => (0.0, 1.0),
            Slot::SyncRange => (0.01, 1.0),
        }
    }

    /// the step of a tap, of `]` / `[` and of one encoder detent
    pub fn tap_step(self) -> f64 {
        match self {
            Slot::Rate(_) => RATE_STEP,
            Slot::Gain(_) => 0.5,
            Slot::SyncRange => 0.01,
            _ => 0.05,
        }
    }

    /// units per second at full ramp speed
    fn speed(self, ramp_s: f64) -> f64 {
        match self {
            Slot::Rate(_) => 0.01 + 0.03 * (ramp_s / 2.0).min(1.0),
            Slot::Gain(_) | Slot::Master | Slot::CueMix | Slot::SyncRange => 0.0,
            _ => {
                let (lo, hi) = self.range();
                (hi - lo) / FULL_SWEEP_S * ease(ramp_s)
            }
        }
    }

    /// the `DjSet` control name and deck
    pub fn control(self) -> (&'static str, u32) {
        match self {
            Slot::Rate(d) => ("rate", d as u32),
            Slot::Ctl(Ctl::Eq(d, 0)) => ("low", d as u32),
            Slot::Ctl(Ctl::Eq(d, 1)) => ("mid", d as u32),
            Slot::Ctl(Ctl::Eq(d, _)) => ("high", d as u32),
            Slot::Ctl(Ctl::Filter(d)) => ("filter", d as u32),
            Slot::Ctl(Ctl::Fader(d)) => ("fader", d as u32),
            Slot::Ctl(Ctl::Crossfader) => ("crossfader", 0),
            Slot::Gain(d) => ("gain", d as u32),
            Slot::Master => ("master", 0),
            Slot::CueMix => ("cue_mix", 0),
            Slot::SyncRange => ("sync_range", 0),
        }
    }
}

/// Ramp easing: starts slow, full speed after about 0.85 s of holding
fn ease(ramp_s: f64) -> f64 {
    (0.15 + ramp_s).min(1.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hold {
    Cue(Deck),
    Nudge(Deck, i8),
    Rate(Deck, i8),
    Ctl(Ctl, i8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tap {
    PlayPause(Deck),
    Sync(Deck),
    Loop(Deck),
    Hotcue(Deck, u8),
    HotcueClear(Deck, u8),
    /// back (-1) or forward (+1) by the deck's jump size
    BeatJump(Deck, i8),
    Headphones(Deck),
    Quantize,
}

/// Keys the screen itself handles
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Nav {
    Exit,
    Command,
    Search,
    LibUp,
    LibDown,
    LibPageUp,
    LibPageDown,
    /// load the highlighted track onto this deck
    Load(Deck),
    /// load the highlighted track onto the deck that is not playing
    LoadSmart,
    /// arm the next mix of the active set (F6)
    SetArmNext,
    /// auto mode on / off (F7)
    SetAuto,
    /// cancel the armed mix's remaining automation (F8)
    SetDisarm,
    /// start / stop recording a mix into the active set (F9)
    RecordToggle,
    /// the set editor pane (F10)
    Editor,
    /// the AI prompt box (F11)
    Prompt,
    /// `f`: show the control hints
    Hints,
    /// `;`: back to the previously selected control
    Toggle,
    /// `]` / `[`: edit the selected control by one step up / down
    Step(i8),
    /// `\`: reset the selected control
    Reset,
    /// an edit key with no control selected (tells the user how to select one)
    NoSelection,
    /// `?`: the key map on the screen, every key explained
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Binding {
    Tap(Tap),
    Hold(Hold),
    Nav(Nav),
}

/// A key with the terminal's shift quirks removed: letters are lowercase and `shift` is set for
/// capitals and shifted US symbols (`<` is `,` + shift), so every protocol mode looks the same
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NKey {
    pub code: KeyCode,
    pub alt: bool,
    pub shift: bool,
    pub ctrl: bool,
}

pub fn normalize(key: &KeyEvent) -> NKey {
    let mods = key.modifiers;
    let mut shift = mods.contains(KeyModifiers::SHIFT);
    let code = match key.code {
        KeyCode::Char(c) => {
            let (c, s) = unshift(c);
            shift |= s;
            KeyCode::Char(c)
        }
        KeyCode::BackTab => {
            shift = true;
            KeyCode::Tab
        }
        other => other,
    };
    NKey {
        code,
        alt: mods.contains(KeyModifiers::ALT),
        shift,
        ctrl: mods.contains(KeyModifiers::CONTROL),
    }
}

fn unshift(c: char) -> (char, bool) {
    match c {
        'A'..='Z' => (c.to_ascii_lowercase(), true),
        '!' => ('1', true),
        '@' => ('2', true),
        '#' => ('3', true),
        '$' => ('4', true),
        '%' => ('5', true),
        '^' => ('6', true),
        '&' => ('7', true),
        '*' => ('8', true),
        '(' => ('9', true),
        ')' => ('0', true),
        '<' => (',', true),
        '>' => ('.', true),
        ':' => (';', true),
        '?' => ('/', true),
        c => (c, false),
    }
}

/// The key map. Deck A on the left hand, deck B on the right, the mixer on Alt and `g b h n t y`.
pub fn bind(k: &NKey) -> Option<Binding> {
    use Binding::*;
    if k.ctrl {
        return None;
    }
    let KeyCode::Char(_) = k.code else {
        return match (k.code, k.alt) {
            (KeyCode::F(1), false) => Some(Tap(self::Tap::Headphones(0))),
            (KeyCode::F(2), false) => Some(Tap(self::Tap::Headphones(1))),
            (KeyCode::F(5), false) => Some(Tap(self::Tap::Quantize)),
            (KeyCode::F(6), false) => Some(Nav(self::Nav::SetArmNext)),
            (KeyCode::F(7), false) => Some(Nav(self::Nav::SetAuto)),
            (KeyCode::F(8), false) => Some(Nav(self::Nav::SetDisarm)),
            (KeyCode::F(9), false) => Some(Nav(self::Nav::RecordToggle)),
            (KeyCode::F(10), false) => Some(Nav(self::Nav::Editor)),
            (KeyCode::F(11), false) => Some(Nav(self::Nav::Prompt)),
            (KeyCode::Esc, false) => Some(Nav(self::Nav::Exit)),
            (KeyCode::Tab, false) => Some(Nav(self::Nav::Search)),
            (KeyCode::Up, false) => Some(Nav(self::Nav::LibUp)),
            (KeyCode::Down, false) => Some(Nav(self::Nav::LibDown)),
            (KeyCode::PageUp, false) => Some(Nav(self::Nav::LibPageUp)),
            (KeyCode::PageDown, false) => Some(Nav(self::Nav::LibPageDown)),
            (KeyCode::Enter, false) => Some(Nav(self::Nav::LoadSmart)),
            _ => None,
        };
    };
    let KeyCode::Char(c) = k.code else { return None };
    if k.alt {
        if k.shift {
            return None;
        }
        let (deck, rest) = match c {
            'q' | 'a' | 'w' | 's' | 'e' | 'd' | 'r' | 'f' => (0, c),
            // the right hand mirrors: u i o p on top, j k l ; below
            'u' => (1, 'q'),
            'j' => (1, 'a'),
            'i' => (1, 'w'),
            'k' => (1, 's'),
            'o' => (1, 'e'),
            'l' => (1, 'd'),
            'p' => (1, 'r'),
            ';' => (1, 'f'),
            _ => return None,
        };
        let (ctl, dir) = match rest {
            'q' => (Ctl::Eq(deck, 0), 1),
            'a' => (Ctl::Eq(deck, 0), -1),
            'w' => (Ctl::Eq(deck, 1), 1),
            's' => (Ctl::Eq(deck, 1), -1),
            'e' => (Ctl::Eq(deck, 2), 1),
            'd' => (Ctl::Eq(deck, 2), -1),
            'r' => (Ctl::Filter(deck), 1),
            _ => (Ctl::Filter(deck), -1),
        };
        return Some(Hold(self::Hold::Ctl(ctl, dir)));
    }
    if k.shift {
        return match c {
            '1'..='4' => Some(Tap(self::Tap::HotcueClear(0, c as u8 - b'1'))),
            '7' => Some(Tap(self::Tap::HotcueClear(1, 0))),
            '8' => Some(Tap(self::Tap::HotcueClear(1, 1))),
            '9' => Some(Tap(self::Tap::HotcueClear(1, 2))),
            '0' => Some(Tap(self::Tap::HotcueClear(1, 3))),
            'a' | ',' => Some(Nav(self::Nav::Load(0))),
            'b' | '.' => Some(Nav(self::Nav::Load(1))),
            ';' => Some(Nav(self::Nav::Command)),
            '/' => Some(Nav(self::Nav::Help)),
            _ => None,
        };
    }
    use self::{Hold as H, Tap as T};
    Some(match c {
        // deck A
        'z' => Hold(H::Cue(0)),
        'x' => Tap(T::PlayPause(0)),
        'c' => Tap(T::Sync(0)),
        'v' => Tap(T::Loop(0)),
        'a' => Hold(H::Rate(0, -1)),
        's' => Hold(H::Rate(0, 1)),
        'q' => Hold(H::Nudge(0, -1)),
        'w' => Hold(H::Nudge(0, 1)),
        '1'..='4' => Tap(T::Hotcue(0, c as u8 - b'1')),
        'e' => Tap(T::BeatJump(0, -1)),
        'r' => Tap(T::BeatJump(0, 1)),
        // deck B
        '/' => Hold(H::Cue(1)),
        '.' => Tap(T::PlayPause(1)),
        ',' => Tap(T::Sync(1)),
        'm' => Tap(T::Loop(1)),
        'k' => Hold(H::Rate(1, -1)),
        'l' => Hold(H::Rate(1, 1)),
        'o' => Hold(H::Nudge(1, -1)),
        'p' => Hold(H::Nudge(1, 1)),
        '7' => Tap(T::Hotcue(1, 0)),
        '8' => Tap(T::Hotcue(1, 1)),
        '9' => Tap(T::Hotcue(1, 2)),
        '0' => Tap(T::Hotcue(1, 3)),
        'u' => Tap(T::BeatJump(1, -1)),
        'i' => Tap(T::BeatJump(1, 1)),
        // control hints: `f` shows labels, a label selects, `]` `[` `\` edit, `;` toggles
        'f' => Nav(self::Nav::Hints),
        ';' => Nav(self::Nav::Toggle),
        ']' => Nav(self::Nav::Step(1)),
        '[' => Nav(self::Nav::Step(-1)),
        '\\' => Nav(self::Nav::Reset),
        // mixer
        'g' => Hold(H::Ctl(Ctl::Fader(0), 1)),
        'b' => Hold(H::Ctl(Ctl::Fader(0), -1)),
        'h' => Hold(H::Ctl(Ctl::Fader(1), 1)),
        'n' => Hold(H::Ctl(Ctl::Fader(1), -1)),
        't' => Hold(H::Ctl(Ctl::Crossfader, -1)),
        'y' => Hold(H::Ctl(Ctl::Crossfader, 1)),
        _ => return None,
    })
}

/// One D-Bus call for the executor
#[derive(Debug, Clone, PartialEq)]
pub enum DjCall {
    Load { deck: u32, path: String },
    Play(u32),
    Pause(u32),
    Cue { deck: u32, pressed: bool },
    Sync(u32),
    Nudge { deck: u32, amount: f64 },
    BeatJump { deck: u32, beats: f64 },
    Loop { deck: u32, beats: f64 },
    Hotcue { deck: u32, slot: u32, action: &'static str },
    Set { control: &'static str, deck: u32, value: f64 },
}

impl DjCall {
    pub fn name(&self) -> &'static str {
        match self {
            DjCall::Load { .. } => "DjLoad",
            DjCall::Play(_) => "DjPlay",
            DjCall::Pause(_) => "DjPause",
            DjCall::Cue { .. } => "DjCue",
            DjCall::Sync(_) => "DjSync",
            DjCall::Nudge { .. } => "DjNudge",
            DjCall::BeatJump { .. } => "DjBeatJump",
            DjCall::Loop { .. } => "DjLoop",
            DjCall::Hotcue { .. } => "DjHotcue",
            DjCall::Set { .. } => "DjSet",
        }
    }
}

/// Drops calls that a later one in the same batch supersedes (an older `Set` of the same control
/// or an older `Nudge` of the same deck), keeping the order of the rest. Used by the executor so
/// a slow bus never builds up a backlog of stale ramp values.
pub fn coalesce(calls: Vec<DjCall>) -> Vec<DjCall> {
    let superseded = |a: &DjCall, b: &DjCall| match (a, b) {
        (
            DjCall::Set { control: c1, deck: d1, .. },
            DjCall::Set { control: c2, deck: d2, .. },
        ) => c1 == c2 && d1 == d2,
        (DjCall::Nudge { deck: d1, .. }, DjCall::Nudge { deck: d2, .. }) => d1 == d2,
        _ => false,
    };
    let mut keep = vec![true; calls.len()];
    for i in 0..calls.len() {
        if calls[i + 1..].iter().any(|later| superseded(&calls[i], later)) {
            keep[i] = false;
        }
    }
    calls.into_iter().zip(keep).filter_map(|(c, k)| k.then_some(c)).collect()
}

struct Held {
    code: KeyCode,
    alt: bool,
    hold: Hold,
    since: Instant,
    last_event: Instant,
    last_tick: Instant,
}

struct SlotState {
    slot: Slot,
    value: f64,
    touched: Instant,
    dirty: bool,
}

pub struct DjKeys {
    release: bool,
    decks: Vec<DeckInfo>,
    mixer: MixerInfo,
    held: Vec<Held>,
    slots: Vec<SlotState>,
    out: Vec<DjCall>,
    /// without release events: cue preview is on for this deck
    cue_preview: [bool; 2],
    last_event: Option<(KeyCode, Instant)>,
    last_flush: Option<Instant>,
    /// the control hints and the selection
    hints: Hints,
    accel: Accel,
    /// controls the screen shows (None until the first frame: all of them)
    visible: Option<Vec<Target>>,
    /// beats of the next loop (`v` / `m`) and of a beat jump, per deck
    loop_len: [f64; 2],
    jump_len: [f64; 2],
    sync_range: f64,
    /// the last trim sent per deck with the path it was sent for (codjd reports the auto-gain
    /// only, not a manual trim)
    gain_set: [Option<(String, f64)>; 2],
    /// the auto-gain a track came with, per deck and path: what `\` goes back to
    auto_gain: [Option<(String, f64)>; 2],
}

impl DjKeys {
    /// `release`: the terminal reports key releases
    pub fn new(release: bool) -> Self {
        DjKeys {
            release,
            decks: Vec::new(),
            mixer: MixerInfo::default(),
            held: Vec::new(),
            slots: Vec::new(),
            out: Vec::new(),
            cue_preview: [false; 2],
            last_event: None,
            last_flush: None,
            hints: Hints::default(),
            accel: Accel::default(),
            visible: None,
            loop_len: [hints::DEFAULT_BEATS; 2],
            jump_len: [hints::DEFAULT_BEATS; 2],
            sync_range: hints::DEFAULT_SYNC_RANGE,
            gain_set: [None, None],
            auto_gain: [None, None],
        }
    }

    pub fn release_events(&self) -> bool {
        self.release
    }

    /// The latest polled state; toggles and untouched controls start from it
    pub fn set_snapshot(&mut self, decks: &[DeckInfo], mixer: &MixerInfo) {
        self.decks = decks.to_vec();
        self.mixer = mixer.clone();
        // remember the auto-gain a track was loaded with, before any trim of ours shows up
        for (i, d) in decks.iter().enumerate().take(2) {
            let known = self.auto_gain[i].as_ref().is_some_and(|(p, _)| *p == d.path);
            if d.loaded && !known {
                self.auto_gain[i] = Some((d.path.clone(), d.gain_db));
            }
        }
    }

    fn current(&self, slot: Slot) -> f64 {
        let deck = |d: Deck| self.decks.get(d as usize);
        match slot {
            Slot::Rate(d) => deck(d).map_or(1.0, |i| if i.rate > 0.0 { i.rate } else { 1.0 }),
            Slot::Ctl(Ctl::Eq(d, b)) => deck(d).map_or(0.0, |i| match b {
                0 => i.eq.0,
                1 => i.eq.1,
                _ => i.eq.2,
            }),
            Slot::Ctl(Ctl::Filter(d)) => deck(d).map_or(0.0, |i| i.filter),
            Slot::Ctl(Ctl::Fader(d)) => deck(d).map_or(0.0, |i| i.fader),
            Slot::Ctl(Ctl::Crossfader) => self.mixer.crossfader,
            Slot::Gain(d) => deck(d).map_or(0.0, |i| match &self.gain_set[d as usize] {
                Some((p, v)) if *p == i.path => *v,
                _ => i.gain_db,
            }),
            Slot::Master => self.mixer.master,
            Slot::CueMix => self.mixer.cue_mix,
            Slot::SyncRange => self.sync_range,
        }
    }

    /// What `\` sets a slot to: the neutral position, for the trim the auto-gain of the track
    fn default_of(&self, slot: Slot) -> f64 {
        match slot {
            Slot::Rate(_) => 1.0,
            Slot::Ctl(Ctl::Fader(_)) => 1.0,
            Slot::Ctl(_) => 0.0,
            Slot::Gain(d) => {
                let path = self.decks.get(d as usize).map(|i| i.path.as_str());
                match &self.auto_gain[d as usize] {
                    Some((p, g)) if Some(p.as_str()) == path => *g,
                    _ => 0.0,
                }
            }
            Slot::Master => 1.0,
            Slot::CueMix => 0.0,
            Slot::SyncRange => hints::DEFAULT_SYNC_RANGE,
        }
    }

    /// The value a target has now, for the status chip
    pub fn value_of(&self, t: Target, now: Instant) -> f64 {
        match t {
            Target::Slot(s) => self.target(s, now),
            Target::LoopLen(d) => self.loop_len[d as usize],
            Target::Jump(d) => self.jump_len[d as usize],
        }
    }

    /// Trim to show for a deck: the last one sent from here, else the auto-gain
    pub fn gain_shown(&self, deck: Deck, now: Instant) -> f64 {
        self.target(Slot::Gain(deck), now)
    }

    pub fn hints(&self) -> &Hints {
        &self.hints
    }

    pub fn loop_len(&self) -> [f64; 2] {
        self.loop_len
    }

    pub fn jump_len(&self) -> [f64; 2] {
        self.jump_len
    }

    pub fn sync_range(&self) -> f64 {
        self.sync_range
    }

    /// The controls the last frame had room for (None until the first frame)
    pub fn visible(&self) -> Option<&[Target]> {
        self.visible.as_deref()
    }

    /// The controls the last frame had room for
    pub fn set_visible(&mut self, targets: Vec<Target>) {
        self.visible = Some(targets);
    }

    /// Ends the hint session without selecting anything (a pane took the keyboard)
    pub fn cancel_hints(&mut self) {
        self.hints.cancel();
    }

    /// One edit step of the selected control: `dir` = +1 / -1, scaled by the encoder
    /// acceleration (fast repeats move further). False when nothing is selected.
    pub fn edit(&mut self, dir: i32, now: Instant) -> bool {
        let Some(target) = self.hints.selected else { return false };
        let n = self.accel.detent(dir, now) as i32 * dir.signum();
        match target {
            Target::Slot(slot) => self.bump(slot, n as f64 * slot.tap_step(), now),
            Target::LoopLen(d) => self.loop_len[d as usize] = hints::ladder_step(self.loop_len[d as usize], n),
            Target::Jump(d) => self.jump_len[d as usize] = hints::ladder_step(self.jump_len[d as usize], n),
        }
        true
    }

    /// Resets the selected control to its neutral value. False when nothing is selected.
    pub fn reset(&mut self, now: Instant) -> bool {
        let Some(target) = self.hints.selected else { return false };
        match target {
            Target::Slot(slot) => {
                let delta = self.default_of(slot) - self.target(slot, now);
                self.bump(slot, delta, now);
            }
            Target::LoopLen(d) => self.loop_len[d as usize] = hints::DEFAULT_BEATS,
            Target::Jump(d) => self.jump_len[d as usize] = hints::DEFAULT_BEATS,
        }
        true
    }

    /// The wheel or a knob turn: edits the selection. False when there is none (the caller scrolls
    /// the library); true and ignored while the hints are up.
    pub fn wheel(&mut self, dir: i32, now: Instant) -> bool {
        if self.hints.active {
            return true;
        }
        self.edit(dir, now)
    }

    /// Knob press: resets the selection; false when there is none
    pub fn press(&mut self, now: Instant) -> bool {
        if self.hints.active {
            return true;
        }
        self.reset(now)
    }

    fn open_hints(&mut self) {
        // a held key's release would be swallowed from here on: end the holds now
        for h in std::mem::take(&mut self.held) {
            self.released(h.hold);
        }
        self.hints.open();
    }

    /// A key while the hints are up: letters pick a label, Esc / `f` close, nothing else acts
    fn hint_key(&mut self, nk: &NKey, kind: KeyEventKind) {
        if kind != KeyEventKind::Press {
            return;
        }
        match nk.code {
            KeyCode::Esc => self.hints.cancel(),
            KeyCode::Backspace => self.hints.backspace(),
            KeyCode::Char('f') if !nk.alt && !nk.ctrl => self.hints.cancel(),
            KeyCode::Char(c) if !nk.alt && !nk.ctrl && !nk.shift => {
                let _: Typed = self.hints.letter(c, self.visible.as_deref());
            }
            _ => {}
        }
    }

    /// Where the control is going: our own latest value while it is being moved, else the daemon's
    pub fn target(&self, slot: Slot, now: Instant) -> f64 {
        match self.slots.iter().find(|s| s.slot == slot) {
            Some(s) if s.dirty || now.saturating_duration_since(s.touched) < SETTLE => s.value,
            _ => self.current(slot),
        }
    }

    fn bump(&mut self, slot: Slot, delta: f64, now: Instant) {
        let (lo, hi) = slot.range();
        let before = self.target(slot, now);
        let value = ((before + delta).clamp(lo, hi) * 1e4).round() / 1e4;
        let changed = (value - before).abs() > 1e-9;
        match slot {
            Slot::SyncRange => self.sync_range = value,
            Slot::Gain(d) => {
                if let Some(deck) = self.decks.get(d as usize) {
                    self.gain_set[d as usize] = Some((deck.path.clone(), value));
                }
            }
            _ => {}
        }
        match self.slots.iter_mut().find(|s| s.slot == slot) {
            Some(s) => {
                s.value = value;
                s.touched = now;
                s.dirty |= changed;
            }
            None if changed => self.slots.push(SlotState { slot, value, touched: now, dirty: true }),
            None => {}
        }
    }

    /// Handles a key event. Performance keys queue calls (collected by [`tick`]); keys for the
    /// screen itself come back as [`Nav`].
    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) -> Option<Nav> {
        let nk = normalize(key);
        if key.kind == KeyEventKind::Release {
            self.release_key(&nk, now);
            return None;
        }
        // without release events a burst of events from one key is its auto-repeat
        let stream = !self.release
            && self
                .last_event
                .is_some_and(|(c, t)| c == nk.code && now.saturating_duration_since(t) < REPEAT_GAP);
        self.last_event = Some((nk.code, now));
        let repeat = key.kind == KeyEventKind::Repeat;
        if self.hints.active {
            self.hint_key(&nk, key.kind);
            return None;
        }
        // Esc steps back: first the selection goes, then the screen
        if nk.code == KeyCode::Esc && !repeat && !stream && self.hints.selected.is_some() {
            self.hints.clear();
            return None;
        }
        match bind(&nk)? {
            Binding::Nav(Nav::Hints) => {
                if !(repeat || stream) {
                    self.open_hints();
                }
                None
            }
            Binding::Nav(Nav::Toggle) => {
                if !(repeat || stream) {
                    self.hints.toggle();
                }
                None
            }
            // a held `]` keeps stepping (and speeds up through the acceleration)
            Binding::Nav(Nav::Step(dir)) => {
                if self.edit(dir as i32, now) {
                    None
                } else {
                    (!(repeat || stream)).then_some(Nav::NoSelection)
                }
            }
            Binding::Nav(Nav::Reset) => {
                if repeat || stream || self.reset(now) {
                    None
                } else {
                    Some(Nav::NoSelection)
                }
            }
            Binding::Nav(n) => {
                let scrolls = matches!(n, Nav::LibUp | Nav::LibDown | Nav::LibPageUp | Nav::LibPageDown);
                (scrolls || !(repeat || stream)).then_some(n)
            }
            Binding::Tap(t) => {
                if !(repeat || stream) {
                    self.tap(t);
                }
                None
            }
            Binding::Hold(h) => {
                self.press_hold(h, &nk, repeat, stream, now);
                None
            }
        }
    }

    fn tap(&mut self, t: Tap) {
        let calls = &mut self.out;
        match t {
            Tap::PlayPause(d) => {
                let playing = self.decks.get(d as usize).is_some_and(|i| i.playing);
                calls.push(if playing { DjCall::Pause(d as u32) } else { DjCall::Play(d as u32) });
            }
            Tap::Sync(d) => calls.push(DjCall::Sync(d as u32)),
            Tap::Loop(d) => {
                let looping = self.decks.get(d as usize).is_some_and(|i| i.loop_beats > 0.0);
                let beats = if looping { 0.0 } else { self.loop_len[d as usize] };
                calls.push(DjCall::Loop { deck: d as u32, beats });
            }
            Tap::Hotcue(d, slot) => {
                let set = self.decks.get(d as usize).is_some_and(|i| i.hotcue(slot as usize).is_some());
                let action = if set { "jump" } else { "set" };
                calls.push(DjCall::Hotcue { deck: d as u32, slot: slot as u32, action });
            }
            Tap::HotcueClear(d, slot) => {
                calls.push(DjCall::Hotcue { deck: d as u32, slot: slot as u32, action: "clear" })
            }
            Tap::BeatJump(d, dir) => {
                calls.push(DjCall::BeatJump { deck: d as u32, beats: dir as f64 * self.jump_len[d as usize] })
            }
            Tap::Headphones(d) => {
                let on = self.decks.get(d as usize).is_some_and(|i| i.headphones);
                calls.push(DjCall::Set {
                    control: "cue",
                    deck: d as u32,
                    value: if on { 0.0 } else { 1.0 },
                });
            }
            Tap::Quantize => calls.push(DjCall::Set {
                control: "quantize",
                deck: 0,
                value: if self.mixer.quantize { 0.0 } else { 1.0 },
            }),
        }
    }

    fn press_hold(&mut self, hold: Hold, nk: &NKey, repeat: bool, stream: bool, now: Instant) {
        if self.release {
            if let Some(h) = self.held.iter_mut().find(|h| h.code == nk.code && h.alt == nk.alt) {
                h.last_event = now;
                return;
            }
            if repeat {
                // the press was missed (focus change), a lone repeat must not start anything
                return;
            }
            self.held.push(Held {
                code: nk.code,
                alt: nk.alt,
                hold,
                since: now,
                last_event: now,
                last_tick: now + HOLD_DELAY,
            });
            match hold {
                Hold::Cue(d) => self.out.push(DjCall::Cue { deck: d as u32, pressed: true }),
                Hold::Nudge(d, s) => self.out.push(DjCall::Nudge { deck: d as u32, amount: s as f64 }),
                Hold::Rate(d, s) => self.step(Slot::Rate(d), s, now),
                Hold::Ctl(c, s) => self.step(Slot::Ctl(c), s, now),
            }
            return;
        }
        // no release events
        match hold {
            Hold::Cue(d) => {
                if stream {
                    return;
                }
                let i = d as usize;
                let playing = self.decks.get(i).is_some_and(|x| x.playing);
                if self.cue_preview[i] {
                    self.cue_preview[i] = false;
                    self.out.push(DjCall::Cue { deck: d as u32, pressed: false });
                } else if playing {
                    // playing: the press stops at the cue point, nothing to hold
                    self.out.push(DjCall::Cue { deck: d as u32, pressed: true });
                    self.out.push(DjCall::Cue { deck: d as u32, pressed: false });
                } else {
                    self.cue_preview[i] = true;
                    self.out.push(DjCall::Cue { deck: d as u32, pressed: true });
                }
            }
            Hold::Nudge(d, s) => {
                let active = self.held.iter_mut().find(|h| matches!(h.hold, Hold::Nudge(dd, ss) if dd == d && ss == s));
                match active {
                    Some(h) => h.last_event = now,
                    None => {
                        self.held.retain(|h| !matches!(h.hold, Hold::Nudge(dd, _) if dd == d));
                        self.held.push(Held {
                            code: nk.code,
                            alt: nk.alt,
                            hold,
                            since: now,
                            last_event: now,
                            last_tick: now,
                        });
                        self.out.push(DjCall::Nudge { deck: d as u32, amount: s as f64 });
                    }
                }
            }
            Hold::Rate(d, s) => self.step(Slot::Rate(d), s, now),
            Hold::Ctl(c, s) => self.step(Slot::Ctl(c), s, now),
        }
    }

    fn step(&mut self, slot: Slot, dir: i8, now: Instant) {
        self.bump(slot, dir as f64 * slot.tap_step(), now);
    }

    fn release_key(&mut self, nk: &NKey, _now: Instant) {
        if !self.release {
            return;
        }
        // Alt may be let go before the letter, so fall back to the key alone
        let at = self
            .held
            .iter()
            .position(|h| h.code == nk.code && h.alt == nk.alt)
            .or_else(|| self.held.iter().position(|h| h.code == nk.code));
        if let Some(i) = at {
            let h = self.held.remove(i);
            self.released(h.hold);
        }
    }

    fn released(&mut self, hold: Hold) {
        match hold {
            Hold::Cue(d) => self.out.push(DjCall::Cue { deck: d as u32, pressed: false }),
            Hold::Nudge(d, _) => {
                // the opposite nudge key may still be down
                let amount = self
                    .held
                    .iter()
                    .find_map(|h| match h.hold {
                        Hold::Nudge(dd, s) if dd == d => Some(s as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0);
                self.out.push(DjCall::Nudge { deck: d as u32, amount });
            }
            Hold::Rate(..) | Hold::Ctl(..) => {}
        }
    }

    /// Ends every hold now (leaving the screen, or a key held across a change of keyboard mode
    /// will never see its release): cue previews stop, nudges return to 0. The next [`tick`]
    /// sends these together with anything still queued.
    pub fn release_all(&mut self) {
        for h in std::mem::take(&mut self.held) {
            self.released(h.hold);
        }
        self.cue_preview = [false; 2];
        self.last_flush = None;
    }

    /// Advances ramps and timeouts; returns the calls to send now (at most one `Set` per control
    /// per [`SEND_GAP`]).
    pub fn tick(&mut self, now: Instant) -> Vec<DjCall> {
        // lost releases / ended nudge taps
        let release = self.release;
        let mut gone = Vec::new();
        self.held.retain(|h| {
            let age = now.saturating_duration_since(h.last_event);
            let dead = if release {
                // a held key keeps sending Repeat events, so silence means a lost release
                age > STALE
            } else {
                age > if matches!(h.hold, Hold::Nudge(..)) { NUDGE_TAP } else { Duration::from_millis(300) }
            };
            if dead {
                gone.push(h.hold);
            }
            !dead
        });
        for h in gone {
            self.released(h);
        }
        // ramps for held keys (with release events)
        if self.release {
            let mut moves = Vec::new();
            for h in &mut self.held {
                let ramp_from = h.since + HOLD_DELAY;
                if now < ramp_from {
                    continue;
                }
                let dt = now.saturating_duration_since(h.last_tick.max(ramp_from)).min(Duration::from_millis(100));
                h.last_tick = now;
                let ramp_s = now.saturating_duration_since(ramp_from).as_secs_f64();
                match h.hold {
                    Hold::Rate(d, s) => moves.push((Slot::Rate(d), s, ramp_s, dt)),
                    Hold::Ctl(c, s) => moves.push((Slot::Ctl(c), s, ramp_s, dt)),
                    _ => {}
                }
            }
            for (slot, dir, ramp_s, dt) in moves {
                self.bump(slot, dir as f64 * slot.speed(ramp_s) * dt.as_secs_f64(), now);
            }
        }
        let mut calls = std::mem::take(&mut self.out);
        if self.last_flush.map_or(true, |t| now.saturating_duration_since(t) >= SEND_GAP) {
            let mut sent = false;
            for s in &mut self.slots {
                if s.dirty {
                    s.dirty = false;
                    let (control, deck) = s.slot.control();
                    calls.push(DjCall::Set { control, deck, value: s.value });
                    sent = true;
                }
            }
            if sent {
                self.last_flush = Some(now);
            }
        }
        calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn ev(code: KeyCode, mods: KeyModifiers, kind: KeyEventKind) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind, state: KeyEventState::NONE }
    }
    fn press(c: char) -> KeyEvent {
        ev(KeyCode::Char(c), KeyModifiers::NONE, KeyEventKind::Press)
    }
    fn rel(c: char) -> KeyEvent {
        ev(KeyCode::Char(c), KeyModifiers::NONE, KeyEventKind::Release)
    }
    fn alt(c: char, kind: KeyEventKind) -> KeyEvent {
        ev(KeyCode::Char(c), KeyModifiers::ALT, kind)
    }
    fn b(c: char) -> Option<Binding> {
        bind(&normalize(&press(c)))
    }
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    fn deck(playing: bool) -> DeckInfo {
        DeckInfo { loaded: true, playing, rate: 1.0, ..Default::default() }
    }
    fn keys(release: bool) -> DjKeys {
        let mut k = DjKeys::new(release);
        k.set_snapshot(&[deck(false), deck(false)], &MixerInfo::default());
        k
    }
    fn sets(calls: &[DjCall]) -> Vec<(&'static str, u32, f64)> {
        calls
            .iter()
            .filter_map(|c| match c {
                DjCall::Set { control, deck, value } => Some((*control, *deck, *value)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn key_map_deck_a_and_b() {
        assert_eq!(b('z'), Some(Binding::Hold(Hold::Cue(0))));
        assert_eq!(b('x'), Some(Binding::Tap(Tap::PlayPause(0))));
        assert_eq!(b('c'), Some(Binding::Tap(Tap::Sync(0))));
        assert_eq!(b('v'), Some(Binding::Tap(Tap::Loop(0))));
        assert_eq!(b('a'), Some(Binding::Hold(Hold::Rate(0, -1))));
        assert_eq!(b('s'), Some(Binding::Hold(Hold::Rate(0, 1))));
        assert_eq!(b('q'), Some(Binding::Hold(Hold::Nudge(0, -1))));
        assert_eq!(b('3'), Some(Binding::Tap(Tap::Hotcue(0, 2))));
        assert_eq!(b('e'), Some(Binding::Tap(Tap::BeatJump(0, -1))));
        assert_eq!(b('r'), Some(Binding::Tap(Tap::BeatJump(0, 1))));
        assert_eq!(b('/'), Some(Binding::Hold(Hold::Cue(1))));
        assert_eq!(b('.'), Some(Binding::Tap(Tap::PlayPause(1))));
        assert_eq!(b(','), Some(Binding::Tap(Tap::Sync(1))));
        assert_eq!(b('m'), Some(Binding::Tap(Tap::Loop(1))));
        assert_eq!(b('k'), Some(Binding::Hold(Hold::Rate(1, -1))));
        assert_eq!(b('p'), Some(Binding::Hold(Hold::Nudge(1, 1))));
        assert_eq!(b('0'), Some(Binding::Tap(Tap::Hotcue(1, 3))));
        assert_eq!(b('7'), Some(Binding::Tap(Tap::Hotcue(1, 0))));
        assert_eq!(b('u'), Some(Binding::Tap(Tap::BeatJump(1, -1))));
        assert_eq!(b('i'), Some(Binding::Tap(Tap::BeatJump(1, 1))));
        assert_eq!(b('5'), None);
        // the old jump keys are free now (f, ; and j are taken by the control hints later on)
        assert_eq!(b('d'), None);
        assert_eq!(b('j'), None);
    }

    #[test]
    fn key_map_mixer() {
        let a = |c| bind(&normalize(&alt(c, KeyEventKind::Press)));
        assert_eq!(a('q'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(0, 0), 1))));
        assert_eq!(a('a'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(0, 0), -1))));
        assert_eq!(a('s'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(0, 1), -1))));
        assert_eq!(a('e'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(0, 2), 1))));
        assert_eq!(a('f'), Some(Binding::Hold(Hold::Ctl(Ctl::Filter(0), -1))));
        assert_eq!(a('u'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(1, 0), 1))));
        assert_eq!(a('j'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(1, 0), -1))));
        assert_eq!(a('i'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(1, 1), 1))));
        assert_eq!(a('l'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(1, 2), -1))));
        assert_eq!(a('p'), Some(Binding::Hold(Hold::Ctl(Ctl::Filter(1), 1))));
        assert_eq!(a(';'), Some(Binding::Hold(Hold::Ctl(Ctl::Filter(1), -1))));
        assert_eq!(a('z'), None);
        assert_eq!(b('g'), Some(Binding::Hold(Hold::Ctl(Ctl::Fader(0), 1))));
        assert_eq!(b('b'), Some(Binding::Hold(Hold::Ctl(Ctl::Fader(0), -1))));
        assert_eq!(b('h'), Some(Binding::Hold(Hold::Ctl(Ctl::Fader(1), 1))));
        assert_eq!(b('n'), Some(Binding::Hold(Hold::Ctl(Ctl::Fader(1), -1))));
        assert_eq!(b('t'), Some(Binding::Hold(Hold::Ctl(Ctl::Crossfader, -1))));
        assert_eq!(b('y'), Some(Binding::Hold(Hold::Ctl(Ctl::Crossfader, 1))));
        let f = |n| bind(&normalize(&ev(KeyCode::F(n), KeyModifiers::NONE, KeyEventKind::Press)));
        assert_eq!(f(1), Some(Binding::Tap(Tap::Headphones(0))));
        assert_eq!(f(2), Some(Binding::Tap(Tap::Headphones(1))));
        assert_eq!(f(5), Some(Binding::Tap(Tap::Quantize)));
        assert_eq!(f(6), Some(Binding::Nav(Nav::SetArmNext)));
        assert_eq!(f(7), Some(Binding::Nav(Nav::SetAuto)));
        assert_eq!(f(8), Some(Binding::Nav(Nav::SetDisarm)));
        assert_eq!(f(9), Some(Binding::Nav(Nav::RecordToggle)));
        assert_eq!(f(10), Some(Binding::Nav(Nav::Editor)));
        assert_eq!(f(11), Some(Binding::Nav(Nav::Prompt)));
    }

    #[test]
    fn shifted_keys_in_every_protocol_mode() {
        // shift + digit: the digit with SHIFT (kitty), or the US symbol (legacy)
        for ev in [
            ev(KeyCode::Char('2'), KeyModifiers::SHIFT, KeyEventKind::Press),
            press('@'),
        ] {
            assert_eq!(bind(&normalize(&ev)), Some(Binding::Tap(Tap::HotcueClear(0, 1))));
        }
        assert_eq!(
            bind(&normalize(&ev(KeyCode::Char('0'), KeyModifiers::SHIFT, KeyEventKind::Press))),
            Some(Binding::Tap(Tap::HotcueClear(1, 3)))
        );
        // loading: < > and shift+a / shift+b, with or without the SHIFT flag
        assert_eq!(b('<'), Some(Binding::Nav(Nav::Load(0))));
        assert_eq!(b('>'), Some(Binding::Nav(Nav::Load(1))));
        assert_eq!(b('A'), Some(Binding::Nav(Nav::Load(0))));
        assert_eq!(
            bind(&normalize(&ev(KeyCode::Char('b'), KeyModifiers::SHIFT, KeyEventKind::Press))),
            Some(Binding::Nav(Nav::Load(1)))
        );
        assert_eq!(b(':'), Some(Binding::Nav(Nav::Command)));
        assert_eq!(
            bind(&normalize(&ev(KeyCode::Char(';'), KeyModifiers::SHIFT, KeyEventKind::Press))),
            Some(Binding::Nav(Nav::Command))
        );
        // a capital performance letter is not a performance key
        assert_eq!(b('Z'), None);
        // ctrl combos never reach the decks
        assert_eq!(
            bind(&normalize(&ev(KeyCode::Char('x'), KeyModifiers::CONTROL, KeyEventKind::Press))),
            None
        );
    }

    #[test]
    fn nav_keys() {
        let n = |c| bind(&normalize(&ev(c, KeyModifiers::NONE, KeyEventKind::Press)));
        assert_eq!(n(KeyCode::Esc), Some(Binding::Nav(Nav::Exit)));
        assert_eq!(n(KeyCode::Tab), Some(Binding::Nav(Nav::Search)));
        assert_eq!(n(KeyCode::Down), Some(Binding::Nav(Nav::LibDown)));
        assert_eq!(n(KeyCode::Enter), Some(Binding::Nav(Nav::LoadSmart)));
        assert_eq!(
            bind(&normalize(&ev(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press))),
            Some(Binding::Nav(Nav::Search))
        );
    }

    #[test]
    fn taps_resolve_against_the_polled_state() {
        let t0 = Instant::now();
        let mut k = DjKeys::new(true);
        let mut a = deck(true);
        a.hotcues = vec![12.0, -1.0, -1.0, -1.0];
        a.loop_beats = 4.0;
        let mut bb = deck(false);
        bb.headphones = true;
        k.set_snapshot(&[a, bb], &MixerInfo { quantize: true, ..Default::default() });
        for c in ['x', '.', '1', '2', 'v', 'm', 'c', 'e'] {
            k.on_key(&press(c), t0);
        }
        k.on_key(&ev(KeyCode::Char('2'), KeyModifiers::SHIFT, KeyEventKind::Press), t0);
        k.on_key(&ev(KeyCode::F(2), KeyModifiers::NONE, KeyEventKind::Press), t0);
        k.on_key(&ev(KeyCode::F(5), KeyModifiers::NONE, KeyEventKind::Press), t0);
        let hc = |slot, action| DjCall::Hotcue { deck: 0, slot, action };
        assert_eq!(
            k.tick(t0),
            vec![
                DjCall::Pause(0),
                DjCall::Play(1),
                hc(0, "jump"),
                hc(1, "set"),
                DjCall::Loop { deck: 0, beats: 0.0 },
                DjCall::Loop { deck: 1, beats: 4.0 },
                DjCall::Sync(0),
                DjCall::BeatJump { deck: 0, beats: -4.0 },
                hc(1, "clear"),
                DjCall::Set { control: "cue", deck: 1, value: 0.0 },
                DjCall::Set { control: "quantize", deck: 0, value: 0.0 },
            ]
        );
    }

    #[test]
    fn repeat_events_do_not_retrigger_taps() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('x'), t0);
        k.on_key(&ev(KeyCode::Char('x'), KeyModifiers::NONE, KeyEventKind::Repeat), t0 + ms(500));
        k.on_key(&rel('x'), t0 + ms(600));
        assert_eq!(k.tick(t0 + ms(600)), vec![DjCall::Play(0)]);
    }

    #[test]
    fn cue_hold_previews_until_release() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('z'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Cue { deck: 0, pressed: true }]);
        k.on_key(&ev(KeyCode::Char('z'), KeyModifiers::NONE, KeyEventKind::Repeat), t0 + ms(400));
        assert!(k.tick(t0 + ms(400)).is_empty());
        k.on_key(&rel('z'), t0 + ms(800));
        assert_eq!(k.tick(t0 + ms(800)), vec![DjCall::Cue { deck: 0, pressed: false }]);
        // a stray release does nothing
        k.on_key(&rel('z'), t0 + ms(900));
        assert!(k.tick(t0 + ms(900)).is_empty());
    }

    #[test]
    fn cue_without_release_events_toggles() {
        let t0 = Instant::now();
        let mut k = keys(false);
        k.on_key(&press('/'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Cue { deck: 1, pressed: true }]);
        // auto-repeat of the held key is ignored
        k.on_key(&press('/'), t0 + ms(40));
        assert!(k.tick(t0 + ms(40)).is_empty());
        k.on_key(&press('/'), t0 + ms(900));
        assert_eq!(k.tick(t0 + ms(900)), vec![DjCall::Cue { deck: 1, pressed: false }]);
        // on a playing deck the press is stop-at-cue: nothing is left to toggle off
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press('/'), t0 + ms(2000));
        assert_eq!(
            k.tick(t0 + ms(2000)),
            vec![DjCall::Cue { deck: 1, pressed: true }, DjCall::Cue { deck: 1, pressed: false }]
        );
    }

    #[test]
    fn nudge_follows_the_key_with_release_events() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('w'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Nudge { deck: 0, amount: 1.0 }]);
        // the other nudge key too: releasing one hands over to the one still down
        k.on_key(&press('q'), t0 + ms(100));
        k.tick(t0 + ms(100));
        k.on_key(&rel('q'), t0 + ms(200));
        assert_eq!(k.tick(t0 + ms(200)), vec![DjCall::Nudge { deck: 0, amount: 1.0 }]);
        k.on_key(&rel('w'), t0 + ms(300));
        assert_eq!(k.tick(t0 + ms(300)), vec![DjCall::Nudge { deck: 0, amount: 0.0 }]);
    }

    #[test]
    fn nudge_is_a_timed_bend_without_release_events() {
        let t0 = Instant::now();
        let mut k = keys(false);
        k.on_key(&press('p'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Nudge { deck: 1, amount: 1.0 }]);
        // auto-repeat keeps the bend alive without resending it
        k.on_key(&press('p'), t0 + ms(250));
        assert!(k.tick(t0 + ms(250)).is_empty());
        assert!(k.tick(t0 + ms(500)).is_empty());
        assert_eq!(k.tick(t0 + ms(600)), vec![DjCall::Nudge { deck: 1, amount: 0.0 }]);
        assert!(k.tick(t0 + ms(700)).is_empty());
    }

    #[test]
    fn lost_cue_release_does_not_stick() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('z'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Cue { deck: 0, pressed: true }]);
        assert_eq!(k.tick(t0 + ms(3100)), vec![DjCall::Cue { deck: 0, pressed: false }]);
    }

    #[test]
    fn release_all_ends_holds_and_flushes_pending_work() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('w'), t0);
        k.on_key(&press('/'), t0);
        k.on_key(&press('x'), t0);
        k.on_key(&alt('q', KeyEventKind::Press), t0);
        k.release_all();
        let calls = k.tick(t0 + ms(5));
        assert!(calls.contains(&DjCall::Nudge { deck: 0, amount: 0.0 }));
        assert!(calls.contains(&DjCall::Cue { deck: 1, pressed: false }));
        assert!(calls.contains(&DjCall::Play(0)));
        assert!(calls.contains(&DjCall::Set { control: "low", deck: 0, value: 0.05 }));
        // nothing is left held
        assert!(k.tick(t0 + ms(100)).is_empty());
        k.on_key(&rel('w'), t0 + ms(120));
        assert!(k.tick(t0 + ms(130)).is_empty());
    }

    #[test]
    fn stale_hold_is_released_when_events_stop() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('w'), t0);
        k.tick(t0);
        assert!(k.tick(t0 + ms(2000)).is_empty());
        assert_eq!(k.tick(t0 + ms(3100)), vec![DjCall::Nudge { deck: 0, amount: 0.0 }]);
    }

    #[test]
    fn eq_tap_steps_once_and_hold_ramps_to_a_full_sweep() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // tap
        k.on_key(&alt('q', KeyEventKind::Press), t0);
        assert_eq!(sets(&k.tick(t0)), vec![("low", 0, 0.05)]);
        k.on_key(&alt('q', KeyEventKind::Release), t0 + ms(80));
        for i in 1..30 {
            assert!(k.tick(t0 + ms(80 + i * 33)).is_empty());
        }
        // hold from the start: decrease the high band of deck B (Alt+l)
        let mut k = keys(true);
        let mut now = t0;
        k.on_key(&alt('l', KeyEventKind::Press), now);
        let mut last = 0.0;
        let mut sent = 0;
        let mut reached = None;
        for i in 0..200 {
            now = t0 + ms(i * 10);
            for (c, d, v) in sets(&k.tick(now)) {
                assert_eq!((c, d), ("high", 1));
                assert!(v <= last + 1e-9, "monotonic");
                last = v;
                sent += 1;
            }
            if last <= -1.0 && reached.is_none() {
                reached = Some(now - t0);
            }
        }
        let t = reached.expect("sweeps to the end").as_secs_f64();
        assert!(t > 1.0 && t < 2.2, "full sweep took {t}s");
        // about 30 Hz at most: 2 s of ticks every 10 ms must not send more than ~70 values
        assert!(sent <= 70 && sent > 20, "sent {sent}");
        assert_eq!(last, -1.0);
    }

    #[test]
    fn ramp_stops_on_release_and_keeps_the_value() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('g'), t0);
        let mut v = 0.0;
        for i in 0..40 {
            for (_, _, x) in sets(&k.tick(t0 + ms(i * 10))) {
                v = x;
            }
        }
        k.on_key(&rel('g'), t0 + ms(400));
        assert!(v > 0.05);
        // whatever the ramp moved since the last send goes out once, then it is quiet
        let last = sets(&k.tick(t0 + ms(450)));
        assert!(last.len() <= 1);
        if let Some((_, _, x)) = last.first() {
            v = *x;
        }
        assert!(k.tick(t0 + ms(480)).is_empty());
        // a stale poll right after must not pull the value back: the next step starts from ours
        k.set_snapshot(&[deck(false), deck(false)], &MixerInfo::default());
        k.on_key(&press('g'), t0 + ms(500));
        let next = sets(&k.tick(t0 + ms(500)))[0].2;
        assert!((next - (v + 0.05).min(1.0)).abs() < 1e-3, "{next} vs {v}");
        // much later the daemon's value is authoritative again
        let mut polled = deck(false);
        polled.fader = 0.2;
        k.set_snapshot(&[polled, deck(false)], &MixerInfo::default());
        k.on_key(&rel('g'), t0 + ms(600));
        assert_eq!(k.target(Slot::Ctl(Ctl::Fader(0)), t0 + ms(5000)), 0.2);
    }

    #[test]
    fn fader_and_crossfader_are_clamped() {
        let t0 = Instant::now();
        let mut k = DjKeys::new(false);
        k.set_snapshot(&[DeckInfo { fader: 0.1, ..deck(false) }, deck(false)], &MixerInfo::default());
        for i in 0..50 {
            k.on_key(&press('b'), t0 + ms(i * 20));
            k.on_key(&press('y'), t0 + ms(i * 20));
        }
        let all = k.tick(t0 + ms(1000));
        assert_eq!(sets(&all), vec![("fader", 0, 0.0), ("crossfader", 0, 1.0)]);
    }

    #[test]
    fn pitch_tap_is_a_tenth_of_a_percent() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('s'), t0);
        assert_eq!(sets(&k.tick(t0)), vec![("rate", 0, 1.001)]);
        k.on_key(&rel('s'), t0 + ms(60));
        k.on_key(&press('k'), t0 + ms(500));
        assert_eq!(sets(&k.tick(t0 + ms(500))), vec![("rate", 1, 0.999)]);
    }

    #[test]
    fn pitch_hold_is_slow() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('s'), t0);
        let mut v = 1.0;
        for i in 0..100 {
            for (_, _, x) in sets(&k.tick(t0 + ms(i * 10))) {
                v = x;
            }
        }
        // one second of holding: well under 2 %
        assert!(v > 1.003 && v < 1.02, "{v}");
    }

    #[test]
    fn without_release_events_every_event_is_one_step() {
        let t0 = Instant::now();
        let mut k = keys(false);
        for i in 0..10 {
            k.on_key(&alt('r', KeyEventKind::Press), t0 + ms(i * 33));
        }
        assert_eq!(sets(&k.tick(t0 + ms(400))), vec![("filter", 0, 0.5)]);
    }

    #[test]
    fn alt_released_before_the_letter_still_ends_the_hold() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&alt('u', KeyEventKind::Press), t0);
        k.tick(t0);
        // the release arrives without the Alt flag
        k.on_key(&rel('u'), t0 + ms(400));
        k.tick(t0 + ms(400));
        assert!(k.tick(t0 + ms(800)).is_empty());
    }

    #[test]
    fn coalesce_keeps_only_the_latest_value() {
        let s = |v| DjCall::Set { control: "low", deck: 0, value: v };
        let calls = vec![
            s(0.1),
            DjCall::Nudge { deck: 0, amount: 1.0 },
            DjCall::Play(1),
            s(0.2),
            DjCall::Set { control: "low", deck: 1, value: 0.5 },
            DjCall::Nudge { deck: 0, amount: 0.0 },
            s(0.3),
        ];
        assert_eq!(
            coalesce(calls),
            vec![
                DjCall::Play(1),
                DjCall::Set { control: "low", deck: 1, value: 0.5 },
                DjCall::Nudge { deck: 0, amount: 0.0 },
                s(0.3),
            ]
        );
    }

    #[test]
    fn nav_keys_do_not_repeat_except_scrolling() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let e = |c, kind| ev(c, KeyModifiers::NONE, kind);
        assert_eq!(k.on_key(&e(KeyCode::Esc, KeyEventKind::Press), t0), Some(Nav::Exit));
        assert_eq!(k.on_key(&e(KeyCode::Esc, KeyEventKind::Repeat), t0), None);
        assert_eq!(k.on_key(&e(KeyCode::Down, KeyEventKind::Repeat), t0), Some(Nav::LibDown));
        assert_eq!(k.on_key(&e(KeyCode::Esc, KeyEventKind::Release), t0), None);
    }

    // ---- control hints, selection and editing

    fn tap_key(k: &mut DjKeys, c: char, t: Instant) {
        k.on_key(&press(c), t);
    }

    /// presses `f` and the letters of a label, with 100 ms between keys
    fn select(k: &mut DjKeys, label: &str, t: Instant) -> Instant {
        let mut t = t;
        tap_key(k, 'f', t);
        for c in label.chars() {
            t += ms(100);
            tap_key(k, c, t);
        }
        t + ms(100)
    }

    #[test]
    fn question_mark_is_the_help_key_and_slash_stays_cue_b() {
        // `?` is Shift + `/` however the terminal reports it
        assert_eq!(b('?'), Some(Binding::Nav(Nav::Help)));
        let shifted = ev(KeyCode::Char('/'), KeyModifiers::SHIFT, KeyEventKind::Press);
        assert_eq!(bind(&normalize(&shifted)), Some(Binding::Nav(Nav::Help)));
        let both = ev(KeyCode::Char('?'), KeyModifiers::SHIFT, KeyEventKind::Press);
        assert_eq!(bind(&normalize(&both)), Some(Binding::Nav(Nav::Help)));
        assert_eq!(b('/'), Some(Binding::Hold(Hold::Cue(1))));
        let mut k = keys(true);
        assert_eq!(k.on_key(&press('?'), Instant::now()), Some(Nav::Help));
        // while the hint labels are up it is ignored like every other non-label key
        k.on_key(&press('f'), Instant::now());
        assert_eq!(k.on_key(&press('?'), Instant::now()), None);
        assert!(k.hints().active);
    }

    #[test]
    fn hint_keys_in_the_key_map() {
        assert_eq!(b('f'), Some(Binding::Nav(Nav::Hints)));
        assert_eq!(b(';'), Some(Binding::Nav(Nav::Toggle)));
        assert_eq!(b(']'), Some(Binding::Nav(Nav::Step(1))));
        assert_eq!(b('['), Some(Binding::Nav(Nav::Step(-1))));
        assert_eq!(b('\\'), Some(Binding::Nav(Nav::Reset)));
        // `:` is still the command line, Alt+f / Alt+; still the filters
        assert_eq!(b(':'), Some(Binding::Nav(Nav::Command)));
        let a = |c| bind(&normalize(&alt(c, KeyEventKind::Press)));
        assert_eq!(a('f'), Some(Binding::Hold(Hold::Ctl(Ctl::Filter(0), -1))));
        assert_eq!(a(';'), Some(Binding::Hold(Hold::Ctl(Ctl::Filter(1), -1))));
        assert_eq!(a('e'), Some(Binding::Hold(Hold::Ctl(Ctl::Eq(0, 2), 1))));
        assert_eq!(a('r'), Some(Binding::Hold(Hold::Ctl(Ctl::Filter(0), 1))));
        // every plain key is bound at most once: the old jump keys d and j, and ; and f are not
        // jumps any more
        for c in ['d', 'j'] {
            assert_eq!(b(c), None);
        }
        for c in ['e', 'r', 'u', 'i'] {
            assert!(matches!(b(c), Some(Binding::Tap(Tap::BeatJump(..)))), "{c}");
        }
    }

    #[test]
    fn f_then_a_label_selects_and_the_keys_do_not_play_meanwhile() {
        let t0 = Instant::now();
        let mut k = keys(true);
        tap_key(&mut k, 'f', t0);
        assert!(k.hints().active);
        // while the labels are up a deck key does nothing: `x` is no label, it is ignored
        // (and `z` would be a prefix of nothing)
        tap_key(&mut k, 'x', t0 + ms(50));
        assert!(k.hints().active, "x starts the label xa / xs");
        tap_key(&mut k, 'f', t0 + ms(80));
        assert!(!k.hints().active, "f again cancels");
        tap_key(&mut k, 'f', t0 + ms(100));
        tap_key(&mut k, '1', t0 + ms(120));
        tap_key(&mut k, 'v', t0 + ms(140));
        tap_key(&mut k, 'c', t0 + ms(160));
        assert!(k.tick(t0 + ms(200)).is_empty(), "no deck action while hinting");
        // Esc cancels the hints, and only the hints
        assert_eq!(k.on_key(&ev(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press), t0 + ms(300)), None);
        assert!(!k.hints().active);
        assert_eq!(k.hints().selected, None);
        // a complete label selects and hides
        let t = select(&mut k, "a", t0 + ms(400));
        assert!(!k.hints().active);
        assert_eq!(k.hints().selected, Some(Target::Slot(Slot::Ctl(Ctl::Filter(0)))));
        // deck keys act again
        tap_key(&mut k, 'x', t);
        assert_eq!(k.tick(t), vec![DjCall::Play(0)]);
    }

    #[test]
    fn esc_goes_hints_then_selection_then_exit() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let esc = |k: &mut DjKeys, t| k.on_key(&ev(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press), t);
        let t = select(&mut k, "w", t0);
        assert!(k.hints().selected.is_some());
        assert_eq!(esc(&mut k, t), None);
        assert!(k.hints().selected.is_none());
        assert_eq!(esc(&mut k, t + ms(100)), Some(Nav::Exit));
    }

    #[test]
    fn bracket_keys_step_the_selected_control_and_send_dj_set() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // A filter (label a): three steps up, slow enough for no acceleration
        let mut t = select(&mut k, "a", t0);
        let mut got = Vec::new();
        for _ in 0..3 {
            tap_key(&mut k, ']', t);
            got.extend(sets(&k.tick(t)));
            t += ms(200);
        }
        assert_eq!(got, vec![("filter", 0, 0.05), ("filter", 0, 0.1), ("filter", 0, 0.15)]);
        tap_key(&mut k, '[', t);
        assert_eq!(sets(&k.tick(t)), vec![("filter", 0, 0.1)]);
        // reset
        tap_key(&mut k, '\\', t + ms(200));
        assert_eq!(sets(&k.tick(t + ms(200))), vec![("filter", 0, 0.0)]);
    }

    #[test]
    fn nothing_selected_says_so_instead_of_editing() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(k.on_key(&press(']'), t0), Some(Nav::NoSelection));
        assert_eq!(k.on_key(&press('\\'), t0), Some(Nav::NoSelection));
        assert!(k.tick(t0).is_empty());
        assert!(!k.wheel(1, t0));
        assert!(!k.press(t0));
    }

    #[test]
    fn steps_and_resets_per_control() {
        let t0 = Instant::now();
        // (label, one step up from the default fixture, reset value after moving)
        let cases: [(&str, &str, u32, f64, f64); 12] = [
            ("d", "low", 0, 0.05, 0.0),
            ("q", "low", 1, 0.05, 0.0),
            ("ta", "mid", 0, 0.05, 0.0),
            ("gs", "high", 1, 0.05, 0.0),
            ("a", "filter", 0, 0.05, 0.0),
            ("w", "crossfader", 0, -0.05, 0.0),
            ("xa", "rate", 0, 1.001, 1.0),
            ("zs", "gain", 1, 0.5, 0.0),
            ("ba", "master", 0, 0.05, 1.0),
            ("bs", "cue_mix", 0, 0.05, 0.0),
            ("bd", "sync_range", 0, 0.17, 0.16),
            ("e", "fader", 0, 0.0, 1.0),
        ];
        for (label, control, deck_no, up, reset) in cases {
            let mut k = DjKeys::new(true);
            // fixture: fader 0.5 so that a step is visible, crossfader +0.0, master 0.0
            let mut a = deck(false);
            a.fader = 0.5;
            let mut bb = deck(false);
            bb.fader = 0.5;
            k.set_snapshot(&[a, bb], &MixerInfo { crossfader: 0.0, master: 0.0, ..Default::default() });
            let t = select(&mut k, label, t0);
            let step_up = if control == "crossfader" { '[' } else { ']' };
            tap_key(&mut k, step_up, t);
            let first = sets(&k.tick(t));
            let want_up = if control == "fader" { 0.55 } else { up };
            let want_up = if control == "master" { 0.05 } else { want_up };
            assert_eq!(first.len(), 1, "{label}: {first:?}");
            assert_eq!((first[0].0, first[0].1), (control, deck_no), "{label}");
            assert!((first[0].2 - want_up).abs() < 1e-9, "{label}: {} vs {want_up}", first[0].2);
            tap_key(&mut k, '\\', t + ms(200));
            let back = sets(&k.tick(t + ms(200)));
            assert_eq!(back.len(), 1, "{label}");
            assert!((back[0].2 - reset).abs() < 1e-9, "{label} reset {}", back[0].2);
        }
    }

    #[test]
    fn pitch_steps_a_tenth_of_a_percent_and_clamps() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let mut t = select(&mut k, "xs", t0);
        for _ in 0..3 {
            tap_key(&mut k, '[', t);
            t += ms(200);
        }
        let v = sets(&k.tick(t)).pop().unwrap();
        assert_eq!((v.0, v.1), ("rate", 1));
        assert!((v.2 - 0.997).abs() < 1e-9, "{}", v.2);
    }

    #[test]
    fn gain_resets_to_the_auto_gain_of_the_track() {
        let t0 = Instant::now();
        let mut k = DjKeys::new(true);
        let mut a = deck(false);
        a.path = "/m/a.flac".into();
        a.gain_db = -2.5;
        k.set_snapshot(&[a.clone(), deck(false)], &MixerInfo::default());
        let mut t = select(&mut k, "za", t0);
        assert_eq!(k.gain_shown(0, t), -2.5);
        tap_key(&mut k, ']', t);
        assert_eq!(sets(&k.tick(t)), vec![("gain", 0, -2.0)]);
        // the daemon keeps reporting the auto-gain, the shown trim is the one sent
        t += ms(2000);
        k.set_snapshot(&[a.clone(), deck(false)], &MixerInfo::default());
        assert_eq!(k.gain_shown(0, t), -2.0);
        tap_key(&mut k, ']', t);
        assert_eq!(sets(&k.tick(t)), vec![("gain", 0, -1.5)]);
        tap_key(&mut k, '\\', t + ms(100));
        assert_eq!(sets(&k.tick(t + ms(100))), vec![("gain", 0, -2.5)]);
        // another track on the deck: its own auto-gain
        let mut other = a.clone();
        other.path = "/m/b.flac".into();
        other.gain_db = 1.0;
        k.set_snapshot(&[other, deck(false)], &MixerInfo::default());
        assert_eq!(k.gain_shown(0, t + ms(5000)), 1.0);
    }

    #[test]
    fn loop_and_jump_sizes_walk_the_ladder_and_feed_the_keys() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // loop length of A: 4 -> 8 -> 16, then v loops 16 beats
        let mut t = select(&mut k, "ca", t0);
        tap_key(&mut k, ']', t);
        t += ms(200);
        tap_key(&mut k, ']', t);
        t += ms(200);
        assert_eq!(k.loop_len(), [16.0, 4.0]);
        tap_key(&mut k, 'v', t);
        assert_eq!(k.tick(t), vec![DjCall::Loop { deck: 0, beats: 16.0 }]);
        // halve three times: 1/2; and never below 1/4
        for _ in 0..8 {
            t += ms(200);
            tap_key(&mut k, '[', t);
        }
        assert_eq!(k.loop_len()[0], 0.25);
        // deck B's jump size 2: u goes back 2, i forward 2
        let t = select(&mut k, "vs", t + ms(500));
        tap_key(&mut k, '[', t);
        tap_key(&mut k, 'u', t + ms(100));
        tap_key(&mut k, 'i', t + ms(200));
        tap_key(&mut k, 'e', t + ms(300));
        assert_eq!(
            k.tick(t + ms(400)),
            vec![
                DjCall::BeatJump { deck: 1, beats: -2.0 },
                DjCall::BeatJump { deck: 1, beats: 2.0 },
                DjCall::BeatJump { deck: 0, beats: -4.0 },
            ]
        );
        // reset: 4 beats again
        tap_key(&mut k, '\\', t + ms(500));
        assert_eq!(k.jump_len(), [4.0, 4.0]);
    }

    #[test]
    fn semicolon_toggles_between_the_last_two_selected() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let t = select(&mut k, "a", t0);
        let t = select(&mut k, "w", t);
        let f0 = Target::Slot(Slot::Ctl(Ctl::Filter(0)));
        let xf = Target::Slot(Slot::Ctl(Ctl::Crossfader));
        assert_eq!(k.hints().selected, Some(xf));
        tap_key(&mut k, ';', t);
        assert_eq!(k.hints().selected, Some(f0));
        tap_key(&mut k, ';', t + ms(100));
        assert_eq!(k.hints().selected, Some(xf));
        // editing hits whichever is current
        tap_key(&mut k, ']', t + ms(300));
        assert_eq!(sets(&k.tick(t + ms(300))), vec![("crossfader", 0, 0.05)]);
    }

    #[test]
    fn fast_detents_accelerate_and_slow_ones_do_not() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let t = select(&mut k, "a", t0);
        // five detents 20 ms apart: 1 + 2 + 4 + 4 + 4 = 15 steps of 0.05, capped by the range
        for i in 0..5 {
            assert!(k.wheel(1, t + ms(i * 20)));
        }
        let v = sets(&k.tick(t + ms(200))).pop().unwrap();
        assert!((v.2 - 0.75).abs() < 1e-9, "{}", v.2);
        // slow detents: one step each
        let mut k = keys(true);
        let t = select(&mut k, "a", t0);
        for i in 0..5 {
            k.wheel(1, t + ms(i * 200));
        }
        let v = sets(&k.tick(t + ms(1000))).pop().unwrap();
        assert!((v.2 - 0.25).abs() < 1e-9, "{}", v.2);
        // down is negative
        k.wheel(-1, t + ms(1100));
        let v = sets(&k.tick(t + ms(1100))).pop().unwrap();
        assert!((v.2 - 0.20).abs() < 1e-9, "{}", v.2);
    }

    #[test]
    fn wheel_routing() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // nothing selected: the caller scrolls the library
        assert!(!k.wheel(1, t0));
        assert!(k.tick(t0).is_empty());
        // hints up: swallowed, nothing happens
        let t = select(&mut k, "a", t0);
        tap_key(&mut k, 'f', t);
        assert!(k.wheel(1, t + ms(100)));
        assert!(k.tick(t + ms(100)).is_empty());
        // selected: edits
        k.on_key(&ev(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press), t + ms(200));
        assert!(k.wheel(1, t + ms(300)));
        assert_eq!(sets(&k.tick(t + ms(300))), vec![("filter", 0, 0.05)]);
        // the press resets
        assert!(k.press(t + ms(500)));
        assert_eq!(sets(&k.tick(t + ms(500))), vec![("filter", 0, 0.0)]);
    }

    #[test]
    fn opening_hints_ends_holds_so_no_release_is_lost() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('w'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Nudge { deck: 0, amount: 1.0 }]);
        tap_key(&mut k, 'f', t0 + ms(100));
        assert_eq!(k.tick(t0 + ms(100)), vec![DjCall::Nudge { deck: 0, amount: 0.0 }]);
        // the release arrives while the labels are up: harmless
        k.on_key(&rel('w'), t0 + ms(200));
        assert!(k.tick(t0 + ms(200)).is_empty());
    }

    #[test]
    fn hidden_controls_cannot_be_picked() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.set_visible(vec![Target::Slot(Slot::Ctl(Ctl::Filter(1)))]);
        let t = select(&mut k, "a", t0);
        assert!(k.hints().active, "a is not on screen: ignored");
        tap_key(&mut k, 's', t);
        assert_eq!(k.hints().selected, Some(Target::Slot(Slot::Ctl(Ctl::Filter(1)))));
    }
}
