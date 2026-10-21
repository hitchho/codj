//! Every explanation the learning features show, in one place so the `?` overlay, the hint
//! status line, `:help <term>`, the tutor and DOCS.md say the same thing. All text is for a
//! beginner who has never DJ'd: short, musical, no jargon without a reason.
//!
//! - [`explain`]: one line per control of the hint table (status line, chip)
//! - [`DECK_KEYS`], [`MIXER_KEYS`], [`GLOBAL_KEYS`]: the key map of the `?` overlay
//! - [`TERMS`]: the glossary behind `:help <term>`
//!
//! Everything here is data or pure functions, tested for coverage and length.

use super::hints::Target;
use super::keys::{Ctl, Slot};

/// Longest explanation shown next to a key or a control
#[cfg(test)]
pub const MAX_LINE: usize = 50;

/// One line about a control of the hint table (every [`Target`] has one: the match is
/// exhaustive, so a new control does not compile without its explanation)
pub fn explain(t: Target) -> &'static str {
    match t {
        Target::Slot(Slot::Ctl(Ctl::Eq(_, 0))) => "low: the bass — keep only one bassline at a time",
        Target::Slot(Slot::Ctl(Ctl::Eq(_, 1))) => "mid: vocals, synths, snares; the body of a track",
        Target::Slot(Slot::Ctl(Ctl::Eq(..))) => "high: hats and shine; lower it to soften a deck",
        Target::Slot(Slot::Ctl(Ctl::Filter(_))) => "filter: below 0 muffles, above 0 thins the bass",
        Target::Slot(Slot::Ctl(Ctl::Fader(_))) => "fader: how loud this deck is in the room",
        Target::Slot(Slot::Ctl(Ctl::Crossfader)) => "crossfader: which deck the room hears",
        Target::Slot(Slot::Gain(_)) => "gain: trim loudness so both decks sound equal",
        Target::Slot(Slot::Rate(_)) => "pitch: speed up or slow down; the tempo follows",
        Target::Slot(Slot::Master) => "master: the volume of everything the room hears",
        Target::Slot(Slot::CueMix) => "cue mix: headphones blend, cued deck vs the room",
        Target::Slot(Slot::SyncRange) => "sync range: the biggest tempo change sync may do",
        Target::LoopLen(_) => "loop length: beats the next loop repeats",
        Target::Jump(_) => "jump size: beats that the jump keys skip",
    }
}

/// A line of the `?` overlay: keys for deck A, keys for deck B (the same for mixer and global
/// entries), the explanation
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub a: &'static str,
    pub b: &'static str,
    pub text: &'static str,
}

const fn e(a: &'static str, b: &'static str, text: &'static str) -> Entry {
    Entry { a, b, text }
}

const fn g(keys: &'static str, text: &'static str) -> Entry {
    Entry { a: keys, b: keys, text }
}

/// Deck panel entries: `{other}` in a text is the other deck's letter
pub const DECK_KEYS: &[Entry] = &[
    e("x", ".", "play / pause this deck"),
    e("z", "/", "cue: a saved spot; hold to hear it in headphones"),
    e("c", ",", "sync: match {self}'s tempo and beats to {other}"),
    e("v", "m", "loop: repeat a few beats; again to leave it"),
    e("e r", "u i", "jump: skip back / forward along the track"),
    e("a s", "k l", "pitch: slower / faster; tempo follows"),
    e("q w", "o p", "nudge: bend tempo while held to line beats up"),
    e("1-4", "7-0", "hot cues: first press saves, then jumps"),
    e("⇧1-4", "⇧7-0", "Shift + the hot cue key clears it"),
    e("<", ">", "load the highlighted library track here"),
    e("F1", "F2", "headphones: hear this deck privately"),
];

/// Mixer entries: keys are written `deck A · deck B`
pub const MIXER_KEYS: &[Entry] = &[
    g("Alt q/a · u/j", "low: the bass — keep only one bassline at a time"),
    g("Alt w/s · i/k", "mid: vocals, synths, snares; the body of a track"),
    g("Alt e/d · o/l", "high: hats and shine; lower it to soften a deck"),
    g("Alt r/f · p/;", "filter: below 0 muffles, above 0 thins the bass"),
    g("g/b · h/n", "faders: how loud each deck is in the room"),
    g("t y", "crossfader: which deck the room hears"),
    g("F5", "quantize: presses snap to the beat"),
];

pub const GLOBAL_KEYS: &[Entry] = &[
    g("f", "hints: pick a control by its label"),
    g("] [ \\", "edit the picked control (wheel too)"),
    g("F9", "record: your playing becomes code"),
    g("F10", "editor: edit the set, vim style"),
    g("F11", "AI: ask an agent to write the set"),
    g("F6 F7 F8", "arm next mix / auto / disarm"),
    g(":", ":help <term> · :tutor · :mix new"),
    g("Tab", "search the library"),
    g("Esc", "close this / clear pick / leave"),
];

/// A deck entry with the deck letters filled in
pub fn deck_entry(entry: &Entry, deck: usize) -> (&'static str, String) {
    let (me, other) = if deck == 0 { ("A", "B") } else { ("B", "A") };
    let keys = if deck == 0 { entry.a } else { entry.b };
    (keys, entry.text.replace("{self}", me).replace("{other}", other))
}

/// A glossary entry for `:help <term>`
pub struct Term {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// the one-liner
    pub short: &'static str,
    /// one to three sentences
    pub long: &'static str,
}

const fn t(name: &'static str, aliases: &'static [&'static str], short: &'static str, long: &'static str) -> Term {
    Term { name, aliases, short, long }
}

pub const TERMS: &[Term] = &[
    t(
        "bpm",
        &["tempo", "beats per minute"],
        "beats per minute: the tempo of a track",
        "BPM is the tempo: how many beats happen in a minute. Dance music sits around 120 to 135. Two tracks blend well when their BPMs are close, within about 6 %; a yellow ? after the BPM on a deck means the tempo may be half, double or 1.5x off (fix it with :grid).",
    ),
    t(
        "beat",
        &["beats"],
        "one pulse of the music; the foot-tap",
        "A beat is one pulse of the music, the thing you tap your foot to. In club music the kick drum usually lands on every beat. The phase meter on a deck lights beats 1 to 4.",
    ),
    t(
        "bar",
        &["bars"],
        "four beats; the unit DJs count in",
        "A bar is four beats, counted 1 2 3 4. DJs count in bars because things change on bar lines: a bass swap, a loop, a new section. Sets are written in bars.",
    ),
    t(
        "phrase",
        &["phrases"],
        "8, 16 or 32 bars: one musical sentence",
        "A phrase is a group of 8, 16 or 32 bars, a musical sentence. Tracks change (breakdown, drop, new bassline) at phrase borders, so start the next track exactly on one and its changes land with the old track's.",
    ),
    t(
        "downbeat",
        &["beat 1", "one"],
        "beat 1 of a bar: the strongest beat",
        "The downbeat is beat 1 of a bar, the strongest beat. When both tracks' downbeats line up, the mix feels locked; the first beat lit in the phase meter is it.",
    ),
    t(
        "sync",
        &["synced"],
        "match a deck's tempo and beats to the other",
        "Sync changes this deck's speed to the other deck's tempo and lines up the beats, and keeps following it until you touch the pitch. It refuses jumps over the sync range (16 % by default): tracks that far apart sound wrong sped up.",
    ),
    t(
        "phase",
        &["phase meter", "phase delta"],
        "where in the beat a track is",
        "Phase is where in the beat a deck is. Two tracks with the same BPM still clash if their beats are offset; the phase meter on each deck and the phase delta in the mixer title show it. Sync fixes it for you; q w / o p nudge it by hand.",
    ),
    t(
        "cue",
        &["cue point", "cue button"],
        "a saved spot; hold to hear it in headphones",
        "A cue is a saved spot in a track. On a paused deck z sets it at the playhead and, while held, plays from it so you can listen; releasing jumps back. On a playing deck z returns to it and pauses.",
    ),
    t(
        "hot cue",
        &["hot cues", "hotcue", "hotcues"],
        "up to four saved spots to jump to",
        "Hot cues are up to four saved spots per deck (1-4 on deck A, 7-0 on B). The first press on an empty one saves the spot, later presses jump to it; Shift plus the key clears it. Use them for the drop or the intro.",
    ),
    t(
        "loop",
        &["loops", "looping"],
        "repeat a few beats until you leave the loop",
        "A loop repeats a few beats (4 by default, change it with f then the loop label) until you press the key again. Use it to stretch an intro or to wait for the next phrase without the track running out.",
    ),
    t(
        "eq",
        &["equalizer", "equaliser"],
        "three tone knobs: low, mid, high",
        "The EQ is three tone knobs per deck: low, mid and high. Turning one down to the bottom removes that band entirely. Most of mixing is managing the lows, so that only one deck has bass at a time.",
    ),
    t(
        "low/mid/high",
        &["low", "mid", "high", "lows", "bass", "treble"],
        "low is the bass, mid the body, high the shine",
        "Low is the kick and bassline, mid is vocals, synths and snares, high is hats and air. Two basslines at once turn to mud and eat the headroom, so keep the low of one deck down while the other plays its bass.",
    ),
    t(
        "kill",
        &["eq kill", "killing"],
        "turn a band all the way down (-1): silence",
        "A kill turns an EQ band all the way down so it is silent. The track coming in is usually brought in with its low killed, then the kill moves to the old track at the bass swap.",
    ),
    t(
        "bass swap",
        &["swap", "swap low", "bassswap"],
        "kill the old bass, bring the new one back",
        "A bass swap, on a bar line: kill the outgoing track's low and bring the incoming track's low back at the same moment. The kick and bassline hand over cleanly instead of fighting. In a set it is `bar 16  swap low`.",
    ),
    t(
        "filter",
        &["flt", "high-pass", "low-pass", "highpass", "lowpass"],
        "one knob that muffles or thins the sound",
        "The filter is one knob per deck: left of centre it muffles the sound (cuts the treble), right of centre it thins it (cuts the bass), 0 is off. Sweeping it is the classic way to fade a track out or build tension.",
    ),
    t(
        "fader",
        &["faders", "channel fader", "volume"],
        "a deck's own volume, 0 to 1",
        "The channel fader is a deck's volume slider, from 0 (silent) to 1 (full). The room hears the deck's fader multiplied by the crossfader's share. g b are A's, h n are B's.",
    ),
    t(
        "crossfader",
        &["xf", "xfade", "crossfade"],
        "which deck the room hears: A, B or both",
        "The crossfader blends between the decks: far left is only A, far right only B, the middle both. t moves it to A, y to B. Many DJs park it in the middle and mix with the faders.",
    ),
    t(
        "headphones/cue",
        &["headphones", "cue send", "hp", "cueing", "monitor", "pfl"],
        "hear a deck privately before the room does",
        "The headphone send (F1 for A, F2 for B) lets you hear a deck on your own output before you bring it in. It only separates from the room when codjd has a second output; with a single output (mode internal) both come out of the same speakers.",
    ),
    t(
        "gain",
        &["trim", "auto-gain", "autogain"],
        "set a track's loudness before its fader",
        "Gain is the trim before the fader: it makes quiet and loud tracks equally loud. codjd sets an auto-gain when a track loads; f then za / zs changes it, \\ goes back to the auto value.",
    ),
    t(
        "key",
        &["camelot", "harmonic", "musical key"],
        "the musical key; neighbours sound good together",
        "The key is shown in the Camelot notation (8A, 5B ...). Tracks in the same key, one number apart (8A with 7A or 9A) or with the other letter (8A with 8B) sound harmonious together; far apart keys clash when the bass is not killed.",
    ),
    t(
        "set",
        &["sets", "set file"],
        "a text file of tracks and the mixes between them",
        "A set is a text file listing tracks, cues and the mixes between them in bars. codjd can run the mixes for you; you edit the file with :e, :edit or F11. :set lists them, :set <name> activates one.",
    ),
    t(
        "mix",
        &["mixes", "transition", "transitions", "blend"],
        "the handover from one track to the next",
        "A mix is the transition from one track to the next. In a set it is a `mix a -> b` block with an action at each bar (kill bass, fade, swap). :mix new <recipe> writes one for the two tracks on your decks.",
    ),
    t(
        "recipe",
        &["recipes"],
        "a ready-made transition shape for :mix new",
        "A recipe is a ready-made transition (a blend, a cut, a filter fade ...). :mix new alone lists them; :mix new <recipe> [bars] turns one into a commented mix block in the active set that you can edit and arm.",
    ),
    t(
        "quantize",
        &["quantise", "snap"],
        "presses snap to the beat grid (F5)",
        "With quantize on (F5) presses snap to the beat grid: a play, cue, loop or hot cue pressed a little late still lands exactly on a beat. Play on a deck starts on the other deck's next beat, phase-aligned.",
    ),
    t(
        "nudge",
        &["bend"],
        "bend the tempo briefly to line beats up",
        "Nudge speeds a deck up or down by up to 4 % while you hold the key (q w on A, o p on B) and then returns. Use it to slide the beats into line when the phase delta drifts.",
    ),
    t(
        "pitch",
        &["rate", "speed"],
        "play faster or slower; the tempo follows",
        "Pitch changes a deck's speed by a hair per press (a s on A, k l on B). It changes the BPM and breaks sync. To match two tempos by hand, make the BPMs equal, then fix the beats with nudge.",
    ),
    t(
        "grid",
        &["beat grid", "beatgrid"],
        "where codjd thinks the beats are",
        "The beat grid is where codjd thinks the beats and bars are. If the tempo detectors disagree a yellow ? shows after the BPM: the grid may be half, double or 1.5x off. :grid a x2, /2, x1.5, /1.5 or reset corrects it (saved per track).",
    ),
    t(
        "jump",
        &["beat jump", "beatjump"],
        "skip forward or back by whole beats",
        "Beat jump skips along the track by the jump size (4 beats by default; f then va / vs changes it), keeping the beat phase. Jump back to repeat a bar, forward to skip ahead.",
    ),
];

/// A term typed by the user: case, spaces, `-`, `_` and `/` are all alike
fn norm(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if matches!(c, '-' | '_' | '/') { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The entry for what the user typed after `:help`: the name or an alias, whole
pub fn lookup(query: &str) -> Option<&'static Term> {
    let q = norm(query);
    if q.is_empty() {
        return None;
    }
    TERMS.iter().find(|t| norm(t.name) == q).or_else(|| TERMS.iter().find(|t| t.aliases.iter().any(|a| norm(a) == q)))
}

/// Entries whose name starts with what was typed, for "did you mean"
pub fn suggest(query: &str) -> Vec<&'static str> {
    let q = norm(query);
    TERMS.iter().filter(|t| !q.is_empty() && (norm(t.name).starts_with(&q) || norm(t.name).contains(&q))).map(|t| t.name).collect()
}

/// `:help` alone: all the names
pub fn term_names() -> Vec<&'static str> {
    TERMS.iter().map(|t| t.name).collect()
}

/// Word-wraps `text` to `width` columns
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let need = cur.chars().count() + usize::from(!cur.is_empty()) + word.chars().count();
        if !cur.is_empty() && need > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// The popup text of `:help <term>`: title and paragraphs (wrapped when drawn)
pub fn help_popup(term: &Term) -> (String, Vec<String>) {
    (format!(" {} · any key closes ", term.name), vec![term.short.to_string(), String::new(), term.long.to_string()])
}

/// `:help` alone: the terms, as popup paragraphs
pub fn index_popup() -> (String, Vec<String>) {
    let names = term_names().join(" · ");
    (
        " :help <term> · any key closes ".to_string(),
        vec![
            "Words a DJ uses, explained for a beginner. Type :help and one of them:".to_string(),
            String::new(),
            names,
            String::new(),
            ":help keys opens the key map (the same as ?). :tutor walks you through a first mix.".to_string(),
        ],
    )
}

/// The status line while the hint labels are up and some letters are typed: what the labels that
/// still match control. One match: its name and explanation; several kinds: the labels with
/// their names; the same control on both decks: the explanation once.
pub fn hover(typed: &str, labels: &[(Target, &'static str)], visible: Option<&[Target]>) -> Option<String> {
    let typed = typed.to_lowercase();
    if typed.is_empty() {
        return None;
    }
    let found: Vec<(Target, &str)> = labels
        .iter()
        .filter(|(t, l)| l.starts_with(&typed) && visible.map_or(true, |v| v.contains(t)))
        .map(|(t, l)| (*t, *l))
        .collect();
    let (first, rest) = found.split_first()?;
    if rest.is_empty() {
        return Some(format!("{} ({}) — {}", first.1.to_uppercase(), first.0.name(), explain(first.0)));
    }
    if rest.iter().all(|(t, _)| explain(*t) == explain(first.0)) {
        let who: Vec<String> = found.iter().map(|(t, l)| format!("{} {}", l.to_uppercase(), t.name())).collect();
        return Some(format!("{}  ({})", explain(first.0), who.join(" · ")));
    }
    let who: Vec<String> = found.iter().map(|(t, l)| format!("{} {}", l.to_uppercase(), t.name())).collect();
    Some(who.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::hints;

    #[test]
    fn every_hint_control_is_explained_briefly() {
        for (target, label) in hints::table() {
            let text = explain(target);
            assert!(!text.is_empty(), "{label}");
            assert!(text.chars().count() <= MAX_LINE, "{label}: {text:?} is {} long", text.chars().count());
        }
    }

    #[test]
    fn every_overlay_entry_is_explained_briefly() {
        for entry in DECK_KEYS.iter().chain(MIXER_KEYS).chain(GLOBAL_KEYS) {
            for deck in 0..2 {
                let (keys, text) = deck_entry(entry, deck);
                assert!(!keys.is_empty() && !text.is_empty());
                assert!(text.chars().count() <= MAX_LINE, "{text:?} is {} long", text.chars().count());
            }
            assert!(!entry.text.is_empty());
        }
        // a deck panel at 120 columns has 56 usable columns: keys column (8) + 48 of text
        // the global panel is 40 % of it: 35 columns of text
        for entry in GLOBAL_KEYS {
            assert!(entry.text.chars().count() <= 35, "{:?}", entry.text);
        }
        for entry in DECK_KEYS {
            assert!(entry.a.chars().count() <= 6 && entry.b.chars().count() <= 6, "{}", entry.a);
            for deck in 0..2 {
                let text = deck_entry(entry, deck).1;
                assert!(text.chars().count() <= 48, "{text:?} is {}", text.chars().count());
            }
        }
    }

    #[test]
    fn the_overlay_names_the_keys_of_the_task() {
        let keys = |deck: usize| DECK_KEYS.iter().map(|x| deck_entry(x, deck).0).collect::<Vec<_>>();
        for k in ["x", "z", "c", "v", "e r", "a s", "q w", "1-4"] {
            assert!(keys(0).contains(&k), "{k}");
        }
        for k in [".", "/", ",", "m", "u i", "k l", "o p", "7-0"] {
            assert!(keys(1).contains(&k), "{k}");
        }
        let sync_a = deck_entry(&DECK_KEYS[2], 0).1;
        let sync_b = deck_entry(&DECK_KEYS[2], 1).1;
        assert_eq!(sync_b, "sync: match B's tempo and beats to A");
        assert_eq!(sync_a, "sync: match A's tempo and beats to B");
        let global: Vec<&str> = GLOBAL_KEYS.iter().map(|x| x.a).collect();
        for k in ["f", "F9", "F10", "F11", ":", "Esc"] {
            assert!(global.contains(&k), "{k}");
        }
    }

    #[test]
    fn every_term_is_complete() {
        let wanted = [
            "bpm", "beat", "bar", "phrase", "downbeat", "sync", "phase", "cue", "hot cue", "loop", "eq", "low", "mid", "high", "kill",
            "bass swap", "filter", "fader", "crossfader", "headphones", "cue send", "gain", "key", "set", "mix", "recipe", "quantize",
        ];
        for w in wanted {
            assert!(lookup(w).is_some(), "no term {w}");
        }
        for term in TERMS {
            assert!(term.short.chars().count() <= MAX_LINE, "{}: short is {}", term.name, term.short.chars().count());
            assert!(!term.long.is_empty() && term.long.len() < 420, "{}", term.name);
            assert!(lookup(term.name).is_some_and(|f| f.name == term.name), "{}", term.name);
        }
    }

    #[test]
    fn lookup_ignores_case_spacing_and_separators() {
        assert_eq!(lookup("Bass  Swap").unwrap().name, "bass swap");
        assert_eq!(lookup("hot-cue").unwrap().name, "hot cue");
        assert_eq!(lookup("LOW/MID/HIGH").unwrap().name, "low/mid/high");
        assert_eq!(lookup("headphones").unwrap().name, "headphones/cue");
        assert!(lookup("").is_none() && lookup("flux capacitor").is_none());
        assert!(suggest("bass").contains(&"bass swap"));
    }

    #[test]
    fn wrapping_keeps_words_whole() {
        let lines = wrap("one two three four five", 9);
        assert_eq!(lines, vec!["one two", "three", "four five"]);
        assert!(wrap("", 10).is_empty());
    }

    #[test]
    fn hover_names_the_matching_controls() {
        let labels = hints::table();
        assert_eq!(hover("", &labels, None), None);
        assert!(hover("zz", &labels, None).is_none());
        // two letters wait for the deck: the same control on both decks
        let h = hover("t", &labels, None).unwrap();
        assert!(h.starts_with("mid:") && h.contains("TA A mid") && h.contains("TS B mid"), "{h}");
        // one visible match gives its own line
        let only = [labels.iter().find(|(_, l)| *l == "ta").unwrap().0];
        let h = hover("t", &labels, Some(&only)).unwrap();
        assert!(h.starts_with("TA (A mid) — mid:"), "{h}");
        // different controls behind one letter: just the names
        let h = hover("b", &labels, None).unwrap();
        assert!(h.contains("BA master") && h.contains("BS cue mix") && h.contains("BD sync range"), "{h}");
    }

    #[test]
    fn help_popup_has_title_and_text() {
        let (title, lines) = help_popup(lookup("bass swap").unwrap());
        assert!(title.contains("bass swap"));
        assert_eq!(lines.len(), 3);
        let (_, index) = index_popup();
        assert!(index.iter().any(|l| l.contains("bass swap") && l.contains("quantize")));
    }
}
