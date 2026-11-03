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
use crate::dj_types::{DeckInfo, MixerInfo, PatchSlot};

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
/// The jog step, seconds a paused deck moves per detent (x2, x4, x8 spun fast): the `JOG`
/// control (`f` and its label, then the knob or `]` `[`) picks one of these
pub const JOG_STEPS: [f64; 8] = [0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0];
/// 0.5 s: a press moves far enough to hear the difference; `g[` gets finer
pub const DEFAULT_JOG_LEVEL: f64 = 4.0;
/// ... a playing deck bends by this nudge amount (-1..1 = -4..+4 %) while the knob turns
pub const JOG_BEND: f64 = 0.5;
/// detents closer than this count as spinning
const JOG_FAST: Duration = Duration::from_millis(60);
/// Detents closer than this are a spin of the picked clock (a hand spin is slower than a jog)
const SCRUB_FAST: Duration = Duration::from_millis(150);
/// a bend ends this long after the last detent
const JOG_RELEASE: Duration = Duration::from_millis(250);

/// The jog wheel's last detent
#[derive(Debug, Clone, Copy)]
struct Jog {
    deck: Deck,
    dir: i8,
    last: Instant,
    streak: u8,
    bending: bool,
}
/// Without release events: key events closer than this are auto-repeat of one key
const REPEAT_GAP: Duration = Duration::from_millis(120);
/// A just-moved control ignores polled values for this long (the poll may still be stale)
const SETTLE: Duration = Duration::from_millis(500);
/// Beats for beat jump and for the loop key
/// Tap step of the playback rate (0.1 %) and its hold speed, 1 %/s growing to 4 %/s
const RATE_STEP: f64 = 0.001;
const RATE_MIN: f64 = 0.5;
const RATE_MAX: f64 = 1.5;
/// A tap of `]` / `[` moves a patch knob by 1/20 of its range
const KNOB_STEPS: f64 = 20.0;

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
    /// the jog step: an index into `JOG_STEPS` (codj's own, never sent to codjd)
    Jog,
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
            Slot::Jog => (0.0, (JOG_STEPS.len() - 1) as f64),
        }
    }

    /// the step of a tap, of `]` / `[` and of one encoder detent
    pub fn tap_step(self) -> f64 {
        match self {
            Slot::Rate(_) => RATE_STEP,
            Slot::Gain(_) => 0.5,
            Slot::SyncRange => 0.01,
            Slot::Jog => 1.0,
            _ => 0.05,
        }
    }

    /// units per second at full ramp speed
    fn speed(self, ramp_s: f64) -> f64 {
        match self {
            Slot::Rate(_) => 0.01 + 0.03 * (ramp_s / 2.0).min(1.0),
            Slot::Gain(_) | Slot::Master | Slot::CueMix | Slot::SyncRange | Slot::Jog => 0.0,
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
            Slot::Jog => ("jog", 0),
        }
    }
}

/// dB as the multiplier `DjSet gain` takes: 0 dB = 1, -6 dB = 0.5
pub fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
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
}

/// Beats in a bar (codjd reports no time signature)
pub const BEATS_PER_BAR: f64 = 4.0;
/// Beats in a phrase: 16 bars
pub const PHRASE_BEATS: f64 = 16.0 * BEATS_PER_BAR;
/// `G` jumps to this many bars before the end (the outro)
pub const OUTRO_BARS: f64 = 32.0;
/// Without a beat grid `G` goes this many seconds before the end
const OUTRO_FALLBACK_S: f64 = 60.0;
/// Hot cue slots (marks `a` to `h`)
pub const MARKS: u8 = 8;
/// A count never grows past this
const COUNT_MAX: u32 = 9999;
/// Edits of one control closer than this are one undo step (a held ramp is one edit)
const EDIT_MERGE: Duration = Duration::from_millis(600);
/// Undo steps kept
const UNDO_MAX: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeekTo {
    /// the first downbeat (the start of the track without a grid)
    Start,
    /// 32 bars before the end
    Outro,
}

/// An action on the focused deck. These are what `.` repeats (with the count they had).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Act {
    PlayPause,
    Sync,
    /// beats per count: `w` 1, `W` 4 (a bar), `}` 64 (a phrase); negative goes back
    Jump(f64),
    Seek(SeekTo),
    /// `v`: start a loop (count beats, else the loop length), resize it with a count, or leave it
    Loop,
    /// `m{a-h}`: save a hot cue at the playhead
    Mark(u8),
    /// `'{a-h}`: jump to a hot cue
    GoMark(u8),
    /// `+` / `-`: pitch by 0.1 % per count
    Pitch(i8),
    /// `=`: pitch back to 0
    PitchReset,
    /// `R`: the deck's strip back to its defaults: EQ flat, filter off, fader up, pitch 0
    ResetDeck,
}

/// A command that waits for one more key
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pending {
    /// `m`
    Mark,
    /// `'` or `` ` ``
    GoMark,
    /// `g`
    G,
    /// `t` (at the next phrase line) or `T` (at the outro): a transition, its technique next
    T { outro: bool },
}

impl Pending {
    fn key(self) -> char {
        match self {
            Pending::Mark => 'm',
            Pending::GoMark => '\'',
            Pending::G => 'g',
            Pending::T { outro: false } => 't',
            Pending::T { outro: true } => 'T',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tap {
    Headphones(Deck),
    Quantize,
}

/// Keys the screen itself handles
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Nav {
    Command,
    /// `Tab`: the library, browsed with vim keys
    Search,
    /// `/`: straight into the library search
    SearchNow,
    LibUp,
    LibDown,
    LibPageUp,
    LibPageDown,
    /// load the highlighted track onto the focused deck (the other one if it plays)
    LoadSmart,
    /// `a` / `b`: the highlighted library track onto deck A / B
    Load(u8),
    /// `p`: hear the focused deck in the headphones only, to find the spot by ear
    Preview,
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
    /// the patch pane (F12)
    Patch,
    /// `f`: show the control hints
    Hints,
    /// `;`: back to the previously selected control
    Toggle,
    /// `\`: reset the selected control
    Reset,
    /// an edit key with no control selected (tells the user how to select one)
    NoSelection,
    /// `?`: the key map on the screen, every key explained
    Help,
    /// `gt`: automix from the other (playing) deck into the focused one, like vim's next tab
    AutoMix,
    /// `q`: the highlighted library track to the player's queue
    Queue,
    /// `t{technique}` / `T{technique}`: the DJ's transition into the focused deck: a recipe
    /// (None: codj chooses), a length in bars (the count), at the outro instead of the next phrase
    Transition { recipe: Option<&'static str>, bars: Option<u32>, outro: bool },
    /// a one-line message for the status line
    Note(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Binding {
    /// an action on the focused deck
    Act(Act),
    /// `c`: cue, hold to preview
    Cue,
    /// `]` / `[`: step the selected control, else nudge the focused deck
    Bracket(i8),
    /// `+` / `-`: pitch of the focused deck (a hold ramps)
    Pitch(i8),
    /// `h` / `l`
    Focus(Deck),
    /// a digit of a count (`0` only after another digit; alone it is the start motion)
    Digit(u8),
    Pend(Pending),
    /// `.`
    Repeat,
    Undo,
    Redo,
    Tap(Tap),
    Nav(Nav),
}

/// A key with the terminal's shift quirks removed: letters are lowercase and `shift` is set for
/// capitals and shifted US symbols (`}` is `]` + shift), so every protocol mode looks the same
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
        '+' => ('=', true),
        '{' => ('[', true),
        '}' => (']', true),
        '_' => ('-', true),
        '"' => ('\'', true),
        '~' => ('`', true),
        '|' => ('\\', true),
        c => (c, false),
    }
}

/// The key map, vim first: one deck has the focus (`h` / `l`), counts go in front of motions,
/// marks are hot cues, `v` is a loop. Mixer values are edited through the hints (`f`) and `:`
/// commands.
pub fn bind(k: &NKey) -> Option<Binding> {
    use Binding::*;
    if k.alt {
        return None;
    }
    if k.ctrl {
        return match (k.code, k.shift) {
            (KeyCode::Char('r'), false) => Some(Redo),
            _ => None,
        };
    }
    let KeyCode::Char(c) = k.code else {
        return match (k.code, k.shift) {
            (KeyCode::F(1), false) => Some(Tap(self::Tap::Headphones(0))),
            (KeyCode::F(2), false) => Some(Tap(self::Tap::Headphones(1))),
            (KeyCode::F(5), false) => Some(Tap(self::Tap::Quantize)),
            (KeyCode::F(6), false) => Some(Nav(self::Nav::SetArmNext)),
            (KeyCode::F(7), false) => Some(Nav(self::Nav::SetAuto)),
            (KeyCode::F(8), false) => Some(Nav(self::Nav::SetDisarm)),
            (KeyCode::F(9), false) => Some(Nav(self::Nav::RecordToggle)),
            (KeyCode::F(10), false) => Some(Nav(self::Nav::Editor)),
            (KeyCode::F(11), false) => Some(Nav(self::Nav::Prompt)),
            (KeyCode::F(12), false) => Some(Nav(self::Nav::Patch)),
            (KeyCode::Tab, _) => Some(Nav(self::Nav::Search)),
            (KeyCode::Left, false) => Some(Focus(0)),
            (KeyCode::Right, false) => Some(Focus(1)),
            (KeyCode::Up, false) => Some(Nav(self::Nav::LibUp)),
            (KeyCode::Down, false) => Some(Nav(self::Nav::LibDown)),
            (KeyCode::PageUp, false) => Some(Nav(self::Nav::LibPageUp)),
            (KeyCode::PageDown, false) => Some(Nav(self::Nav::LibPageDown)),
            (KeyCode::Enter, false) => Some(Nav(self::Nav::LoadSmart)),
            _ => None,
        };
    };
    use self::{Act as A, Nav as N};
    if k.shift {
        return match c {
            // vim's WORD is the bigger word: a bar; G the end; } a paragraph: a phrase
            'w' => Some(Act(A::Jump(BEATS_PER_BAR))),
            'b' => Some(Act(A::Jump(-BEATS_PER_BAR))),
            ']' => Some(Act(A::Jump(PHRASE_BEATS))),
            '[' => Some(Act(A::Jump(-PHRASE_BEATS))),
            'g' | '4' => Some(Act(A::Seek(SeekTo::Outro))),
            't' => Some(Pend(Pending::T { outro: true })),
            '=' => Some(Pitch(1)),
            'r' => Some(Act(A::ResetDeck)),
            '/' => Some(Nav(N::Help)),
            ';' => Some(Nav(N::Command)),
            _ => None,
        };
    }
    Some(match c {
        'h' => Focus(0),
        'l' => Focus(1),
        ' ' => Act(A::PlayPause),
        'c' => Cue,
        's' => Act(A::Sync),
        'w' => Act(A::Jump(1.0)),
        // the decks are A and B: their letters load them, from anywhere
        'a' => Nav(N::Load(0)),
        'b' => Nav(N::Load(1)),
        'p' => Nav(N::Preview),
        'v' => Act(A::Loop),
        '0'..='9' => Digit(c as u8 - b'0'),
        'g' => Pend(Pending::G),
        't' => Pend(Pending::T { outro: false }),
        // the library is always under the hand: no Tab to pick the next track
        'j' => Nav(N::LibDown),
        'k' => Nav(N::LibUp),
        'q' => Nav(N::Queue),
        'm' => Pend(Pending::Mark),
        '\'' | '`' => Pend(Pending::GoMark),
        '-' => Pitch(-1),
        '=' => Act(A::PitchReset),
        '.' => Repeat,
        'u' => Undo,
        '/' => Nav(N::SearchNow),
        // control hints: `f` shows labels, a label selects, `]` `[` `\` edit, `;` toggles
        'f' => Nav(N::Hints),
        ';' => Nav(N::Toggle),
        ']' => Bracket(1),
        '[' => Bracket(-1),
        '\\' => Nav(N::Reset),
        _ => return None,
    })
}

/// The length a digit in the `t` menu picks, in bars: the steps 4 8 16 32
pub fn step_bars(c: char) -> Option<u32> {
    Some(match c {
        '1' => 4,
        '2' => 8,
        '3' => 16,
        '4' => 32,
        _ => return None,
    })
}

/// The `t` menu: one line per technique, then the length steps
pub fn technique_menu(outro: bool, bars: Option<u32>) -> (String, Vec<String>) {
    let when = if outro { "at the outro" } else { "at the next phrase" };
    let len = bars.map_or("its own length".to_string(), |b| format!("{b} bars"));
    let title = format!(" transition {when} · {len} · Esc cancels ");
    let mut lines: Vec<String> = [
        ('b', "blend", "both play, faders cross"),
        ('s', "bass swap", "the low end changes hands on a bar line"),
        ('e', "EQ swap", "highs, mids, lows hand over one by one"),
        ('f', "filter blend", "one rises out of a filter, the other sinks"),
        ('o', "echo-out", "the old one echoes away, the new one drops"),
        ('r', "loop roll", "the old one rolls tighter, then the new one lands"),
        ('c', "cut", "straight in on the downbeat"),
        ('k', "brake", "the old one spins down"),
        ('t', "codj picks", "the technique that fits these two tracks"),
    ]
    .iter()
    .map(|(k, name, what)| format!("{k}  {name} · {what}"))
    .collect();
    lines.push(String::new());
    lines.push("length: 1 = 4 bars · 2 = 8 · 3 = 16 · 4 = 32, then the letter (t2s: bass swap over 8)".into());
    (title, lines)
}

/// The technique a letter after `t` names: `Some(None)` is `tt`, codj's own choice
pub fn technique(c: char) -> Option<Option<&'static str>> {
    Some(match c {
        'b' => Some("blend"),
        's' => Some("bassswap"),
        'e' => Some("eq-swap"),
        'f' => Some("filter-blend"),
        'o' => Some("echo-out"),
        'r' => Some("loop-roll"),
        'c' => Some("cut"),
        'k' => Some("brake"),
        't' => None,
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
    /// `DjSeek`: jump to a time (not beat-phase preserving, unlike a beat jump)
    Seek { deck: u32, seconds: f64 },
    Nudge { deck: u32, amount: f64 },
    BeatJump { deck: u32, beats: f64 },
    Loop { deck: u32, beats: f64 },
    Hotcue { deck: u32, slot: u32, action: &'static str },
    Set { control: &'static str, deck: u32, value: f64 },
    /// a patch knob: `PatchAction {"op":"knob",...}` on the slot
    PatchKnob { slot: u32, name: String, value: f64 },
}

impl DjCall {
    pub fn name(&self) -> &'static str {
        match self {
            DjCall::Load { .. } => "DjLoad",
            DjCall::Play(_) => "DjPlay",
            DjCall::Pause(_) => "DjPause",
            DjCall::Cue { .. } => "DjCue",
            DjCall::Sync(_) => "DjSync",
            DjCall::Seek { .. } => "DjSeek",
            DjCall::Nudge { .. } => "DjNudge",
            DjCall::BeatJump { .. } => "DjBeatJump",
            DjCall::Loop { .. } => "DjLoop",
            DjCall::Hotcue { .. } => "DjHotcue",
            DjCall::Set { .. } => "DjSet",
            DjCall::PatchKnob { .. } => "PatchAction",
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
        (DjCall::PatchKnob { slot: s1, name: n1, .. }, DjCall::PatchKnob { slot: s2, name: n2, .. }) => s1 == s2 && n1 == n2,
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

/// One undoable change of a control: it went from `before` to `after`
#[derive(Debug, Clone, Copy)]
struct Edit {
    slot: Slot,
    before: f64,
    after: f64,
    at: Instant,
}

struct SlotState {
    slot: Slot,
    value: f64,
    touched: Instant,
    dirty: bool,
}

/// A patch knob being moved from here
struct KnobState {
    slot: u8,
    name: String,
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
    jog: Option<Jog>,
    /// the `JOG` control: an index into `JOG_STEPS`
    jog_level: f64,
    /// without release events: cue preview is on for this deck
    cue_preview: [bool; 2],
    last_event: Option<(KeyCode, Instant)>,
    last_flush: Option<Instant>,
    /// the control hints and the selection
    hints: Hints,
    accel: Accel,
    /// controls the screen shows (None until the first frame: all of them)
    visible: Option<Vec<Target>>,
    /// the deck every key acts on (`h` / `l`)
    focus: Deck,
    /// the count typed before a motion (`4w`)
    count: Option<u32>,
    pending: Option<Pending>,
    /// what `.` repeats, with its count
    last: Option<(Act, Option<u32>)>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// seconds of the first downbeat per deck, when the grid is known (`0` goes there)
    starts: [Option<f64>; 2],
    /// beats of the next loop (`v` without a count), per deck
    loop_len: [f64; 2],
    sync_range: f64,
    /// the last trim sent per deck with the path it was sent for (codjd reports the auto-gain
    /// only, not a manual trim)
    gain_set: [Option<(String, f64)>; 2],
    /// the auto-gain a track came with, per deck and path: what `\` goes back to
    auto_gain: [Option<(String, f64)>; 2],
    /// the patch slots as last polled, and the knob values their files have (what `\` returns to)
    patches: Vec<PatchSlot>,
    knob_defaults: [Vec<(String, f64)>; 4],
    knob_moves: Vec<KnobState>,
    /// filter / low of a patch slot are not reported: the last value sent from here
    patch_mix_sent: Vec<(Slot, f64)>,
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
            focus: 0,
            count: None,
            pending: None,
            jog: None,
            jog_level: DEFAULT_JOG_LEVEL,
            last: None,
            undo: Vec::new(),
            redo: Vec::new(),
            starts: [None; 2],
            loop_len: [hints::DEFAULT_BEATS; 2],
            sync_range: hints::DEFAULT_SYNC_RANGE,
            gain_set: [None, None],
            auto_gain: [None, None],
            patches: Vec::new(),
            knob_defaults: Default::default(),
            knob_moves: Vec::new(),
            patch_mix_sent: Vec::new(),
        }
    }

    /// The patch slots as last polled, and the knob values of their files
    pub fn set_patches(&mut self, slots: &[PatchSlot], defaults: [Vec<(String, f64)>; 4]) {
        self.patches = slots.to_vec();
        self.knob_defaults = defaults;
    }

    /// The knob a `Target::Knob` points at: (name, value, min, max)
    fn knob_info(&self, slot: u8, idx: u8) -> Option<(&str, f64, f64, f64)> {
        let k = self.patches.get(slot as usize)?.knobs.get(idx as usize)?;
        Some((k.0.as_str(), k.1, k.2, k.3))
    }

    /// Where a knob is going: our own latest value while it is being moved, else the daemon's
    fn knob_target(&self, slot: u8, idx: u8, now: Instant) -> f64 {
        let Some((name, value, ..)) = self.knob_info(slot, idx) else { return 0.0 };
        match self.knob_moves.iter().find(|k| k.slot == slot && k.name == name) {
            Some(k) if k.dirty || now.saturating_duration_since(k.touched) < SETTLE => k.value,
            _ => value,
        }
    }

    fn bump_knob(&mut self, slot: u8, idx: u8, delta: f64, now: Instant) {
        let Some((name, _, lo, hi)) = self.knob_info(slot, idx).map(|(n, v, a, b)| (n.to_string(), v, a, b)) else { return };
        let before = self.knob_target(slot, idx, now);
        let value = ((before + delta).clamp(lo, hi) * 1e4).round() / 1e4;
        let changed = (value - before).abs() > 1e-9;
        match self.knob_moves.iter_mut().find(|k| k.slot == slot && k.name == name) {
            Some(k) => {
                k.value = value;
                k.touched = now;
                k.dirty |= changed;
            }
            None if changed => self.knob_moves.push(KnobState { slot, name, value, touched: now, dirty: true }),
            None => {}
        }
    }

    /// "P1 cutoff" for a knob, else the target's own name
    pub fn target_name(&self, t: Target) -> String {
        match t {
            Target::Knob(s, i) => match self.knob_info(s, i) {
                Some((name, ..)) => format!("P{} {name}", s + 1),
                None => t.name(),
            },
            _ => t.name(),
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
        // the mixer controls of a patch slot (deck 2 + slot): the fader is reported, the rest not
        if let Slot::Ctl(Ctl::Fader(d) | Ctl::Filter(d) | Ctl::Eq(d, _)) = slot {
            if d >= 2 {
                return match slot {
                    Slot::Ctl(Ctl::Fader(_)) => self.patches.get(d as usize - 2).map_or(0.0, |p| p.fader),
                    _ => self.patch_mix_sent.iter().find(|(s, _)| *s == slot).map_or(0.0, |(_, v)| *v),
                };
            }
        }
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
            Slot::Jog => self.jog_level,
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
            Slot::Jog => DEFAULT_JOG_LEVEL,
        }
    }

    /// The value a target has now, for the status chip
    pub fn value_of(&self, t: Target, now: Instant) -> f64 {
        match t {
            Target::Slot(s) => self.target(s, now),
            Target::LoopLen(d) => self.loop_len[d as usize],
            Target::Time(d) => self.decks.get(d as usize).map_or(0.0, |x| x.position),
            Target::Knob(s, i) => self.knob_target(s, i, now),
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
            Target::Time(d) => self.scrub(d, n.signum(), now),
            Target::Knob(s, i) => {
                let (lo, hi) = self.knob_info(s, i).map_or((0.0, 1.0), |k| (k.2, k.3));
                self.bump_knob(s, i, n as f64 * (hi - lo) / KNOB_STEPS, now);
            }
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
            Target::Time(_) => {}
            Target::Knob(s, i) => {
                // the value the patch file has; a knob the file does not list goes to its minimum
                let home = self.knob_info(s, i).map(|(name, _, lo, _)| {
                    self.knob_defaults[s as usize].iter().find(|(n, _)| n == name).map_or(lo, |(_, v)| *v)
                });
                if let Some(home) = home {
                    let delta = home - self.knob_target(s, i, now);
                    self.bump_knob(s, i, delta, now);
                }
            }
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

    /// Moves `deck` through its track, playing or not (its time picked with `f`): a slow turn a
    /// bar, spinning 2, 4, 8 and 16 bars a detent, so a spin crosses minutes and slowing down
    /// lands on a bar line (2 s steps for a track without a grid)
    pub fn scrub(&mut self, deck: Deck, dir: i32, now: Instant) {
        let Some(d) = self.decks.get(deck as usize).filter(|d| d.loaded) else { return };
        let (gridded, position, bpm) = (d.has_grid(), d.position, d.track_bpm);
        let dir = dir.signum() as i8;
        let prev = self.jog.filter(|j| j.deck == deck && j.dir == dir && !j.bending);
        let streak = prev.filter(|j| now.saturating_duration_since(j.last) < SCRUB_FAST).map_or(0, |j| j.streak.saturating_add(1));
        let mult = dir as f64 * f64::from(1u8 << streak.min(4));
        if gridded && bpm > 0.0 {
            self.out.push(DjCall::BeatJump { deck: deck as u32, beats: mult * BEATS_PER_BAR });
        } else {
            self.out.push(DjCall::Seek { deck: deck as u32, seconds: (position + mult * 2.0).max(0.0) });
        }
        self.jog = Some(Jog { deck, dir, last: now, streak, bending: false });
    }

    /// The knob as the focused deck's jog wheel, like a record under the hand: a paused deck
    /// scrubs (finding a spot by ear), a playing one bends its tempo while the knob turns
    pub fn jog(&mut self, dir: i32, now: Instant) {
        let deck = self.focus;
        let Some(d) = self.decks.get(deck as usize).filter(|d| d.loaded) else { return };
        let (playing, gridded, position, bpm) = (d.playing, d.has_grid(), d.position, d.track_bpm);
        let dir = dir.signum() as i8;
        let prev = self.jog.filter(|j| j.deck == deck && j.dir == dir);
        let streak = prev.filter(|j| now.saturating_duration_since(j.last) < JOG_FAST).map_or(0, |j| j.streak.saturating_add(1));
        if playing {
            if !prev.is_some_and(|j| j.bending) {
                self.out.push(DjCall::Nudge { deck: deck as u32, amount: dir as f64 * JOG_BEND });
            }
        } else {
            let mult = dir as f64 * f64::from(1u8 << streak.min(3));
            // a beat jump is relative, so detents faster than the 30 Hz snapshot all count
            let seconds = mult * self.jog_seconds();
            if gridded && bpm > 0.0 {
                // seconds of the track, as beats of its own grid
                self.out.push(DjCall::BeatJump { deck: deck as u32, beats: seconds * bpm / 60.0 });
            } else {
                self.out.push(DjCall::Seek { deck: deck as u32, seconds: (position + seconds).max(0.0) });
            }
        }
        // a bend the other way replaces the old one; the release in `tick` ends it
        self.jog = Some(Jog { deck, dir, last: now, streak, bending: playing });
    }

    /// `[knob] jog`: where the jog step starts, the step nearest to `seconds` (None: 0.5 s)
    pub fn set_jog_seconds(&mut self, seconds: Option<f64>) {
        self.jog_level = match seconds.filter(|s| s.is_finite() && *s > 0.0) {
            Some(s) => {
                let off = |i: usize| (JOG_STEPS[i].ln() - s.ln()).abs();
                (0..JOG_STEPS.len()).min_by(|&a, &b| off(a).total_cmp(&off(b))).unwrap_or(2) as f64
            }
            None => DEFAULT_JOG_LEVEL,
        };
    }

    /// Seconds a paused deck moves per jog detent
    pub fn jog_seconds(&self) -> f64 {
        JOG_STEPS[(self.jog_level as usize).min(JOG_STEPS.len() - 1)]
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

    /// Moves a control by `delta` and remembers the change for `u`
    fn bump(&mut self, slot: Slot, delta: f64, now: Instant) {
        let (before, after) = self.apply(slot, delta, now, false);
        if (after - before).abs() > 1e-9 {
            self.record(Edit { slot, before, after, at: now });
        }
    }

    /// A change by hand: edits of one control that follow each other are one undo step
    fn record(&mut self, e: Edit) {
        self.redo.clear();
        if let Some(top) = self.undo.last_mut() {
            if top.slot == e.slot && (top.after - e.before).abs() < 1e-9 && e.at.saturating_duration_since(top.at) < EDIT_MERGE {
                top.after = e.after;
                top.at = e.at;
                return;
            }
        }
        self.undo.push(e);
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
    }

    /// `u`: the last change of a control goes back
    fn undo(&mut self, now: Instant) -> Option<Nav> {
        let Some(e) = self.undo.pop() else { return Some(Nav::Note("nothing to undo")) };
        self.restore(e.slot, e.before, now);
        self.redo.push(Edit { at: now, ..e });
        None
    }

    /// `Ctrl-r`: what `u` took back is done again
    fn redo(&mut self, now: Instant) -> Option<Nav> {
        let Some(e) = self.redo.pop() else { return Some(Nav::Note("nothing to redo")) };
        self.restore(e.slot, e.after, now);
        self.undo.push(Edit { at: now, ..e });
        None
    }

    /// Sets a control to `value` (a `:` command); undoable
    pub fn set_control(&mut self, slot: Slot, value: f64, now: Instant) {
        let delta = value - self.target(slot, now);
        self.bump(slot, delta, now);
    }

    /// Puts a control at a value it had, and sends it even when the last poll still shows it
    fn restore(&mut self, slot: Slot, value: f64, now: Instant) {
        let delta = value - self.target(slot, now);
        self.apply(slot, delta, now, true);
    }

    /// Moves a control and returns its value before and after (clamped to its range). `force`
    /// sends the value even if it did not change.
    fn apply(&mut self, slot: Slot, delta: f64, now: Instant, force: bool) -> (f64, f64) {
        let (lo, hi) = slot.range();
        let before = self.target(slot, now);
        let value = ((before + delta).clamp(lo, hi) * 1e4).round() / 1e4;
        let changed = force || (value - before).abs() > 1e-9;
        match slot {
            Slot::SyncRange => self.sync_range = value,
            Slot::Jog => self.jog_level = value.round(),
            Slot::Gain(d) => {
                if let Some(deck) = self.decks.get(d as usize) {
                    self.gain_set[d as usize] = Some((deck.path.clone(), value));
                }
            }
            Slot::Ctl(Ctl::Filter(d) | Ctl::Eq(d, _)) if d >= 2 => {
                self.patch_mix_sent.retain(|(s, _)| *s != slot);
                self.patch_mix_sent.push((slot, value));
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
        (before, value)
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
        let again = repeat || stream;
        // Esc steps back and never leaves: a half-typed command, the count, then the selection
        if nk.code == KeyCode::Esc && !nk.alt && !nk.ctrl {
            if !again && (self.pending.take().is_some() | self.count.take().is_some()) {
                return None;
            }
            if !again && self.hints.selected.is_some() {
                self.hints.clear();
            }
            return None;
        }
        // the second key of `m` `'` `g`
        if let Some(p) = self.pending {
            if again {
                return None;
            }
            self.pending = None;
            let count = self.count.take();
            return self.pending_key(p, &nk, count, now);
        }
        let Some(b) = bind(&nk) else {
            // a key that means nothing cancels the count, like in vim
            self.count = None;
            return None;
        };
        match b {
            Binding::Digit(d) => {
                if again {
                    return None;
                }
                if d == 0 && self.count.is_none() {
                    // `0` alone: the start of the track
                    return self.run(Act::Seek(SeekTo::Start), None, now, true);
                }
                self.count = Some((self.count.unwrap_or(0) * 10 + d as u32).min(COUNT_MAX));
                None
            }
            Binding::Pend(p) => {
                if !again {
                    self.pending = Some(p);
                }
                None
            }
            Binding::Focus(d) => {
                if !again {
                    self.focus = d;
                }
                self.count = None;
                None
            }
            Binding::Act(a) => {
                let count = self.count.take();
                if again {
                    return None;
                }
                self.run(a, count, now, true)
            }
            Binding::Repeat => {
                let count = self.count.take();
                if again {
                    return None;
                }
                match self.last {
                    // a count replaces the old one for good, like in vim
                    Some((a, c)) => {
                        let count = count.or(c);
                        self.last = Some((a, count));
                        self.run(a, count, now, false)
                    }
                    None => Some(Nav::Note("nothing to repeat")),
                }
            }
            Binding::Undo => {
                self.count = None;
                if again {
                    return None;
                }
                self.undo(now)
            }
            Binding::Redo => {
                self.count = None;
                if again {
                    return None;
                }
                self.redo(now)
            }
            Binding::Cue => {
                self.count = None;
                self.press_hold(Hold::Cue(self.focus), &nk, repeat, stream, now);
                None
            }
            Binding::Pitch(dir) => {
                let count = self.count.take();
                if !again {
                    self.last = Some((Act::Pitch(dir), count));
                }
                match count {
                    // a hold ramps the pitch
                    None => self.press_hold(Hold::Rate(self.focus, dir), &nk, repeat, stream, now),
                    Some(n) if !again => self.step_n(Slot::Rate(self.focus), dir, n, now),
                    Some(_) => {}
                }
                None
            }
            Binding::Bracket(dir) => {
                self.count = None;
                // a picked control is edited (a held key keeps stepping and speeds up through the
                // acceleration); with none picked the keys bend the focused deck's tempo
                // a paused deck has no tempo to bend: the keys scrub it like the jog wheel (a held
                // key auto-repeats, which spins it faster)
                let paused = self.decks.get(self.focus as usize).is_some_and(|i| i.loaded && !i.playing);
                if self.hints.selected.is_some() {
                    self.edit(dir as i32, now);
                } else if paused {
                    self.jog(dir as i32, now);
                } else {
                    self.press_hold(Hold::Nudge(self.focus, dir), &nk, repeat, stream, now);
                }
                None
            }
            Binding::Nav(Nav::Hints) => {
                self.count = None;
                if !again {
                    self.open_hints();
                }
                None
            }
            Binding::Nav(Nav::Toggle) => {
                self.count = None;
                if !again {
                    self.hints.toggle();
                }
                None
            }
            Binding::Nav(Nav::Reset) => {
                self.count = None;
                if again || self.reset(now) {
                    None
                } else {
                    Some(Nav::NoSelection)
                }
            }
            Binding::Nav(n) => {
                self.count = None;
                let scrolls = matches!(n, Nav::LibUp | Nav::LibDown | Nav::LibPageUp | Nav::LibPageDown);
                (scrolls || !again).then_some(n)
            }
            Binding::Tap(t) => {
                self.count = None;
                if !again {
                    self.tap(t);
                }
                None
            }
        }
    }

    /// The key after `m`, `'` or `g`
    fn pending_key(&mut self, p: Pending, nk: &NKey, count: Option<u32>, now: Instant) -> Option<Nav> {
        let KeyCode::Char(c) = nk.code else { return None };
        if nk.alt || nk.ctrl || nk.shift {
            return None;
        }
        match p {
            Pending::Mark | Pending::GoMark => {
                if !('a'..='h').contains(&c) {
                    return Some(Nav::Note("marks are a to h (eight hot cues)"));
                }
                let slot = c as u8 - b'a';
                self.run(if p == Pending::Mark { Act::Mark(slot) } else { Act::GoMark(slot) }, None, now, true)
            }
            Pending::G => match c {
                'g' => self.run(Act::Seek(SeekTo::Start), count, now, true),
                't' => Some(Nav::AutoMix),
                // `ge`: back a beat (vim's back to the end of a word; `b` loads deck B)
                'e' => self.run(Act::Jump(-1.0), count, now, true),
                // the jog step (`[` `]` on a paused deck, the knob as jog wheel): finer, coarser
                '[' | ']' => {
                    let d = if c == ']' { 1. } else { -1. };
                    self.jog_level = (self.jog_level + d).clamp(0., (JOG_STEPS.len() - 1) as f64);
                    None
                }
                _ => None,
            },
            // a digit in the menu is a length step; the menu stays up for the technique
            Pending::T { outro } if step_bars(c).is_some() => {
                self.pending = Some(Pending::T { outro });
                self.count = step_bars(c);
                None
            }
            Pending::T { outro } => match technique(c) {
                Some(recipe) => Some(Nav::Transition { recipe, bars: count, outro }),
                None => Some(Nav::Note("transitions: tb blend · ts bass swap · te EQ swap · tf filter · to echo-out · tr roll · tc cut · tk brake · tt codj picks")),
            },
        }
    }

    /// What is typed so far, like vim's showcmd: the count and the half-typed command
    pub fn showcmd(&self) -> String {
        let mut s = self.count.map(|n| n.to_string()).unwrap_or_default();
        if let Some(p) = self.pending {
            s.push(p.key());
        }
        s
    }

    /// The deck every key acts on
    pub fn focus(&self) -> Deck {
        self.focus
    }

    /// Focuses a deck (the `:focus` command)
    pub fn set_focus(&mut self, deck: Deck) {
        self.focus = deck.min(1);
        self.count = None;
        self.pending = None;
    }

    /// The first downbeats of the loaded tracks (seconds), for `0`
    pub fn set_starts(&mut self, starts: [Option<f64>; 2]) {
        self.starts = starts;
    }

    /// Something half-typed that Esc would cancel
    /// The command waiting for its next key, and the count typed before it
    pub fn pending(&self) -> Option<(Pending, Option<u32>)> {
        self.pending.map(|p| (p, self.count))
    }

    pub fn has_pending(&self) -> bool {
        self.count.is_some() || self.pending.is_some()
    }

    fn step_n(&mut self, slot: Slot, dir: i8, n: u32, now: Instant) {
        self.bump(slot, dir as f64 * n as f64 * slot.tap_step(), now);
    }

    /// Runs an action on the focused deck. `remember`: `.` repeats it.
    fn run(&mut self, act: Act, count: Option<u32>, now: Instant, remember: bool) -> Option<Nav> {
        if remember {
            self.last = Some((act, count));
        }
        let d = self.focus;
        let deck = d as u32;
        let info = self.decks.get(d as usize);
        let n = count.unwrap_or(1) as f64;
        match act {
            Act::PlayPause => {
                let playing = info.is_some_and(|i| i.playing);
                self.out.push(if playing { DjCall::Pause(deck) } else { DjCall::Play(deck) });
            }
            Act::Sync => self.out.push(DjCall::Sync(deck)),
            Act::Jump(unit) => self.out.push(DjCall::BeatJump { deck, beats: unit * n }),
            Act::Seek(to) => {
                let Some(i) = info.filter(|i| i.loaded) else { return Some(Nav::Note("no track on this deck")) };
                let seconds = match to {
                    SeekTo::Start => self.starts[d as usize].unwrap_or(0.0),
                    SeekTo::Outro => {
                        let back = if i.track_bpm > 0.0 { OUTRO_BARS * BEATS_PER_BAR * 60.0 / i.track_bpm } else { OUTRO_FALLBACK_S };
                        (i.duration - back).max(0.0)
                    }
                };
                self.out.push(DjCall::Seek { deck, seconds });
            }
            Act::Loop => {
                let looping = info.is_some_and(|i| i.loop_beats > 0.0);
                let beats = match (looping, count) {
                    (false, None) => self.loop_len[d as usize],
                    (_, Some(c)) => c as f64,
                    (true, None) => 0.0,
                };
                self.out.push(DjCall::Loop { deck, beats });
            }
            Act::Mark(slot) => self.out.push(DjCall::Hotcue { deck, slot: slot as u32, action: "set" }),
            Act::GoMark(slot) => {
                if info.is_some_and(|i| i.loaded && i.hotcue(slot as usize).is_none()) {
                    return Some(Nav::Note("that mark is not set: m then a to h saves one"));
                }
                self.out.push(DjCall::Hotcue { deck, slot: slot as u32, action: "jump" });
            }
            Act::Pitch(dir) => self.step_n(Slot::Rate(d), dir, count.unwrap_or(1), now),
            Act::PitchReset => {
                let delta = 1.0 - self.target(Slot::Rate(d), now);
                self.bump(Slot::Rate(d), delta, now);
            }
            Act::ResetDeck => {
                let home = [
                    (Slot::Ctl(Ctl::Eq(d, 0)), 0.0),
                    (Slot::Ctl(Ctl::Eq(d, 1)), 0.0),
                    (Slot::Ctl(Ctl::Eq(d, 2)), 0.0),
                    (Slot::Ctl(Ctl::Filter(d)), 0.0),
                    (Slot::Ctl(Ctl::Fader(d)), 1.0),
                    (Slot::Rate(d), 1.0),
                ];
                for (slot, value) in home {
                    let delta = value - self.target(slot, now);
                    if delta.abs() > 1e-9 {
                        self.bump(slot, delta, now);
                    }
                }
            }
        }
        None
    }

    fn tap(&mut self, t: Tap) {
        let calls = &mut self.out;
        match t {
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
            Hold::Rate(..) => {}
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
        // the jog bend ends once the knob stops
        if let Some(j) = self.jog.filter(|j| j.bending && now.saturating_duration_since(j.last) > JOG_RELEASE) {
            self.out.push(DjCall::Nudge { deck: j.deck as u32, amount: 0.0 });
            self.jog = None;
        }
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
                    // the jog step is codj's own
                    if s.slot == Slot::Jog {
                        continue;
                    }
                    let (control, deck) = s.slot.control();
                    // the screen's gain is the deck's level in dB; codjd multiplies its `gain` (1 =
                    // as is) with the track's auto-gain trim, which `gain_db` reports: send the step
                    // from that trim as a multiplier
                    let value = match s.slot {
                        Slot::Gain(d) => db_to_gain(s.value - self.decks.get(d as usize).map_or(0.0, |i| i.gain_db)),
                        _ => s.value,
                    };
                    calls.push(DjCall::Set { control, deck, value });
                    sent = true;
                }
            }
            for k in &mut self.knob_moves {
                if k.dirty {
                    k.dirty = false;
                    calls.push(DjCall::PatchKnob { slot: k.slot as u32, name: k.name.clone(), value: k.value });
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

    fn slot_with_knobs() -> PatchSlot {
        PatchSlot {
            loaded: true,
            name: "acid".into(),
            knobs: vec![("cutoff".into(), 0.3, 0.0, 1.0), ("reso".into(), 0.5, 0.0, 0.8)],
            fader: 0.6,
            ..Default::default()
        }
    }

    fn patch_keys() -> DjKeys {
        let mut k = keys(true);
        let slots = vec![slot_with_knobs(), PatchSlot::default(), PatchSlot::default(), PatchSlot::default()];
        k.set_patches(&slots, [vec![("cutoff".into(), 0.25), ("reso".into(), 0.4)], vec![], vec![], vec![]]);
        k
    }

    fn knob_calls(calls: &[DjCall]) -> Vec<(u32, String, f64)> {
        calls
            .iter()
            .filter_map(|c| match c {
                DjCall::PatchKnob { slot, name, value } => Some((*slot, name.clone(), *value)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_hinted_patch_knob_is_edited_by_its_range_and_sent_as_a_knob_action() {
        let t0 = Instant::now();
        let mut k = patch_keys();
        // P1 knob 1 is `td`: a step is 1/20 of the range
        let mut t = select(&mut k, "td", t0);
        assert_eq!(k.hints().selected, Some(Target::Knob(0, 0)));
        assert_eq!(k.target_name(Target::Knob(0, 0)), "P1 cutoff");
        let mut got = Vec::new();
        for _ in 0..2 {
            tap_key(&mut k, ']', t);
            got.extend(knob_calls(&k.tick(t)));
            t += ms(200);
        }
        assert_eq!(got, vec![(0, "cutoff".to_string(), 0.35), (0, "cutoff".to_string(), 0.4)]);
        tap_key(&mut k, '[', t);
        assert_eq!(knob_calls(&k.tick(t)), vec![(0, "cutoff".to_string(), 0.35)]);
        // the value shown follows what we sent, not the stale poll
        assert!((k.value_of(Target::Knob(0, 0), t) - 0.35).abs() < 1e-9);
        // `\` goes back to the value the patch file has
        tap_key(&mut k, '\\', t + ms(200));
        assert_eq!(knob_calls(&k.tick(t + ms(200))), vec![(0, "cutoff".to_string(), 0.25)]);
        // the second knob (`tq`) has its own range: 0..0.8, steps of 0.04
        let t = select(&mut k, "tq", t + ms(400));
        tap_key(&mut k, ']', t);
        assert_eq!(knob_calls(&k.tick(t)), vec![(0, "reso".to_string(), 0.54)]);
        // a knob that stops at its maximum does not send past it
        let mut t = t + ms(400);
        for _ in 0..20 {
            tap_key(&mut k, ']', t);
            k.tick(t);
            t += ms(200);
        }
        assert!((k.value_of(Target::Knob(0, 1), t) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn slot_mixer_controls_go_out_as_djset_on_deck_two_plus_slot() {
        let t0 = Instant::now();
        let mut k = patch_keys();
        // P1's fader is `tz` and starts from what PatchState says
        let t = select(&mut k, "tz", t0);
        assert_eq!(k.hints().selected, Some(Target::Slot(Slot::Ctl(Ctl::Fader(2)))));
        assert_eq!(k.target_name(k.hints().selected.unwrap()), "P1 fader");
        assert!((k.value_of(k.hints().selected.unwrap(), t) - 0.6).abs() < 1e-9);
        tap_key(&mut k, '[', t);
        assert_eq!(sets(&k.tick(t)), vec![("fader", 2, 0.55)]);
        // the filter (`tx`) is not reported: it starts at 0 and then remembers what was sent
        let t = select(&mut k, "tx", t + ms(500));
        tap_key(&mut k, ']', t);
        assert_eq!(sets(&k.tick(t)), vec![("filter", 2, 0.05)]);
        let later = t + ms(2000);
        assert!((k.value_of(Target::Slot(Slot::Ctl(Ctl::Filter(2))), later) - 0.05).abs() < 1e-9);
        // P4's low (`cc`): deck 5
        let t = select(&mut k, "cc", later);
        tap_key(&mut k, '[', t);
        assert_eq!(sets(&k.tick(t)), vec![("low", 5, -0.05)]);
    }

    #[test]
    fn patch_hint_labels_are_two_letters_prefix_free_and_never_a_deck_label() {
        let tab = hints::table();
        let deck_labels: Vec<&str> = tab.iter().filter(|(t, _)| !matches!(t, Target::Knob(..) | Target::Slot(Slot::Ctl(Ctl::Fader(2..) | Ctl::Filter(2..) | Ctl::Eq(2.., _))))).map(|(_, l)| *l).collect();
        assert_eq!(deck_labels.len(), 23);
        let patch: Vec<&str> = tab.iter().skip(23).map(|(_, l)| *l).collect();
        assert_eq!(patch.len(), 48);
        for l in &patch {
            assert_eq!(l.len(), 2, "{l}");
            assert!(!deck_labels.contains(l), "{l} is a deck label");
        }
        // the first letter of a patch label is never a one-letter label of the deck set
        for l in &patch {
            assert!(!deck_labels.iter().any(|d| d.len() == 1 && l.starts_with(d)), "{l}");
        }
        let mut all: Vec<&str> = tab.iter().map(|(_, l)| *l).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), tab.len(), "labels are unique");
    }

    #[test]
    fn knob_actions_coalesce_per_slot_and_name() {
        let knob = |slot, name: &str, value| DjCall::PatchKnob { slot, name: name.into(), value };
        let out = coalesce(vec![knob(0, "cutoff", 0.1), knob(0, "reso", 0.2), knob(0, "cutoff", 0.3), knob(1, "cutoff", 0.9)]);
        assert_eq!(out, vec![knob(0, "reso", 0.2), knob(0, "cutoff", 0.3), knob(1, "cutoff", 0.9)]);
        assert_eq!(knob(0, "x", 0.0).name(), "PatchAction");
    }

    #[test]
    fn the_patch_pane_key_is_f12() {
        let f12 = ev(KeyCode::F(12), KeyModifiers::NONE, KeyEventKind::Press);
        assert_eq!(bind(&normalize(&f12)), Some(Binding::Nav(Nav::Patch)));
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
            // the gain is shown in dB and sent as a multiplier: +0.5 dB, and 1 (the trim as is)
            ("zs", "gain", 1, db_to_gain(0.5), 1.0),
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
        // shown in dB (-2.0), sent as codjd's multiplier on top of the -2.5 dB trim: +0.5 dB.
        // Never the dB value itself: codjd takes a negative gain as silence
        let gain = |db: f64| ("gain", 0, db_to_gain(db));
        assert_eq!(sets(&k.tick(t)), vec![gain(0.5)]);
        // the daemon keeps reporting the auto-gain, the shown trim is the one sent
        t += ms(2000);
        k.set_snapshot(&[a.clone(), deck(false)], &MixerInfo::default());
        assert_eq!(k.gain_shown(0, t), -2.0);
        tap_key(&mut k, ']', t);
        assert_eq!(sets(&k.tick(t)), vec![gain(1.0)]);
        // back to the default: the trim as it is, a multiplier of 1
        tap_key(&mut k, '\\', t + ms(100));
        assert_eq!(sets(&k.tick(t + ms(100))), vec![("gain", 0, 1.0)]);
        // another track on the deck: its own auto-gain
        let mut other = a.clone();
        other.path = "/m/b.flac".into();
        other.gain_db = 1.0;
        k.set_snapshot(&[other, deck(false)], &MixerInfo::default());
        assert_eq!(k.gain_shown(0, t + ms(5000)), 1.0);
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
    fn hidden_controls_cannot_be_picked() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.set_visible(vec![Target::Slot(Slot::Ctl(Ctl::Filter(1)))]);
        let t = select(&mut k, "a", t0);
        assert!(k.hints().active, "a is not on screen: ignored");
        tap_key(&mut k, 's', t);
        assert_eq!(k.hints().selected, Some(Target::Slot(Slot::Ctl(Ctl::Filter(1)))));
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
        tap_key(&mut k, ' ', t);
        assert_eq!(k.tick(t), vec![DjCall::Play(0)]);
    }

    // ---- the vim key map

    fn shifted(c: char) -> KeyEvent {
        ev(KeyCode::Char(c), KeyModifiers::SHIFT, KeyEventKind::Press)
    }
    fn ctrl(c: char) -> KeyEvent {
        ev(KeyCode::Char(c), KeyModifiers::CONTROL, KeyEventKind::Press)
    }
    fn esc() -> KeyEvent {
        ev(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press)
    }
    /// types the characters one after the other, 100 ms apart, and returns what the keys queued
    fn typed(k: &mut DjKeys, text: &str, t: Instant) -> Vec<DjCall> {
        let mut at = t;
        for c in text.chars() {
            k.on_key(&press(c), at);
            at += ms(100);
        }
        k.tick(at)
    }
    fn loaded_a(hotcues: Vec<f64>, loop_beats: f64) -> DeckInfo {
        DeckInfo { hotcues, loop_beats, duration: 300.0, track_bpm: 120.0, ..deck(false) }
    }

    #[test]
    fn key_map_is_vim_first() {
        use Binding::*;
        let n = |c: char| bind(&normalize(&press(c)));
        // focus
        assert_eq!(n('h'), Some(Focus(0)));
        assert_eq!(n('l'), Some(Focus(1)));
        // the deck
        assert_eq!(n(' '), Some(Act(self::Act::PlayPause)));
        assert_eq!(n('c'), Some(Cue));
        assert_eq!(n('s'), Some(Act(self::Act::Sync)));
        // motions: w (ge back) beats, W B bars, } { phrases; a b load the decks
        assert_eq!(n('w'), Some(Act(self::Act::Jump(1.0))));
        assert_eq!(n('a'), Some(Nav(self::Nav::Load(0))));
        assert_eq!(n('b'), Some(Nav(self::Nav::Load(1))));
        assert_eq!(bind(&normalize(&press('W'))), Some(Act(self::Act::Jump(4.0))));
        assert_eq!(bind(&normalize(&press('B'))), Some(Act(self::Act::Jump(-4.0))));
        assert_eq!(bind(&normalize(&press('}'))), Some(Act(self::Act::Jump(64.0))));
        assert_eq!(bind(&normalize(&press('{'))), Some(Act(self::Act::Jump(-64.0))));
        assert_eq!(n('g'), Some(Pend(Pending::G)));
        assert_eq!(bind(&normalize(&press('G'))), Some(Act(self::Act::Seek(SeekTo::Outro))));
        assert_eq!(bind(&normalize(&press('$'))), Some(Act(self::Act::Seek(SeekTo::Outro))));
        // marks, loops, pitch
        assert_eq!(n('m'), Some(Pend(Pending::Mark)));
        assert_eq!(n('\''), Some(Pend(Pending::GoMark)));
        assert_eq!(n('`'), Some(Pend(Pending::GoMark)));
        assert_eq!(n('v'), Some(Act(self::Act::Loop)));
        assert_eq!(n('-'), Some(Pitch(-1)));
        assert_eq!(bind(&normalize(&press('+'))), Some(Pitch(1)));
        assert_eq!(n('='), Some(Act(self::Act::PitchReset)));
        assert_eq!(n('['), Some(Bracket(-1)));
        assert_eq!(n(']'), Some(Bracket(1)));
        // repeat, undo, redo
        assert_eq!(n('.'), Some(Repeat));
        assert_eq!(n('u'), Some(Undo));
        assert_eq!(bind(&normalize(&ctrl('r'))), Some(Redo));
        // digits are counts
        for d in 0..=9u8 {
            assert_eq!(n((b'0' + d) as char), Some(Digit(d)));
        }
        // the screen
        assert_eq!(n('f'), Some(Nav(self::Nav::Hints)));
        assert_eq!(n(';'), Some(Nav(self::Nav::Toggle)));
        assert_eq!(n('\\'), Some(Nav(self::Nav::Reset)));
        assert_eq!(n('/'), Some(Nav(self::Nav::SearchNow)));
        assert_eq!(n(':'), Some(Nav(self::Nav::Command)));
        assert_eq!(n('?'), Some(Nav(self::Nav::Help)));
        let key = |c| bind(&normalize(&ev(c, KeyModifiers::NONE, KeyEventKind::Press)));
        assert_eq!(key(KeyCode::Tab), Some(Nav(self::Nav::Search)));
        assert_eq!(key(KeyCode::Left), Some(Focus(0)));
        assert_eq!(key(KeyCode::Right), Some(Focus(1)));
        assert_eq!(key(KeyCode::Down), Some(Nav(self::Nav::LibDown)));
        assert_eq!(key(KeyCode::Enter), Some(Nav(self::Nav::LoadSmart)));
        assert_eq!(key(KeyCode::F(1)), Some(Tap(self::Tap::Headphones(0))));
        assert_eq!(key(KeyCode::F(2)), Some(Tap(self::Tap::Headphones(1))));
        assert_eq!(key(KeyCode::F(5)), Some(Tap(self::Tap::Quantize)));
        assert_eq!(key(KeyCode::F(6)), Some(Nav(self::Nav::SetArmNext)));
        assert_eq!(key(KeyCode::F(7)), Some(Nav(self::Nav::SetAuto)));
        assert_eq!(key(KeyCode::F(8)), Some(Nav(self::Nav::SetDisarm)));
        assert_eq!(key(KeyCode::F(9)), Some(Nav(self::Nav::RecordToggle)));
        assert_eq!(key(KeyCode::F(10)), Some(Nav(self::Nav::Editor)));
        assert_eq!(key(KeyCode::F(11)), Some(Nav(self::Nav::Prompt)));
        assert_eq!(key(KeyCode::F(12)), Some(Nav(self::Nav::Patch)));
        // Shift+Tab reaches the library search too
        assert_eq!(bind(&normalize(&ev(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press))), Some(Nav(self::Nav::Search)));
    }

    #[test]
    fn shifted_keys_in_every_protocol_mode() {
        use Binding::*;
        // the kitty protocol reports the base key with SHIFT, a legacy terminal the US symbol
        for (with_shift, symbol, want) in [
            ('=', '+', Pitch(1)),
            (']', '}', Act(self::Act::Jump(64.0))),
            ('[', '{', Act(self::Act::Jump(-64.0))),
            ('4', '$', Act(self::Act::Seek(SeekTo::Outro))),
            ('g', 'G', Act(self::Act::Seek(SeekTo::Outro))),
            ('w', 'W', Act(self::Act::Jump(4.0))),
            ('/', '?', Nav(self::Nav::Help)),
            (';', ':', Nav(self::Nav::Command)),
        ] {
            assert_eq!(bind(&normalize(&shifted(with_shift))), Some(want), "{with_shift} + shift");
            assert_eq!(bind(&normalize(&press(symbol))), Some(want), "{symbol}");
        }
        // `?` with SHIFT and as a symbol at once
        assert_eq!(bind(&normalize(&shifted('?'))), Some(Nav(self::Nav::Help)));
    }

    #[test]
    fn nothing_of_the_old_map_is_reachable() {
        // Alt combinations are gone, whatever the letter
        for c in (b' '..=b'~').map(|c| c as char) {
            assert_eq!(bind(&normalize(&alt(c, KeyEventKind::Press))), None, "Alt+{c}");
        }
        // the old deck letters that have no meaning now
        for c in "zxeriynz,<>".chars() {
            assert_eq!(bind(&normalize(&press(c))), None, "{c}");
        }
        // capitals and shifted symbols of the old map: A B load, Shift+digit clears
        for c in "ACDEFHIJKLMNOPQSUVXYZ!@#%^&*()_|".chars() {
            assert_eq!(bind(&normalize(&press(c))), None, "{c}");
        }
        // ctrl combinations never reach the decks (Ctrl-r is redo, nothing else)
        for c in (b'a'..=b'z').map(|c| c as char).filter(|c| *c != 'r') {
            assert_eq!(bind(&normalize(&ctrl(c))), None, "Ctrl-{c}");
        }
        // Esc is not a deck key: it never leaves
        assert_eq!(bind(&normalize(&esc())), None);
    }

    #[test]
    fn digits_are_counts_not_hot_cues() {
        let t0 = Instant::now();
        let mut k = keys(true);
        for c in "1234567890".chars() {
            assert_eq!(k.on_key(&press(c), t0), None);
        }
        assert!(k.tick(t0).is_empty(), "digits queue nothing");
    }

    #[test]
    fn counts_multiply_the_motions() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(typed(&mut k, "w", t0), vec![DjCall::BeatJump { deck: 0, beats: 1.0 }]);
        assert_eq!(typed(&mut k, "ge", t0), vec![DjCall::BeatJump { deck: 0, beats: -1.0 }]);
        assert_eq!(typed(&mut k, "4w", t0), vec![DjCall::BeatJump { deck: 0, beats: 4.0 }]);
        assert_eq!(typed(&mut k, "12ge", t0), vec![DjCall::BeatJump { deck: 0, beats: -12.0 }]);
        // a zero inside a count is a digit
        assert_eq!(typed(&mut k, "10w", t0), vec![DjCall::BeatJump { deck: 0, beats: 10.0 }]);
        // bars and phrases
        assert_eq!(typed(&mut k, "W", t0), vec![DjCall::BeatJump { deck: 0, beats: 4.0 }]);
        assert_eq!(typed(&mut k, "3B", t0), vec![DjCall::BeatJump { deck: 0, beats: -12.0 }]);
        assert_eq!(typed(&mut k, "}", t0), vec![DjCall::BeatJump { deck: 0, beats: 64.0 }]);
        assert_eq!(typed(&mut k, "2}", t0), vec![DjCall::BeatJump { deck: 0, beats: 128.0 }]);
        assert_eq!(typed(&mut k, "{", t0), vec![DjCall::BeatJump { deck: 0, beats: -64.0 }]);
        // the count is used up: the next motion is a single one again
        assert_eq!(typed(&mut k, "w", t0), vec![DjCall::BeatJump { deck: 0, beats: 1.0 }]);
        // a count is capped
        let calls = typed(&mut k, "99999w", t0);
        assert_eq!(calls, vec![DjCall::BeatJump { deck: 0, beats: 9999.0 }]);
    }

    #[test]
    fn a_digit_in_the_t_menu_is_a_length_step() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(k.on_key(&press('t'), t0), None);
        assert_eq!(k.on_key(&press('2'), t0 + ms(50)), None);
        assert_eq!(k.pending(), Some((Pending::T { outro: false }, Some(8))));
        assert_eq!(k.on_key(&press('s'), t0 + ms(100)), Some(Nav::Transition { recipe: Some("bassswap"), bars: Some(8), outro: false }));
        assert_eq!(k.pending(), None);
    }

    #[test]
    fn showcmd_shows_the_count_and_the_half_typed_command() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(k.showcmd(), "");
        k.on_key(&press('4'), t0);
        assert_eq!(k.showcmd(), "4");
        k.on_key(&press('2'), t0 + ms(300));
        assert_eq!(k.showcmd(), "42");
        k.on_key(&press('m'), t0 + ms(600));
        assert_eq!(k.showcmd(), "42m");
        k.on_key(&press('a'), t0 + ms(900));
        assert_eq!(k.showcmd(), "");
        k.on_key(&press('g'), t0 + ms(1200));
        assert_eq!(k.showcmd(), "g");
        k.on_key(&press('\''), t0 + ms(1500));
        // the key after `g` is not `'`: the command is dropped
        assert_eq!(k.showcmd(), "");
    }

    #[test]
    fn esc_and_unbound_keys_cancel_a_count() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('4'), t0);
        assert_eq!(k.on_key(&esc(), t0 + ms(300)), None);
        assert_eq!(k.showcmd(), "");
        assert_eq!(typed(&mut k, "w", t0 + ms(600)), vec![DjCall::BeatJump { deck: 0, beats: 1.0 }]);
        // `z` means nothing and drops the count like in vim
        assert_eq!(typed(&mut k, "4zw", t0 + ms(2000)), vec![DjCall::BeatJump { deck: 0, beats: 1.0 }]);
        // Esc cancels the half-typed `m`, then a mark letter is no mark
        k.on_key(&press('m'), t0 + ms(5000));
        k.on_key(&esc(), t0 + ms(5300));
        assert_eq!(k.showcmd(), "");
        assert_eq!(typed(&mut k, "a", t0 + ms(5600)), vec![]);
    }

    #[test]
    fn auto_repeat_does_not_grow_a_count_or_repeat_a_motion() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('4'), t0);
        k.on_key(&ev(KeyCode::Char('4'), KeyModifiers::NONE, KeyEventKind::Repeat), t0 + ms(40));
        assert_eq!(k.showcmd(), "4");
        k.on_key(&press('w'), t0 + ms(100));
        k.on_key(&ev(KeyCode::Char('w'), KeyModifiers::NONE, KeyEventKind::Repeat), t0 + ms(140));
        assert_eq!(k.tick(t0 + ms(200)), vec![DjCall::BeatJump { deck: 0, beats: 4.0 }]);
        // without release events a burst of one key is its auto-repeat too
        let mut k = keys(false);
        k.on_key(&press('4'), t0);
        k.on_key(&press('4'), t0 + ms(30));
        assert_eq!(k.showcmd(), "4");
    }

    #[test]
    fn marks_are_hot_cues() {
        let t0 = Instant::now();
        let mut k = DjKeys::new(true);
        k.set_snapshot(&[loaded_a(vec![12.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0, 40.0], 0.0), deck(false)], &MixerInfo::default());
        let hc = |slot, action| DjCall::Hotcue { deck: 0, slot, action };
        // m{a-h} saves, ' and ` jump
        assert_eq!(typed(&mut k, "ma", t0), vec![hc(0, "set")]);
        assert_eq!(typed(&mut k, "mc", t0), vec![hc(2, "set")]);
        assert_eq!(typed(&mut k, "mh", t0), vec![hc(7, "set")]);
        assert_eq!(typed(&mut k, "'a", t0), vec![hc(0, "jump")]);
        assert_eq!(typed(&mut k, "`h", t0), vec![hc(7, "jump")]);
        // a mark that is not set is not jumped to
        let mut calls = Vec::new();
        k.on_key(&press('\''), t0 + ms(100));
        let nav = k.on_key(&press('b'), t0 + ms(500));
        calls.extend(k.tick(t0 + ms(500)));
        assert!(calls.is_empty() && matches!(nav, Some(Nav::Note(n)) if n.contains("not set")), "{calls:?} {nav:?}");
        // only a to h
        k.on_key(&press('m'), t0 + ms(1000));
        assert!(matches!(k.on_key(&press('z'), t0 + ms(1400)), Some(Nav::Note(n)) if n.contains("a to h")));
        assert!(k.tick(t0 + ms(1400)).is_empty());
        // marks belong to the focused deck
        k.on_key(&press('l'), t0 + ms(2000));
        let calls = typed(&mut k, "mb", t0 + ms(2100));
        assert_eq!(calls, vec![DjCall::Hotcue { deck: 1, slot: 1, action: "set" }]);
    }

    #[test]
    fn v_is_a_loop_and_a_count_sizes_it() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(typed(&mut k, "v", t0), vec![DjCall::Loop { deck: 0, beats: 4.0 }]);
        assert_eq!(typed(&mut k, "8v", t0), vec![DjCall::Loop { deck: 0, beats: 8.0 }]);
        assert_eq!(typed(&mut k, "16v", t0), vec![DjCall::Loop { deck: 0, beats: 16.0 }]);
        assert_eq!(typed(&mut k, "2v", t0), vec![DjCall::Loop { deck: 0, beats: 2.0 }]);
        // while looping, v leaves it and a count resizes it
        k.set_snapshot(&[loaded_a(vec![], 8.0), deck(false)], &MixerInfo::default());
        assert_eq!(typed(&mut k, "v", t0), vec![DjCall::Loop { deck: 0, beats: 0.0 }]);
        assert_eq!(typed(&mut k, "4v", t0), vec![DjCall::Loop { deck: 0, beats: 4.0 }]);
        // the loop length of the hints is the default size
        let mut k = keys(true);
        let t = select(&mut k, "ca", t0);
        tap_key(&mut k, ']', t);
        assert_eq!(typed(&mut k, "v", t + ms(300)), vec![DjCall::Loop { deck: 0, beats: 8.0 }]);
    }

    #[test]
    fn focus_decides_the_deck_of_every_action() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(k.focus(), 0);
        k.on_key(&press('l'), t0);
        assert_eq!(k.focus(), 1);
        let calls = typed(&mut k, " s4wv", t0 + ms(100));
        assert_eq!(
            calls,
            vec![
                DjCall::Play(1),
                DjCall::Sync(1),
                DjCall::BeatJump { deck: 1, beats: 4.0 },
                DjCall::Loop { deck: 1, beats: 4.0 },
            ]
        );
        // cue, nudge and pitch too (a playing deck: ] bends it, a paused one it rolls)
        k.set_snapshot(&[deck(false), deck(true)], &MixerInfo::default());
        k.on_key(&press('c'), t0 + ms(3000));
        assert_eq!(k.tick(t0 + ms(3000)), vec![DjCall::Cue { deck: 1, pressed: true }]);
        k.on_key(&rel('c'), t0 + ms(3100));
        k.tick(t0 + ms(3100));
        k.on_key(&press(']'), t0 + ms(3200));
        assert_eq!(k.tick(t0 + ms(3200)), vec![DjCall::Nudge { deck: 1, amount: 1.0 }]);
        k.on_key(&rel(']'), t0 + ms(3300));
        k.tick(t0 + ms(3300));
        k.on_key(&press('-'), t0 + ms(3400));
        assert_eq!(sets(&k.tick(t0 + ms(3400))), vec![("rate", 1, 0.999)]);
        k.on_key(&rel('-'), t0 + ms(3450));
        k.tick(t0 + ms(3450));
        // back to A, with the arrow keys as well
        k.on_key(&press('h'), t0 + ms(4000));
        assert_eq!(typed(&mut k, " ", t0 + ms(4100)), vec![DjCall::Play(0)]);
        k.on_key(&ev(KeyCode::Right, KeyModifiers::NONE, KeyEventKind::Press), t0 + ms(5000));
        assert_eq!(k.focus(), 1);
        k.on_key(&ev(KeyCode::Left, KeyModifiers::NONE, KeyEventKind::Press), t0 + ms(5100));
        assert_eq!(k.focus(), 0);
        // focusing drops a half-typed count
        k.on_key(&press('4'), t0 + ms(6000));
        k.on_key(&press('l'), t0 + ms(6300));
        assert_eq!(k.showcmd(), "");
        // play and pause follow the polled state of the focused deck
        k.set_snapshot(&[deck(false), deck(true)], &MixerInfo::default());
        assert_eq!(typed(&mut k, " ", t0 + ms(7000)), vec![DjCall::Pause(1)]);
    }

    #[test]
    fn gt_asks_for_an_automix_and_sends_nothing_itself() {
        let t0 = Instant::now();
        let mut k = DjKeys::new(true);
        k.set_snapshot(&[DeckInfo { playing: true, ..loaded_a(vec![], 0.0) }, loaded_a(vec![], 0.0)], &MixerInfo::default());
        assert_eq!(k.on_key(&press('g'), t0), None);
        assert_eq!(k.showcmd(), "g");
        assert_eq!(k.on_key(&press('t'), t0 + ms(100)), Some(Nav::AutoMix));
        assert_eq!(k.showcmd(), "");
        assert_eq!(k.tick(t0 + ms(200)), vec![]);
    }

    #[test]
    fn start_and_outro_seek_on_the_focused_deck() {
        let t0 = Instant::now();
        let mut k = DjKeys::new(true);
        k.set_snapshot(&[loaded_a(vec![], 0.0), DeckInfo { track_bpm: 0.0, duration: 200.0, ..deck(false) }], &MixerInfo::default());
        k.set_starts([Some(0.37), None]);
        let seek = |deck, seconds| DjCall::Seek { deck, seconds };
        // 0 and gg: the first downbeat when the grid is known
        assert_eq!(typed(&mut k, "0", t0), vec![seek(0, 0.37)]);
        assert_eq!(typed(&mut k, "gg", t0), vec![seek(0, 0.37)]);
        // G and $: 32 bars before the end: 128 beats at 120 BPM = 64 s
        assert_eq!(typed(&mut k, "G", t0), vec![seek(0, 236.0)]);
        assert_eq!(typed(&mut k, "$", t0), vec![seek(0, 236.0)]);
        // 0 after a count is a digit, not the start: 10 then something
        assert_eq!(typed(&mut k, "10", t0), vec![]);
        k.on_key(&esc(), t0 + ms(9000));
        // no grid: the start of the track, and the last minute for the outro
        k.on_key(&press('l'), t0 + ms(9100));
        assert_eq!(typed(&mut k, "0", t0 + ms(9200)), vec![seek(1, 0.0)]);
        assert_eq!(typed(&mut k, "G", t0 + ms(9300)), vec![seek(1, 140.0)]);
        // an empty deck says so
        let mut k = DjKeys::new(true);
        k.set_snapshot(&[DeckInfo::default(), DeckInfo::default()], &MixerInfo::default());
        assert!(matches!(k.on_key(&press('0'), t0), Some(Nav::Note(n)) if n.contains("no track")));
    }

    #[test]
    fn dot_repeats_the_last_action_on_the_focused_deck() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert!(matches!(k.on_key(&press('.'), t0), Some(Nav::Note(n)) if n.contains("nothing to repeat")));
        assert_eq!(
            typed(&mut k, "4w.", t0 + ms(100)),
            vec![DjCall::BeatJump { deck: 0, beats: 4.0 }, DjCall::BeatJump { deck: 0, beats: 4.0 }]
        );
        // a count in front of the dot replaces the old one
        assert_eq!(typed(&mut k, "2.", t0 + ms(5000)), vec![DjCall::BeatJump { deck: 0, beats: 2.0 }]);
        // the dot acts on whichever deck has the focus now
        k.on_key(&press('l'), t0 + ms(8000));
        assert_eq!(typed(&mut k, ".", t0 + ms(8100)), vec![DjCall::BeatJump { deck: 1, beats: 2.0 }]);
        // sync, play, marks and loops repeat too; the dot itself is not remembered
        assert_eq!(typed(&mut k, "s..", t0 + ms(10000)), vec![DjCall::Sync(1), DjCall::Sync(1), DjCall::Sync(1)]);
        assert_eq!(typed(&mut k, "mb.", t0 + ms(13000)).len(), 2);
        // pitch steps repeat, a hold does not matter
        let calls = typed(&mut k, "3+.", t0 + ms(20000));
        let rates = sets(&calls);
        assert_eq!(rates.last().map(|r| r.0), Some("rate"));
        assert!((rates.last().unwrap().2 - 1.006).abs() < 1e-9, "{rates:?}");
    }

    #[test]
    fn pitch_keys_step_with_counts_and_reset() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // a count steps without a ramp
        assert_eq!(sets(&typed(&mut k, "4+", t0)), vec![("rate", 0, 1.004)]);
        assert_eq!(sets(&typed(&mut k, "2-", t0 + ms(300))), vec![("rate", 0, 1.002)]);
        // = is back to 0 %
        assert_eq!(sets(&typed(&mut k, "=", t0 + ms(600))), vec![("rate", 0, 1.0)]);
    }

    #[test]
    fn u_and_ctrl_r_undo_and_redo_control_changes() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert!(matches!(k.on_key(&press('u'), t0), Some(Nav::Note(n)) if n.contains("nothing to undo")));
        assert!(matches!(k.on_key(&ctrl('r'), t0), Some(Nav::Note(n)) if n.contains("nothing to redo")));
        // three separate edits: filter A up twice, crossfader down once (a second apart)
        let mut t = select(&mut k, "a", t0);
        tap_key(&mut k, ']', t);
        t += ms(1000);
        tap_key(&mut k, ']', t);
        t += ms(1000);
        let t_sel = select(&mut k, "w", t);
        tap_key(&mut k, '[', t_sel);
        let mut t = t_sel + ms(1000);
        assert_eq!(sets(&k.tick(t)), vec![("filter", 0, 0.1), ("crossfader", 0, -0.05)]);
        // u takes the last one back: the crossfader returns to 0
        t += ms(100);
        k.on_key(&press('u'), t);
        assert_eq!(sets(&k.tick(t)), vec![("crossfader", 0, 0.0)]);
        t += ms(1000);
        // again: filter 0.1 -> 0.05
        k.on_key(&press('u'), t);
        assert_eq!(sets(&k.tick(t)), vec![("filter", 0, 0.05)]);
        t += ms(1000);
        // Ctrl-r does it again
        k.on_key(&ctrl('r'), t);
        assert_eq!(sets(&k.tick(t)), vec![("filter", 0, 0.1)]);
        t += ms(1000);
        k.on_key(&ctrl('r'), t);
        assert_eq!(sets(&k.tick(t)), vec![("crossfader", 0, -0.05)]);
        t += ms(1000);
        assert!(matches!(k.on_key(&ctrl('r'), t), Some(Nav::Note(_))));
        // a new change after an undo drops what could be redone
        k.on_key(&press('u'), t + ms(1000));
        k.tick(t + ms(1000));
        tap_key(&mut k, ']', t + ms(2000));
        k.tick(t + ms(2000));
        assert!(matches!(k.on_key(&ctrl('r'), t + ms(3000)), Some(Nav::Note(n)) if n.contains("nothing to redo")));
    }

    #[test]
    fn one_held_ramp_is_one_undo_step() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('+'), t0);
        for i in 0..100 {
            k.tick(t0 + ms(i * 10));
        }
        k.on_key(&rel('+'), t0 + ms(1000));
        let moved = k.target(Slot::Rate(0), t0 + ms(1000));
        assert!(moved > 1.003, "{moved}");
        k.tick(t0 + ms(1050));
        k.on_key(&press('u'), t0 + ms(2000));
        let back = sets(&k.tick(t0 + ms(2000)));
        assert_eq!(back, vec![("rate", 0, 1.0)]);
        assert!(matches!(k.on_key(&press('u'), t0 + ms(3000)), Some(Nav::Note(_))), "a single step");
    }

    #[test]
    fn commands_set_controls_and_can_be_undone() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.set_control(Slot::Ctl(Ctl::Fader(1)), 0.4, t0);
        k.set_control(Slot::Ctl(Ctl::Eq(0, 0)), -1.0, t0);
        k.set_control(Slot::Ctl(Ctl::Filter(0)), -0.3, t0);
        k.set_control(Slot::Ctl(Ctl::Crossfader), 2.0, t0);
        let out = sets(&k.tick(t0));
        assert!(out.contains(&("fader", 1, 0.4)) && out.contains(&("low", 0, -1.0)) && out.contains(&("filter", 0, -0.3)));
        // out of range is clamped
        assert!(out.contains(&("crossfader", 0, 1.0)));
        k.on_key(&press('u'), t0 + ms(2000));
        assert_eq!(sets(&k.tick(t0 + ms(2000))), vec![("crossfader", 0, 0.0)]);
    }

    #[test]
    fn nudge_and_cue_keys_on_the_focused_deck() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // a playing deck: [ ] bend it (a paused one they roll)
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press(']'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Nudge { deck: 0, amount: 1.0 }]);
        k.on_key(&rel(']'), t0 + ms(300));
        assert_eq!(k.tick(t0 + ms(300)), vec![DjCall::Nudge { deck: 0, amount: 0.0 }]);
        // with a control picked the same keys step it instead
        let t = select(&mut k, "a", t0 + ms(1000));
        tap_key(&mut k, ']', t);
        let calls = k.tick(t);
        assert_eq!(sets(&calls), vec![("filter", 0, 0.05)]);
        assert!(!calls.iter().any(|c| matches!(c, DjCall::Nudge { .. })));
        // Esc drops the pick (never leaves): the keys nudge again
        assert_eq!(k.on_key(&esc(), t + ms(500)), None);
        tap_key(&mut k, '[', t + ms(1000));
        assert_eq!(k.tick(t + ms(1000)), vec![DjCall::Nudge { deck: 0, amount: -1.0 }]);
    }

    #[test]
    fn esc_never_leaves_dj_mode() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // plain, repeated, with a count, a pending mark, hints, a selection: always nothing
        assert_eq!(k.on_key(&esc(), t0), None);
        assert_eq!(k.on_key(&ev(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Repeat), t0 + ms(10)), None);
        k.on_key(&press('3'), t0 + ms(100));
        assert_eq!(k.on_key(&esc(), t0 + ms(200)), None);
        k.on_key(&press('m'), t0 + ms(300));
        assert_eq!(k.on_key(&esc(), t0 + ms(400)), None);
        let t = select(&mut k, "w", t0 + ms(500));
        for i in 0..4 {
            assert_eq!(k.on_key(&esc(), t + ms(i * 300)), None);
        }
        assert!(k.hints().selected.is_none());
        // `:q` is how DJ mode ends
        assert_eq!(super::super::parse_command("q"), super::super::Command::Exit);
        assert_eq!(super::super::parse_command("q!"), super::super::Command::Exit);
    }

    #[test]
    fn nav_keys_do_not_repeat_except_scrolling() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let e = |c, kind| ev(c, KeyModifiers::NONE, kind);
        assert_eq!(k.on_key(&e(KeyCode::Tab, KeyEventKind::Press), t0), Some(Nav::Search));
        assert_eq!(k.on_key(&e(KeyCode::Tab, KeyEventKind::Repeat), t0), None);
        assert_eq!(k.on_key(&e(KeyCode::Down, KeyEventKind::Repeat), t0), Some(Nav::LibDown));
        assert_eq!(k.on_key(&e(KeyCode::Tab, KeyEventKind::Release), t0), None);
        // `/` is the library search, `?` the help, `:` the command line
        assert_eq!(k.on_key(&press('/'), t0 + ms(500)), Some(Nav::SearchNow));
        assert_eq!(k.on_key(&press('?'), t0 + ms(900)), Some(Nav::Help));
        assert_eq!(k.on_key(&press(':'), t0 + ms(1300)), Some(Nav::Command));
    }

    #[test]
    fn question_mark_is_the_help_key() {
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
        assert_eq!(b('\\'), Some(Binding::Nav(Nav::Reset)));
        assert_eq!(b('['), Some(Binding::Bracket(-1)));
        assert_eq!(b(']'), Some(Binding::Bracket(1)));
    }

    #[test]
    fn taps_resolve_against_the_polled_state() {
        let t0 = Instant::now();
        let mut k = DjKeys::new(true);
        let mut bb = deck(false);
        bb.headphones = true;
        k.set_snapshot(&[deck(true), bb], &MixerInfo { quantize: true, ..Default::default() });
        k.on_key(&press(' '), t0);
        k.on_key(&ev(KeyCode::F(2), KeyModifiers::NONE, KeyEventKind::Press), t0);
        k.on_key(&ev(KeyCode::F(1), KeyModifiers::NONE, KeyEventKind::Press), t0);
        k.on_key(&ev(KeyCode::F(5), KeyModifiers::NONE, KeyEventKind::Press), t0);
        assert_eq!(
            k.tick(t0),
            vec![
                DjCall::Pause(0),
                DjCall::Set { control: "cue", deck: 1, value: 0.0 },
                DjCall::Set { control: "cue", deck: 0, value: 1.0 },
                DjCall::Set { control: "quantize", deck: 0, value: 0.0 },
            ]
        );
    }

    #[test]
    fn repeat_events_do_not_retrigger_taps() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press(' '), t0);
        k.on_key(&ev(KeyCode::Char(' '), KeyModifiers::NONE, KeyEventKind::Repeat), t0 + ms(500));
        k.on_key(&rel(' '), t0 + ms(600));
        assert_eq!(k.tick(t0 + ms(600)), vec![DjCall::Play(0)]);
    }

    #[test]
    fn cue_hold_previews_until_release() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('c'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Cue { deck: 0, pressed: true }]);
        k.on_key(&ev(KeyCode::Char('c'), KeyModifiers::NONE, KeyEventKind::Repeat), t0 + ms(400));
        assert!(k.tick(t0 + ms(400)).is_empty());
        k.on_key(&rel('c'), t0 + ms(800));
        assert_eq!(k.tick(t0 + ms(800)), vec![DjCall::Cue { deck: 0, pressed: false }]);
        // a stray release does nothing
        k.on_key(&rel('c'), t0 + ms(900));
        assert!(k.tick(t0 + ms(900)).is_empty());
    }

    #[test]
    fn cue_without_release_events_toggles() {
        let t0 = Instant::now();
        let mut k = keys(false);
        k.on_key(&press('l'), t0);
        k.on_key(&press('c'), t0 + ms(300));
        assert_eq!(k.tick(t0 + ms(300)), vec![DjCall::Cue { deck: 1, pressed: true }]);
        // auto-repeat of the held key is ignored
        k.on_key(&press('c'), t0 + ms(340));
        assert!(k.tick(t0 + ms(340)).is_empty());
        k.on_key(&press('c'), t0 + ms(1200));
        assert_eq!(k.tick(t0 + ms(1200)), vec![DjCall::Cue { deck: 1, pressed: false }]);
        // on a playing deck the press is stop-at-cue: nothing is left to toggle off
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press('c'), t0 + ms(2300));
        assert_eq!(
            k.tick(t0 + ms(2300)),
            vec![DjCall::Cue { deck: 1, pressed: true }, DjCall::Cue { deck: 1, pressed: false }]
        );
    }

    #[test]
    fn nudge_follows_the_key_with_release_events() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // a playing deck: [ ] bend it (a paused one they roll)
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press(']'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Nudge { deck: 0, amount: 1.0 }]);
        // the other nudge key too: releasing one hands over to the one still down
        k.on_key(&press('['), t0 + ms(100));
        k.tick(t0 + ms(100));
        k.on_key(&rel('['), t0 + ms(200));
        assert_eq!(k.tick(t0 + ms(200)), vec![DjCall::Nudge { deck: 0, amount: 1.0 }]);
        k.on_key(&rel(']'), t0 + ms(300));
        assert_eq!(k.tick(t0 + ms(300)), vec![DjCall::Nudge { deck: 0, amount: 0.0 }]);
    }

    #[test]
    fn brackets_roll_a_paused_deck_and_r_resets_it() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let gridded = DeckInfo { track_bpm: 120.0, beat_in_bar: 1, ..deck(false) };
        k.set_snapshot(&[gridded, deck(false)], &MixerInfo::default());
        k.on_key(&press(']'), t0);
        assert_eq!(k.tick(t0 + ms(10)), [DjCall::BeatJump { deck: 0, beats: 0.5 * 120.0 / 60.0 }]);
        k.on_key(&press('['), t0 + ms(500));
        assert_eq!(k.tick(t0 + ms(510)), [DjCall::BeatJump { deck: 0, beats: -0.5 * 120.0 / 60.0 }]);
        // R: everything moved goes home
        k.on_key(&press('2'), t0 + ms(1000));
        k.on_key(&press('-'), t0 + ms(1010));
        k.tick(t0 + ms(1100));
        assert!(k.target(Slot::Rate(0), t0 + ms(1100)) < 1.0, "2- lowered the pitch");
        k.on_key(&KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT), t0 + ms(2000));
        let after = k.tick(t0 + ms(2100));
        assert!(sets(&after).is_empty() || sets(&after).iter().all(|(_, d, _)| *d == 0), "{after:?}");
        assert!((k.target(Slot::Rate(0), t0 + ms(2100)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn t_then_a_letter_is_a_transition_and_a_count_its_length() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(k.on_key(&press('t'), t0), None);
        assert_eq!(k.showcmd(), "t", "half typed, like vim");
        assert_eq!(k.on_key(&press('b'), t0 + ms(50)), Some(Nav::Transition { recipe: Some("blend"), bars: None, outro: false }));
        // a count is the length, a capital T is at the outro
        k.on_key(&press('1'), t0 + ms(100));
        k.on_key(&press('6'), t0 + ms(110));
        k.on_key(&KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT), t0 + ms(120));
        assert_eq!(k.on_key(&press('r'), t0 + ms(130)), Some(Nav::Transition { recipe: Some("loop-roll"), bars: Some(16), outro: true }));
        // tt: codj chooses; an unknown letter says what the letters are
        k.on_key(&press('t'), t0 + ms(200));
        assert_eq!(k.on_key(&press('t'), t0 + ms(210)), Some(Nav::Transition { recipe: None, bars: None, outro: false }));
        k.on_key(&press('t'), t0 + ms(300));
        assert!(matches!(k.on_key(&press('q'), t0 + ms(310)), Some(Nav::Note(n)) if n.contains("tb blend")));
        for (c, r) in [('s', "bassswap"), ('e', "eq-swap"), ('f', "filter-blend"), ('o', "echo-out"), ('c', "cut"), ('k', "brake")] {
            assert_eq!(technique(c), Some(Some(r)));
        }
    }

    #[test]
    fn spinning_the_picked_clock_moves_by_bars_and_faster_spins_go_further() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let gridded = DeckInfo { track_bpm: 120.0, beat_in_bar: 1, position: 60.0, ..deck(true) };
        k.set_snapshot(&[gridded, deck(false)], &MixerInfo::default());
        // a slow turn: a bar each; a spin: 2, 4, 8, 16 bars; turning back starts over
        let at = [0, 400, 500, 600, 700, 800, 900, 1000];
        for (i, ms_) in at.iter().enumerate() {
            k.scrub(0, if i == 7 { -1 } else { 1 }, t0 + ms(*ms_));
        }
        let bars: Vec<f64> = k.tick(t0 + ms(1100)).iter().map(|c| match c {
            DjCall::BeatJump { deck: 0, beats } => beats / 4.0,
            other => panic!("{other:?}"),
        }).collect();
        assert_eq!(bars, [1.0, 1.0, 2.0, 4.0, 8.0, 16.0, 16.0, -1.0]);
    }

    #[test]
    fn the_jog_step_is_a_control_of_its_own() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(k.jog_seconds(), 0.5);
        // picked and turned: coarser, never sent to codjd
        k.hints.selected = Some(Target::Slot(Slot::Jog));
        k.edit(1, t0);
        k.edit(1, t0 + ms(500));
        assert_eq!(k.jog_seconds(), 2.0);
        assert!(k.tick(t0 + ms(600)).is_empty(), "the jog step is codj's own");
        // the knob press puts it back, the config picks the nearest step
        k.reset(t0 + ms(700));
        assert_eq!(k.jog_seconds(), 0.5);
        k.set_jog_seconds(Some(0.9));
        assert_eq!(k.jog_seconds(), 1.0);
        k.set_jog_seconds(Some(100.0));
        assert_eq!(k.jog_seconds(), 5.0);
        assert_eq!(crate::dj::hints::fmt_jog(0.1), "0.1s");
        assert_eq!(crate::dj::hints::fmt_jog(2.0), "2s");
    }

    #[test]
    fn the_knob_is_a_jog_wheel() {
        let t0 = Instant::now();
        let mut k = keys(false);
        let gridded = |playing| DeckInfo { track_bpm: 120.0, beat_in_bar: 1, position: 10.0, ..deck(playing) };
        // paused: a slow turn scrubs 0.5 s a detent (1 beat at 120 BPM), spinning goes faster
        k.set_snapshot(&[gridded(false), deck(false)], &MixerInfo::default());
        k.jog(1, t0);
        k.jog(1, t0 + ms(500));
        k.jog(1, t0 + ms(520));
        k.jog(1, t0 + ms(540));
        k.jog(-1, t0 + ms(560));
        let jumps: Vec<f64> = k.tick(t0 + ms(600)).iter().map(|c| match c {
            DjCall::BeatJump { deck: 0, beats } => (*beats * 5.0 * 1e9).round() / 1e9,
            other => panic!("{other:?}"),
        }).collect();
        assert_eq!(jumps, [5.0, 5.0, 10.0, 20.0, -5.0]);
        // playing: the turn bends the tempo once, the bend ends when the knob stops
        k.set_snapshot(&[gridded(true), deck(false)], &MixerInfo::default());
        k.jog(1, t0 + ms(1000));
        k.jog(1, t0 + ms(1100));
        assert_eq!(k.tick(t0 + ms(1150)), [DjCall::Nudge { deck: 0, amount: JOG_BEND }]);
        k.jog(-1, t0 + ms(1200));
        assert_eq!(k.tick(t0 + ms(1250)), [DjCall::Nudge { deck: 0, amount: -JOG_BEND }]);
        assert!(k.tick(t0 + ms(1400)).is_empty());
        assert_eq!(k.tick(t0 + ms(1500)), [DjCall::Nudge { deck: 0, amount: 0.0 }]);
        // the focused deck is the one that moves
        k.set_focus(1);
        k.jog(1, t0 + ms(2000));
        assert!(matches!(k.tick(t0 + ms(2000))[..], [DjCall::Seek { deck: 1, .. }]), "no grid: a seek");
    }

    #[test]
    fn nudge_is_a_timed_bend_without_release_events() {
        let t0 = Instant::now();
        let mut k = keys(false);
        // a playing deck: [ ] bend it (a paused one they roll)
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press('l'), t0);
        k.on_key(&press(']'), t0 + ms(300));
        assert_eq!(k.tick(t0 + ms(300)), vec![DjCall::Nudge { deck: 1, amount: 1.0 }]);
        // auto-repeat keeps the bend alive without resending it
        k.on_key(&press(']'), t0 + ms(550));
        assert!(k.tick(t0 + ms(550)).is_empty());
        assert!(k.tick(t0 + ms(800)).is_empty());
        assert_eq!(k.tick(t0 + ms(900)), vec![DjCall::Nudge { deck: 1, amount: 0.0 }]);
        assert!(k.tick(t0 + ms(1000)).is_empty());
    }

    #[test]
    fn lost_cue_release_does_not_stick() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('c'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Cue { deck: 0, pressed: true }]);
        assert_eq!(k.tick(t0 + ms(3100)), vec![DjCall::Cue { deck: 0, pressed: false }]);
    }

    #[test]
    fn release_all_ends_holds_and_flushes_pending_work() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // a playing deck: [ ] bend it (a paused one they roll)
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press(']'), t0);
        k.on_key(&press('c'), t0);
        k.on_key(&press(' '), t0);
        k.on_key(&press('+'), t0);
        k.release_all();
        let calls = k.tick(t0 + ms(5));
        assert!(calls.contains(&DjCall::Nudge { deck: 0, amount: 0.0 }));
        assert!(calls.contains(&DjCall::Cue { deck: 0, pressed: false }));
        assert!(calls.contains(&DjCall::Pause(0)));
        assert!(calls.contains(&DjCall::Set { control: "rate", deck: 0, value: 1.001 }));
        // nothing is left held
        assert!(k.tick(t0 + ms(100)).is_empty());
        k.on_key(&rel(']'), t0 + ms(120));
        assert!(k.tick(t0 + ms(130)).is_empty());
    }

    #[test]
    fn stale_hold_is_released_when_events_stop() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // a playing deck: [ ] bend it (a paused one they roll)
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press(']'), t0);
        k.tick(t0);
        assert!(k.tick(t0 + ms(2000)).is_empty());
        assert_eq!(k.tick(t0 + ms(3100)), vec![DjCall::Nudge { deck: 0, amount: 0.0 }]);
    }

    #[test]
    fn pitch_tap_is_a_tenth_of_a_percent() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('+'), t0);
        assert_eq!(sets(&k.tick(t0)), vec![("rate", 0, 1.001)]);
        k.on_key(&rel('+'), t0 + ms(60));
        k.on_key(&press('l'), t0 + ms(300));
        k.on_key(&press('-'), t0 + ms(500));
        assert_eq!(sets(&k.tick(t0 + ms(500))), vec![("rate", 1, 0.999)]);
    }

    #[test]
    fn pitch_hold_is_slow() {
        let t0 = Instant::now();
        let mut k = keys(true);
        k.on_key(&press('+'), t0);
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
            k.on_key(&press('+'), t0 + ms(i * 33));
        }
        let v = sets(&k.tick(t0 + ms(400)));
        assert_eq!(v.len(), 1);
        assert!((v[0].2 - 1.01).abs() < 1e-9, "{v:?}");
    }

    #[test]
    fn opening_hints_ends_holds_so_no_release_is_lost() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // a playing deck: [ ] bend it (a paused one they roll)
        k.set_snapshot(&[deck(true), deck(true)], &MixerInfo::default());
        k.on_key(&press(']'), t0);
        assert_eq!(k.tick(t0), vec![DjCall::Nudge { deck: 0, amount: 1.0 }]);
        tap_key(&mut k, 'f', t0 + ms(100));
        assert_eq!(k.tick(t0 + ms(100)), vec![DjCall::Nudge { deck: 0, amount: 0.0 }]);
        // the release arrives while the labels are up: harmless
        k.on_key(&rel(']'), t0 + ms(200));
        assert!(k.tick(t0 + ms(200)).is_empty());
    }

    #[test]
    fn nothing_selected_says_so_instead_of_resetting() {
        let t0 = Instant::now();
        let mut k = keys(true);
        assert_eq!(k.on_key(&press('\\'), t0), Some(Nav::NoSelection));
        assert!(k.tick(t0).is_empty());
        assert!(!k.wheel(1, t0));
        assert!(!k.press(t0));
    }

    #[test]
    fn esc_goes_hints_then_selection() {
        let t0 = Instant::now();
        let mut k = keys(true);
        let t = select(&mut k, "w", t0);
        assert!(k.hints().selected.is_some());
        assert_eq!(k.on_key(&esc(), t), None);
        assert!(k.hints().selected.is_none());
        assert_eq!(k.on_key(&esc(), t + ms(100)), None);
        // with the hints up, Esc only closes them
        tap_key(&mut k, 'f', t + ms(200));
        assert!(k.hints().active);
        assert_eq!(k.on_key(&esc(), t + ms(300)), None);
        assert!(!k.hints().active);
    }

    #[test]
    fn loop_length_walks_the_ladder_and_feeds_v() {
        let t0 = Instant::now();
        let mut k = keys(true);
        // loop length of A: 4 -> 8 -> 16, then v loops 16 beats
        let mut t = select(&mut k, "ca", t0);
        tap_key(&mut k, ']', t);
        t += ms(200);
        tap_key(&mut k, ']', t);
        t += ms(200);
        assert_eq!(k.loop_len(), [16.0, 4.0]);
        // the selection is still the loop length: edit keys, not deck keys, so leave it first
        k.on_key(&esc(), t);
        t += ms(200);
        tap_key(&mut k, 'v', t);
        assert_eq!(k.tick(t), vec![DjCall::Loop { deck: 0, beats: 16.0 }]);
        // halve until the bottom: never below 1/4
        let mut t = select(&mut k, "ca", t + ms(500));
        for _ in 0..8 {
            t += ms(200);
            tap_key(&mut k, '[', t);
        }
        assert_eq!(k.loop_len()[0], 0.25);
        // reset: 4 beats again
        tap_key(&mut k, '\\', t + ms(200));
        assert_eq!(k.loop_len(), [4.0, 4.0]);
    }
}
