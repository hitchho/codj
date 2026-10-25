//! Vimium-style control hints: every adjustable control of the DJ screen gets a short label, a
//! typed label selects the control, and the selected control is then edited with `]` `[` `\`, the
//! mouse wheel or a hardware knob (see `knob`). Everything here is pure.
//!
//! Labels use the left hand only, so the right hand can stay on the mouse or the knob:
//! `a s d q w e r t z x c v g b`. The seven most used controls get one letter, the rest two. The
//! table is fixed (not recomputed from what is on screen) so a label never moves with the window
//! size; a control the screen cannot show at the moment just does not match.

use std::time::{Duration, Instant};

use super::keys::{Ctl, Deck, Slot};

/// The letters labels are made of
#[cfg(test)]
pub const LETTERS: &str = "asdqwertzxcvgb";

/// Beat counts for the loop length (1/4 .. 32, `]` doubles, `[` halves)
pub const LADDER: [f64; 8] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// Loop length before anyone changes it
pub const DEFAULT_BEATS: f64 = 4.0;
/// `sync_range` of codjd until changed (not readable from the daemon)
pub const DEFAULT_SYNC_RANGE: f64 = 0.16;

/// Consecutive detents closer than this speed an encoder up
pub const ACCEL_WINDOW: Duration = Duration::from_millis(60);
/// Cap of the step multiplier
pub const ACCEL_MAX: u32 = 4;

/// Something the hints can select and edit
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// a value sent with `DjSet`
    Slot(Slot),
    /// beats of the next loop made with `v`
    LoopLen(Deck),
    /// where the deck is in its track: the knob and `[` `]` scrub it, `:124` jumps
    Time(Deck),
    /// a knob of a patch slot: (slot 0..3, index in the slot's knob list), sent as a `knob` action
    Knob(u8, u8),
}

/// Knobs per slot that get a hint label
pub const KNOBS_LABELLED: usize = 9;

/// Patch labels per slot: 9 knobs, then the slot's fader, filter and low. The slot picks the first
/// letter (`t g x c`), so none is a prefix of a deck or mixer label (those use second letters
/// `a s d`, these never do).
const PATCH_LABELS: [[&str; 12]; 4] = [
    ["td", "tq", "tw", "te", "tr", "tt", "tv", "tg", "tb", "tz", "tx", "tc"],
    ["gd", "gq", "gw", "ge", "gr", "gt", "gv", "gg", "gb", "gz", "gx", "gc"],
    ["xd", "xq", "xw", "xe", "xr", "xt", "xv", "xg", "xb", "xz", "xx", "xc"],
    ["cd", "cq", "cw", "ce", "cr", "ct", "cv", "cg", "cb", "cz", "cx", "cc"],
];

/// Deck index of a patch slot in `DjSet` (decks 0 and 1 are A and B)
pub const fn slot_deck(slot: u8) -> Deck {
    2 + slot
}

impl Target {
    /// the single-value shorthand used all over the tables
    const fn ctl(c: Ctl) -> Target {
        Target::Slot(Slot::Ctl(c))
    }

    /// Shown in the status chip: "A filter"
    pub fn name(self) -> String {
        let deck = |d: Deck| match d {
            0 => "A".to_string(),
            1 => "B".to_string(),
            d => format!("P{}", d - 1),
        };
        match self {
            Target::Knob(s, i) => format!("P{} knob {}", s + 1, i + 1),
            Target::Slot(Slot::Ctl(Ctl::Eq(d, 0))) => format!("{} low", deck(d)),
            Target::Slot(Slot::Ctl(Ctl::Eq(d, 1))) => format!("{} mid", deck(d)),
            Target::Slot(Slot::Ctl(Ctl::Eq(d, _))) => format!("{} high", deck(d)),
            Target::Slot(Slot::Ctl(Ctl::Filter(d))) => format!("{} filter", deck(d)),
            Target::Slot(Slot::Ctl(Ctl::Fader(d))) => format!("{} fader", deck(d)),
            Target::Slot(Slot::Ctl(Ctl::Crossfader)) => "crossfader".into(),
            Target::Slot(Slot::Gain(d)) => format!("{} gain", deck(d)),
            Target::Slot(Slot::Rate(d)) => format!("{} pitch", deck(d)),
            Target::Slot(Slot::Master) => "master".into(),
            Target::Slot(Slot::CueMix) => "cue mix".into(),
            Target::Slot(Slot::SyncRange) => "sync range".into(),
            Target::Slot(Slot::Jog) => "jog step".into(),
            Target::LoopLen(d) => format!("{} loop", deck(d)),
            Target::Time(d) => format!("{} time", deck(d)),
        }
    }

    /// The value as the chip shows it
    pub fn format(self, v: f64) -> String {
        match self {
            Target::Slot(Slot::Ctl(Ctl::Fader(_))) | Target::Slot(Slot::Master) | Target::Slot(Slot::CueMix) => {
                format!("{v:.2}")
            }
            Target::Slot(Slot::Ctl(_)) => format!("{v:+.2}"),
            Target::Slot(Slot::Gain(_)) => format!("{v:+.1} dB"),
            Target::Slot(Slot::Rate(_)) => format!("{:+.2}%", (v - 1.0) * 100.0),
            Target::Slot(Slot::SyncRange) => format!("±{:.0}%", v * 100.0),
            Target::Slot(Slot::Jog) => fmt_jog(crate::dj::keys::JOG_STEPS[(v.max(0.0) as usize).min(crate::dj::keys::JOG_STEPS.len() - 1)]),
            Target::LoopLen(_) => format!("{} beats", fmt_beats(v)),
            Target::Time(_) => crate::dj::render::fmt_time(v),
            Target::Knob(..) => format!("{v:.2}"),
        }
    }
}

/// 0.25 as "1/4", 0.5 as "1/2", whole numbers plain
pub fn fmt_beats(b: f64) -> String {
    if (b - 0.25).abs() < 1e-9 {
        "1/4".into()
    } else if (b - 0.5).abs() < 1e-9 {
        "1/2".into()
    } else if (b - b.round()).abs() < 1e-9 {
        format!("{}", b.round() as i64)
    } else {
        format!("{b:.2}")
    }
}

/// Moves `by` notches along [`LADDER`] (an off-ladder value snaps to the nearest first)
pub fn ladder_step(current: f64, by: i32) -> f64 {
    let nearest = LADDER
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - current).abs().total_cmp(&(b.1 - current).abs()))
        .map_or(0, |(i, _)| i);
    LADDER[(nearest as i32 + by).clamp(0, LADDER.len() as i32 - 1) as usize]
}

/// Every control with its label, most used first (the one-letter labels)
pub fn table() -> Vec<(Target, &'static str)> {
    use Ctl::{Crossfader, Eq, Fader, Filter};
    let c = Target::ctl;
    let s = Target::Slot;
    let mut t = vec![
        (c(Filter(0)), "a"),
        (c(Filter(1)), "s"),
        (c(Eq(0, 0)), "d"),
        (c(Eq(1, 0)), "q"),
        (c(Crossfader), "w"),
        (c(Fader(0)), "e"),
        (c(Fader(1)), "r"),
        // two letters: the first picks the kind, the second the deck (a = A, s = B)
        (c(Eq(0, 1)), "ta"),
        (c(Eq(1, 1)), "ts"),
        (c(Eq(0, 2)), "ga"),
        (c(Eq(1, 2)), "gs"),
        (s(Slot::Rate(0)), "xa"),
        (s(Slot::Rate(1)), "xs"),
        (Target::LoopLen(0), "ca"),
        (Target::LoopLen(1), "cs"),
        (Target::Time(0), "va"),
        (Target::Time(1), "vs"),
        (s(Slot::Gain(0)), "za"),
        (s(Slot::Gain(1)), "zs"),
        // mixer-wide
        (s(Slot::Master), "ba"),
        (s(Slot::CueMix), "bs"),
        (s(Slot::SyncRange), "bd"),
        (s(Slot::Jog), "bq"),
    ];
    // patch slots: knobs, then fader / filter / low (DjSet deck = 2 + slot)
    for (slot, labels) in PATCH_LABELS.iter().enumerate() {
        let slot = slot as u8;
        for (i, label) in labels.iter().enumerate() {
            let target = match i {
                0..=8 => Target::Knob(slot, i as u8),
                9 => c(Fader(slot_deck(slot))),
                10 => c(Filter(slot_deck(slot))),
                _ => c(Eq(slot_deck(slot), 0)),
            };
            t.push((target, *label));
        }
    }
    t
}

/// The label of a control
#[cfg(test)]
pub fn label_of(t: Target) -> Option<&'static str> {
    table().into_iter().find(|(x, _)| *x == t).map(|(_, l)| l)
}

/// Result of typing one more letter while the hints are up
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Typed {
    /// a prefix of one or more labels: keep typing
    Pending,
    /// a whole label
    Selected(Target),
    /// no visible label starts like that: the key is ignored
    Ignored,
}

/// The hint session and the selection (the last two selected controls are remembered)
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// labels are showing and the keys only go to matching
    pub active: bool,
    pub typed: String,
    pub selected: Option<Target>,
    previous: Option<Target>,
}

impl Hints {
    pub fn open(&mut self) {
        self.active = true;
        self.typed.clear();
    }

    pub fn cancel(&mut self) {
        self.active = false;
        self.typed.clear();
    }

    pub fn backspace(&mut self) {
        self.typed.pop();
    }

    /// Makes `t` the selection; the one it replaces becomes the one `;` goes back to
    pub fn select(&mut self, t: Target) {
        if self.selected != Some(t) {
            self.previous = self.selected;
            self.selected = Some(t);
        }
    }

    /// `;`: swaps the selection with the one before it (no-op with fewer than two)
    pub fn toggle(&mut self) -> Option<Target> {
        let prev = self.previous?;
        self.previous = self.selected;
        self.selected = Some(prev);
        self.selected
    }

    /// Esc with no hints up: nothing is selected any more (and `;` has nowhere to go back to)
    pub fn clear(&mut self) {
        self.selected = None;
        self.previous = None;
    }

    /// A letter while the hints are up. `visible`: the controls the screen shows (None = all).
    pub fn letter(&mut self, c: char, visible: Option<&[Target]>) -> Typed {
        let typed: String = self.typed.chars().chain(std::iter::once(c.to_ascii_lowercase())).collect();
        let shown = |t: &Target| visible.map_or(true, |v| v.contains(t));
        let mut longer = false;
        for (t, label) in table() {
            if !shown(&t) || !label.starts_with(&typed) {
                continue;
            }
            if label == typed {
                self.selected_by_label(t);
                return Typed::Selected(t);
            }
            longer = true;
        }
        if longer {
            self.typed = typed;
            Typed::Pending
        } else {
            Typed::Ignored
        }
    }

    fn selected_by_label(&mut self, t: Target) {
        self.select(t);
        self.active = false;
        self.typed.clear();
    }
}

/// Encoder acceleration: detents within [`ACCEL_WINDOW`] of each other in one direction double
/// the step each time (x1, x2, x4), a pause or a reversal starts again at x1
#[derive(Debug, Clone, Default)]
pub struct Accel {
    last: Option<(Instant, i32)>,
    mult: u32,
}

impl Accel {
    /// One detent in direction `dir` (+1 / -1); returns the multiplier for it
    pub fn detent(&mut self, dir: i32, now: Instant) -> u32 {
        let fast = self
            .last
            .is_some_and(|(t, d)| d == dir.signum() && now.saturating_duration_since(t) <= ACCEL_WINDOW);
        self.mult = if fast { (self.mult.max(1) * 2).min(ACCEL_MAX) } else { 1 };
        self.last = Some((now, dir.signum()));
        self.mult
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(label: &str) -> Target {
        table().into_iter().find(|(_, l)| *l == label).unwrap().0
    }

    #[test]
    fn labels_are_left_hand_prefix_free_and_complete() {
        let tab = table();
        assert_eq!(tab.len(), 23 + 48);
        for (_, l) in &tab {
            assert!(matches!(l.len(), 1 | 2), "{l}");
            assert!(l.chars().all(|c| LETTERS.contains(c)), "{l}");
        }
        for (i, (a, la)) in tab.iter().enumerate() {
            for (j, (b, lb)) in tab.iter().enumerate() {
                if i != j {
                    assert!(!lb.starts_with(la), "{la} is a prefix of {lb}");
                    assert!(a != b, "a control twice");
                }
            }
        }
        // the target list covers every control of the screen: 8 per deck and 4 mixer-wide
        let all = |d| {
            vec![
                Target::ctl(Ctl::Eq(d, 0)),
                Target::ctl(Ctl::Eq(d, 1)),
                Target::ctl(Ctl::Eq(d, 2)),
                Target::ctl(Ctl::Filter(d)),
                Target::ctl(Ctl::Fader(d)),
                Target::Slot(Slot::Gain(d)),
                Target::Slot(Slot::Rate(d)),
                Target::LoopLen(d),
                Target::Time(d),
            ]
        };
        for x in all(0).into_iter().chain(all(1)).chain([
            Target::ctl(Ctl::Crossfader),
            Target::Slot(Slot::Master),
            Target::Slot(Slot::CueMix),
            Target::Slot(Slot::SyncRange),
            Target::Slot(Slot::Jog),
        ]) {
            assert!(label_of(x).is_some(), "{x:?}");
        }
        // and every knob and mixer control of the four patch slots
        for slot in 0..4u8 {
            for i in 0..KNOBS_LABELLED as u8 {
                assert!(label_of(Target::Knob(slot, i)).is_some(), "P{} knob {i}", slot + 1);
            }
            for x in [Ctl::Fader(slot_deck(slot)), Ctl::Filter(slot_deck(slot)), Ctl::Eq(slot_deck(slot), 0)] {
                assert!(label_of(Target::ctl(x)).is_some(), "{x:?}");
            }
        }
    }

    #[test]
    fn most_used_controls_get_one_letter() {
        let ones: Vec<_> = table().into_iter().filter(|(_, l)| l.len() == 1).map(|(t, _)| t).collect();
        assert_eq!(
            ones,
            vec![
                Target::ctl(Ctl::Filter(0)),
                Target::ctl(Ctl::Filter(1)),
                Target::ctl(Ctl::Eq(0, 0)),
                Target::ctl(Ctl::Eq(1, 0)),
                Target::ctl(Ctl::Crossfader),
                Target::ctl(Ctl::Fader(0)),
                Target::ctl(Ctl::Fader(1)),
            ]
        );
        // and they come first in the priority order
        assert!(table().iter().take(7).all(|(_, l)| l.len() == 1));
    }

    #[test]
    fn typing_a_label_selects_and_hides() {
        let mut h = Hints::default();
        h.open();
        assert_eq!(h.letter('a', None), Typed::Selected(t("a")));
        assert!(!h.active);
        assert_eq!(h.selected, Some(Target::ctl(Ctl::Filter(0))));
        // two letters: a prefix first
        h.open();
        assert_eq!(h.letter('x', None), Typed::Pending);
        assert_eq!(h.typed, "x");
        assert!(h.active);
        assert_eq!(h.letter('s', None), Typed::Selected(Target::Slot(Slot::Rate(1))));
        assert!(h.typed.is_empty());
        // a letter that starts no label (f, j, or a digit) is ignored, the typed prefix stays
        h.open();
        h.letter('c', None);
        assert_eq!(h.letter('f', None), Typed::Ignored);
        assert_eq!(h.typed, "c");
        assert_eq!(h.letter('S', None), Typed::Selected(Target::LoopLen(1)));
        // backspace
        h.open();
        h.letter('b', None);
        h.backspace();
        assert!(h.typed.is_empty());
    }

    #[test]
    fn hidden_controls_do_not_match() {
        let mut h = Hints::default();
        h.open();
        // only deck B's filter is on screen
        let vis = [Target::ctl(Ctl::Filter(1))];
        assert_eq!(h.letter('a', Some(&vis)), Typed::Ignored);
        assert_eq!(h.letter('x', Some(&vis)), Typed::Ignored);
        assert_eq!(h.letter('s', Some(&vis)), Typed::Selected(Target::ctl(Ctl::Filter(1))));
    }

    #[test]
    fn semicolon_toggles_between_the_last_two() {
        let mut h = Hints::default();
        assert_eq!(h.toggle(), None);
        h.select(t("a"));
        assert_eq!(h.toggle(), None);
        h.select(t("w"));
        assert_eq!(h.selected, Some(t("w")));
        assert_eq!(h.toggle(), Some(t("a")));
        assert_eq!(h.toggle(), Some(t("w")));
        // selecting the current one again changes nothing
        h.select(t("w"));
        assert_eq!(h.toggle(), Some(t("a")));
        // a third selection: the toggle partner is the one selected just before
        h.select(t("d"));
        assert_eq!(h.toggle(), Some(t("a")));
        h.clear();
        assert_eq!(h.selected, None);
        assert_eq!(h.toggle(), None);
    }

    #[test]
    fn acceleration_doubles_up_to_the_cap() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut a = Accel::default();
        assert_eq!(a.detent(1, t0), 1);
        assert_eq!(a.detent(1, t0 + ms(30)), 2);
        assert_eq!(a.detent(1, t0 + ms(60)), 4);
        assert_eq!(a.detent(1, t0 + ms(90)), 4);
        // a pause starts over
        assert_eq!(a.detent(1, t0 + ms(400)), 1);
        // a reversal too, even when fast
        assert_eq!(a.detent(1, t0 + ms(420)), 2);
        assert_eq!(a.detent(-1, t0 + ms(440)), 1);
        // a slow turn never accelerates
        let mut s = Accel::default();
        for i in 0..5 {
            assert_eq!(s.detent(1, t0 + ms(i * 200)), 1);
        }
    }

    #[test]
    fn ladder_steps_and_clamps() {
        assert_eq!(ladder_step(4.0, 1), 8.0);
        assert_eq!(ladder_step(4.0, -1), 2.0);
        assert_eq!(ladder_step(4.0, -3), 0.5);
        assert_eq!(ladder_step(0.25, -1), 0.25);
        assert_eq!(ladder_step(32.0, 4), 32.0);
        assert_eq!(ladder_step(4.0, 4), 32.0);
        // off the ladder: snaps first
        assert_eq!(ladder_step(5.0, 0), 4.0);
        assert_eq!(fmt_beats(0.25), "1/4");
        assert_eq!(fmt_beats(16.0), "16");
    }

    #[test]
    fn value_text() {
        assert_eq!(t("a").format(-0.3), "-0.30");
        assert_eq!(t("e").format(1.0), "1.00");
        assert_eq!(Target::Slot(Slot::Rate(0)).format(1.0123), "+1.23%");
        assert_eq!(Target::Slot(Slot::Gain(0)).format(-2.5), "-2.5 dB");
        assert_eq!(Target::Slot(Slot::SyncRange).format(0.16), "±16%");
        assert_eq!(Target::LoopLen(0).format(0.5), "1/2 beats");
        assert_eq!(t("a").name(), "A filter");
        assert_eq!(t("q").name(), "B low");
        assert_eq!(t("w").name(), "crossfader");
    }
}

/// A jog step: `0.1s`, `2s`
pub fn fmt_jog(seconds: f64) -> String {
    if seconds < 1.0 { format!("{seconds}s") } else { format!("{seconds:.0}s") }
}
