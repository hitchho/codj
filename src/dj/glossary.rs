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
        Target::Slot(Slot::Jog) => "jog step: how far a knob notch rolls",
        Target::LoopLen(_) => "loop length: beats the next v loop repeats",
        Target::Time(_) => "time: spin the knob, bars at a time · :124 = 2:04",
        Target::Knob(..) => "patch knob: ] [ turn it, \\ back to the saved value",
    }
}

/// A line of the `?` overlay: the keys and what they do
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub keys: &'static str,
    pub text: &'static str,
}

const fn e(keys: &'static str, text: &'static str) -> Entry {
    Entry { keys, text }
}

/// The focused deck's keys (`{self}` in a text is its letter, `{other}` the other deck's). The
/// one panel of the overlay: every key acts on the focused deck.
pub const DECK_KEYS: &[Entry] = &[
    e("h l", "focus deck A / B: every key acts on it"),
    e("Space", "play / pause the focused deck"),
    e("c", "cue: a saved spot; hold to hear it in headphones"),
    e("s", "sync: match {self}'s tempo and beats to {other}"),
    e("a b", "load the highlighted track onto deck A / B"),
    e("p", "preview: this deck in the headphones only"),
    e("w ge", "jump 1 beat forward / back (4w: 4 beats)"),
    e("W B", "jump a bar forward / back (2W: 2 bars)"),
    e("} {", "jump a phrase (16 bars) forward / back"),
    e("0 gg", "back to the start of the track"),
    e("gt", "automix: {other} hands over to {self}, beat-matched"),
    e("t{x}", "transition: b s e f o r c k t (16tb: 16 bars)"),
    e("T{x}", "the same, starting at the outro"),
    e("G $", "to the outro: 32 bars before the end"),
    e("m{a-h}", "mark: save a hot cue here (eight of them)"),
    e("'{a-h}", "jump to a mark (` works too)"),
    e("v", "loop 4 beats; 8v loops 8; v again leaves"),
    e("+ -", "pitch up / down 0.1 % (hold to ramp)"),
    e("=", "pitch back to 0"),
    e("R", "reset deck: EQ, filter, fader, pitch"),
    e("[ ]", "bend a playing deck, scrub a paused one"),
    e(".", "repeat the last action on the focused deck"),
    e("^z ^r", "undo: rolls back a transition, or a control"),
    e("4w 2}", "a count goes in front, like in vim"),
];

/// Mixer entries: the mixer is edited through the hints and `:` commands
pub const MIXER_KEYS: &[Entry] = &[
    e("f", "hints: label every control, type one to pick it"),
    e("] [ \\", "step the picked control up / down / reset"),
    e(":fader .5", "fader of the focused deck (0 to 1)"),
    e(":low kill", "low: the bass — keep only one bassline at a time"),
    e(":mid :high", "mid: the body of a track · high: hats and shine"),
    e(":filter -.3", "filter: below 0 muffles, above 0 thins the bass"),
    e(":xfade a", "crossfader: which deck the room hears"),
    e("F1 F2", "headphones: hear deck A / B privately"),
    e("F5", "quantize: presses snap to the beat"),
    e(":delm a", "delete a mark (:delm! all of them)"),
];

pub const GLOBAL_KEYS: &[Entry] = &[
    e("j k ⏎ q", "library: move, load, queue"),
    e("Tab", "browse: Ctrl-d gg G, a b load"),
    e("/", "search the library"),
    e(":", ":help :tutor :mix new · :q leaves"),
    e("F9", "record: your playing becomes code"),
    e("F12", "patch decks: synths, drums, a grid"),
    e("F10 F11", "set editor · AI agent"),
    e("F6 F7 F8", "arm next mix / auto / disarm"),
    e("Esc", "cancel count / pick (never leaves)"),
    e("?", "this key map"),
];

/// An entry with the focused deck's letters filled in
pub fn deck_entry(entry: &Entry, focus: usize) -> (&'static str, String) {
    let (me, other) = if focus == 0 { ("A", "B") } else { ("B", "A") };
    (entry.keys, entry.text.replace("{self}", me).replace("{other}", other))
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
        "Phase is where in the beat a deck is. Two tracks with the same BPM still clash if their beats are offset; the phase meter on each deck and the phase delta in the mixer title show it. Sync fixes it for you; [ ] nudge it by hand.",
    ),
    t(
        "cue",
        &["cue point", "cue button"],
        "a saved spot; hold to hear it in headphones",
        "A cue is a saved spot in a track. On a paused deck c sets it at the playhead and, while held, plays from it so you can listen; releasing jumps back. On a playing deck c returns to it and pauses.",
    ),
    t(
        "hot cue",
        &["hot cues", "hotcue", "hotcues"],
        "eight saved spots to jump to, vim's marks",
        "Hot cues are eight saved spots per deck, and they are the marks of vim: m then a to h saves one at the playhead, ' then the letter jumps to it (a backtick works too), :delm a deletes it and :delm! all of them. Use them for the drop or the intro.",
    ),
    t(
        "loop",
        &["loops", "looping"],
        "repeat a few beats until you leave the loop",
        "A loop repeats a few beats until you press v again: v loops 4 (or the length set with f then the loop label), 8v loops 8, a count while looping resizes it. Use it to stretch an intro or to wait for the next phrase without the track running out.",
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
        "The channel fader is a deck's volume slider, from 0 (silent) to 1 (full). The room hears the deck's fader multiplied by the crossfader's share. :fader 0.5 sets the focused deck's; f e or f r picks it for ] [.",
    ),
    t(
        "crossfader",
        &["xf", "xfade", "crossfade"],
        "which deck the room hears: A, B or both",
        "The crossfader blends between the decks: far left is only A, far right only B, the middle both. f w picks it for ] [, or :xfade a, :xfade b, :xfade 0. Many DJs park it in the middle and mix with the faders.",
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
        "automix",
        &["auto mix", "gt"],
        "the player's own beat-matched transitions",
        "Automix mixes one track into the next by itself, like a streaming app's crossfade but on the beat: close tempos get a long blend, further apart a filter blend, a track without a steady grid a plain crossfade. In the normal view it is on by default (:automix toggles it); in DJ mode gt does it from the playing deck into the focused one.",
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
        "Nudge speeds a deck up or down by up to 4 % while you hold [ or ] (with no control picked) and then returns. Use it to slide the beats into line when the phase delta drifts.",
    ),
    t(
        "pitch",
        &["rate", "speed"],
        "play faster or slower; the tempo follows",
        "Pitch changes a deck's speed by a hair per press (- and + move it 0.1 %, 4+ moves 0.4 %, = resets it). It changes the BPM and breaks sync. To match two tempos by hand, make the BPMs equal, then fix the beats with nudge.",
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
        "Beat jump skips along the track keeping the beat phase: w and b move one beat, W and B a bar, } and { a phrase of 16 bars, and a count multiplies (8w is eight beats). Jump back to repeat a bar, forward to skip ahead.",
    ),
    t(
        "patch",
        &["patches", "patch deck", "patch decks", "synth", "slot"],
        "a synth or drum machine that plays with the decks",
        "A patch is a little instrument built from connected nodes: a synth, a drum machine, a bassline. It sits in one of four slots (P1 to P4) that work like decks with their own fader, filter and low, and its sequencer follows the decks' beat grid, so it is always in time. F12 or :patch opens the pane; :patch new d1 makes a drum machine to start from.",
    ),
    t(
        "node",
        &["nodes", "module"],
        "one building block of a patch, wired to others",
        "A node is one building block of a patch: an oscillator, an envelope, a filter, a sequencer, a drum sound. Nodes have inputs and outputs and you wire an output into an input, like patch cables on a modular synth. Everything is one kind of signal: a pitch in Hz, a gate that is 0 or 1, or a level.",
    ),
    t(
        "oscillator",
        &["osc", "vco", "saw", "sine", "pulse", "triangle"],
        "makes a steady tone: sine, saw, pulse, triangle",
        "An oscillator makes a repeating wave, which you hear as a tone: its frequency is the pitch. A sine is pure, a saw is bright and buzzy (the acid bassline sound), a pulse is hollow, a triangle is soft. Feed an oscillator's freq input from a sequencer to play notes.",
    ),
    t(
        "envelope",
        &["adsr", "env", "envelope (adsr)"],
        "ADSR: how a note's loudness rises and fades",
        "An ADSR envelope shapes a note over time: attack (how fast it rises), decay (how fast it falls to the sustain level), sustain (the level it holds while the gate is on) and release (how long it fades after). Multiply it with a tone to make a plucky or a pad sound.",
    ),
    t(
        "patch filter",
        &["lp", "cutoff", "resonance", "reso", "vcf"],
        "cuts highs or lows of a sound; cutoff moves it",
        "A filter inside a patch removes part of the sound: a low-pass keeps the lows and darkens it, a high-pass keeps the highs, a band-pass keeps a band. The cutoff knob moves the edge, resonance (reso) makes it squelch around the edge. Turning cutoff up and down is the acid sound.",
    ),
    t(
        "sequencer",
        &["seq", "step sequencer", "gateseq", "monoseq"],
        "plays a pattern of steps in time with the beat",
        "A sequencer plays a row of steps in time with the beat, usually 16 per bar. A drum sequencer has one row per sound (kick, snare, hat); a note sequencer plays one note per step, the row giving its pitch in the scale. Edit it with the grid: hjkl move, x toggles a step.",
    ),
    t(
        "step",
        &["steps", "cell"],
        "one slot in a sequencer, a sixteenth note",
        "A step is one slot of a sequencer. With 4 steps per beat each step is a sixteenth note and 16 steps make a bar. A step is on (it plays) or off; on a drum row it also has a velocity. The moving column in the grid is the step playing right now.",
    ),
    t(
        "pattern",
        &["patterns"],
        "one of 8 step layouts (A-H) a sequencer can play",
        "A sequencer holds 8 patterns, A to H: eight different layouts of its steps, such as a verse beat, a fill and a breakdown. Capital letters pick the pattern you edit; Enter queues it, and it takes over at the next bar so the change is always in time.",
    ),
    t(
        "gate",
        &["gates", "trigger", "trig"],
        "an on/off signal: a note or a drum hit starts",
        "A gate is a signal that is 1 while a step plays and 0 otherwise. An envelope uses it to know when a note starts and ends, and a drum sound hits every time its gate goes up. Sequencers make gates; a drum sequencer has one gate output per row.",
    ),
    t(
        "velocity",
        &["vel", "accent"],
        "how hard a step hits; 1-9 sets it on a drum row",
        "Velocity is how hard a step is played, from soft to full. In the drum grid 1 to 9 set it on the cell under the cursor (9 is full, 1 is a ghost note), and x then uses the last one. Softer hats between hard kicks make a beat groove.",
    ),
    t(
        "knob",
        &["knobs", "control"],
        "a patch control you can turn: cutoff, level ...",
        "A knob is a control a patch exposes: a filter cutoff, a level, a decay. Press f, type its label (two letters), then ] and [ turn it, \\ puts it back to the saved value. The mouse wheel and a hardware knob turn it too, and sets can automate it.",
    ),
    t(
        "live/cue",
        &["live", "slot live", "cue only"],
        "LIVE: the room hears the slot; CUE: only you",
        "A slot starts as CUE after loading or after a structural edit: only your headphones hear it, so you can tweak it unheard. p sends it LIVE to the room, and p again takes it back. A slot that eats too much processing is muted by itself (a red note) so the decks never glitch.",
    ),
    t(
        "swing",
        &["shuffle", "groove"],
        "pushes every second step late for a shuffled feel",
        "Swing delays every second step a little, so a straight robotic beat starts to bounce. It is the swing setting of the patch's Clock node, from 0 (straight) to 0.75 (heavy). Edit it in the patch's JSON with :patch edit <name>.",
    ),
];

/// Keys of the patch pane, for `?` while it is open
pub const PATCH_KEYS: &[(&str, &str)] = &[
    ("F12 Esc", "open / close the pane (Esc leaves the grid first)"),
    ("1-4  j k", "open the grid of a slot / select a slot"),
    ("Tab", "next sequencer, across slots (Shift+Tab back)"),
    ("hjkl ←↓↑→", "move the cursor on the grid"),
    ("w b 0 $", "next / previous beat, first / last step"),
    ("x", "toggle the step (a note step holds one note)"),
    ("1-9", "velocity of a drum cell: 9 is full (not on notes)"),
    ("A-H", "choose the pattern you edit (Shift + letter)"),
    ("Enter", "queue the edited pattern: it plays at the next bar"),
    ("p", "send the slot LIVE to the room / back to headphones"),
    ("u  Ctrl-r", "undo / redo the slot's edits"),
    (":w [name]", "save the slot's patch (to its own name or a new one)"),
    ("f ] [ \\", "hints: pick a knob or slot fader / filter / low, edit it"),
    (":patch ...", "new load unload live edit import list"),
];

/// The `?` popup while the patch pane is open
pub fn patch_keys_popup() -> (String, Vec<String>) {
    let lines = PATCH_KEYS.iter().map(|(k, t)| format!("{k}  {t}")).collect();
    (" patch pane keys · any key closes ".to_string(), lines)
}

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
        // the global panel is 40 % of a 120 column screen: 35 columns of text
        for entry in GLOBAL_KEYS {
            assert!(entry.text.chars().count() <= 35, "{:?}", entry.text);
        }
        for entry in DECK_KEYS {
            assert!(entry.keys.chars().count() <= 6, "{}", entry.keys);
        }
    }

    #[test]
    fn the_overlay_names_the_keys_of_the_focused_deck() {
        let keys = |entries: &[Entry]| entries.iter().map(|x| x.keys).collect::<Vec<_>>();
        let deck = keys(DECK_KEYS);
        for k in ["h l", "Space", "c", "s", "a b", "w ge", "W B", "} {", "0 gg", "G $", "m{a-h}", "'{a-h}", "v", "+ -", "=", "[ ]", ".", "^z ^r"] {
            assert!(deck.contains(&k), "{k}");
        }
        let sync = DECK_KEYS.iter().find(|e| e.keys == "s").unwrap();
        assert_eq!(deck_entry(sync, 1).1, "sync: match B's tempo and beats to A");
        assert_eq!(deck_entry(sync, 0).1, "sync: match A's tempo and beats to B");
        let mixer = keys(MIXER_KEYS);
        for k in ["f", ":fader .5", ":low kill", ":filter -.3", ":xfade a", "F1 F2", "F5", ":delm a"] {
            assert!(mixer.contains(&k), "{k}");
        }
        let global = keys(GLOBAL_KEYS);
        for k in ["Tab", "/", ":", "F9", "F10 F11", "F12", "F6 F7 F8", "Esc", "?"] {
            assert!(global.contains(&k), "{k}");
        }
        // no key of the old map is listed any more
        for old in ["x", "z", "e r", "a s", "q w", "1-4", "Alt q/a · u/j", "g/b · h/n", "t y"] {
            assert!(!deck.contains(&old) && !mixer.contains(&old), "{old}");
        }
    }

    #[test]
    fn every_term_is_complete() {
        let wanted = [
            "bpm", "beat", "bar", "phrase", "downbeat", "sync", "phase", "cue", "hot cue", "loop", "eq", "low", "mid", "high", "kill",
            "bass swap", "filter", "fader", "crossfader", "headphones", "cue send", "gain", "key", "set", "mix", "recipe", "quantize", "automix",
            "patch", "node", "oscillator", "envelope", "adsr", "patch filter", "sequencer", "step", "pattern", "gate", "velocity", "knob", "live/cue",
            "live", "swing",
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
    fn patch_terms_are_reachable_and_the_patch_keys_fit_a_popup() {
        // no alias of the patch terms steals a word from the DJ terms
        assert_eq!(lookup("filter").unwrap().name, "filter");
        assert_eq!(lookup("cue").unwrap().name, "cue");
        assert_eq!(lookup("cutoff").unwrap().name, "patch filter");
        assert_eq!(lookup("ADSR").unwrap().name, "envelope");
        assert_eq!(lookup("Live").unwrap().name, "live/cue");
        let mut seen = std::collections::HashSet::new();
        for term in TERMS {
            for w in std::iter::once(&term.name).chain(term.aliases) {
                assert!(seen.insert(norm(w)), "{w:?} names two terms");
            }
        }
        let (title, lines) = patch_keys_popup();
        assert!(title.contains("patch"));
        for k in ["hjkl", "x", "1-9", "A-H", "Enter", "p", "Ctrl-r", ":w", "Tab", "f ] ["] {
            assert!(lines.iter().any(|l| l.contains(k)), "{k}");
        }
        assert!(lines.iter().all(|l| l.chars().count() <= 70), "a line is too long for the popup");
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
        // the deck and mixer controls: the patch labels share first letters with them
        let decks: Vec<Target> = labels.iter().filter(|(t, _)| !matches!(t, Target::Knob(..) | Target::Slot(Slot::Ctl(Ctl::Fader(2..) | Ctl::Filter(2..) | Ctl::Eq(2.., _))))).map(|(t, _)| *t).collect();
        assert_eq!(hover("", &labels, None), None);
        assert!(hover("zz", &labels, None).is_none());
        // two letters wait for the deck: the same control on both decks
        let h = hover("t", &labels, Some(&decks)).unwrap();
        assert!(h.starts_with("mid:") && h.contains("TA A mid") && h.contains("TS B mid"), "{h}");
        // one visible match gives its own line
        let only = [labels.iter().find(|(_, l)| *l == "ta").unwrap().0];
        let h = hover("t", &labels, Some(&only)).unwrap();
        assert!(h.starts_with("TA (A mid) — mid:"), "{h}");
        // different controls behind one letter: just the names
        let h = hover("b", &labels, Some(&decks)).unwrap();
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
