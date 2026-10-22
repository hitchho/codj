//! `:tutor`: a guided first mix with the user's own library, like vimtutor. A small pane at the
//! bottom explains one step at a time (why, the exact keys) and the step completes by itself when
//! the polled deck and mixer state shows it was done; `Tab` skips a step, `Esc` pauses and
//! `:tutor` resumes where it was left (progress lives in the state directory).
//!
//! Everything here is pure: the step conditions are functions over [`DeckInfo`] / [`MixerInfo`]
//! snapshots, the bar math over a deck's position and grid, and the choice of a track pair over
//! tempos the caller fetched.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::dj_types::{DeckInfo, MixerInfo};

/// A finished step stays on screen this long before the next one shows (so a quick succession of
/// done conditions is still read)
pub const MIN_DWELL: Duration = Duration::from_millis(1200);
/// Step 3 gives up waiting for the headphone send after this long: with one output there is
/// nothing to hear, and the point of the step was the explanation
pub const HEAR_SKIP: Duration = Duration::from_secs(45);
/// Bars of a phrase the tutor teaches
pub const PHRASE_BARS: i64 = 16;
/// Tempo window of a proposed pair
pub const TEMPO_WINDOW: f64 = 0.06;
/// Most `GetGrid` calls spent on finding a pair
pub const MAX_GRID_CALLS: usize = 14;
/// A band counts as killed at or below this, and as restored at or above [`RESTORED`]
pub const KILLED: f64 = -0.9;
pub const RESTORED: f64 = -0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Pick,
    PlayA,
    HearB,
    SyncB,
    Phrase,
    BringIn,
    BassSwap,
    FadeOut,
    Done,
}

impl Step {
    pub const ALL: [Step; 9] = [
        Step::Pick,
        Step::PlayA,
        Step::HearB,
        Step::SyncB,
        Step::Phrase,
        Step::BringIn,
        Step::BassSwap,
        Step::FadeOut,
        Step::Done,
    ];
    /// steps with something to do (the last one is the goodbye)
    pub const COUNT: usize = 8;

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Step {
        Step::ALL.get(i).copied().unwrap_or(Step::Done)
    }

    pub fn next(self) -> Step {
        Step::from_index(self.index() + 1)
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Pick => "Choose two tracks",
            Step::PlayA => "Play deck A",
            Step::HearB => "Listen to B in headphones",
            Step::SyncB => "Sync B to A",
            Step::Phrase => "Start B on a phrase",
            Step::BringIn => "Bring B in without its bass",
            Step::BassSwap => "Swap the bass",
            Step::FadeOut => "Fade A out",
            Step::Done => "You mixed two tracks",
        }
    }

    /// One to three sentences of why
    pub fn why(self) -> &'static str {
        match self {
            Step::Pick => "Two tracks blend best when their tempos are close, within 6 %, so neither has to be sped up much. The pair below comes from your library; load them onto the decks.",
            Step::PlayA => "BPM is beats per minute; a beat is the pulse you tap your foot to and four beats make a bar. Watch the phase meter under the title light up beats 1 to 4 and count along.",
            Step::HearB => "DJs listen to the next track in headphones before the room hears it. F2 sends B to your headphones while the crossfader on A keeps B out of the room.",
            Step::SyncB => "Sync changes B's speed to A's tempo and lines up the beats so they stay together. It refuses over 16 %; a yellow ? by a BPM means half or double time (fix: :grid).",
            Step::Phrase => "Tracks are built in phrases of 16 or 32 bars; the bass and drums change at their edges. Start B on a phrase line of A so the changes land together. Quantize (F5) snaps an early press to the beat.",
            Step::BringIn => "Two basslines at once turn to mud, the kicks and basses fight. So B comes in with its low killed (EQ low at -1): only its drums and highs join A.",
            Step::BassSwap => "On a bar line, kill A's low and bring B's low back at the same moment, so one bassline hands over to the other without a clash. In a set this is the line `swap low`.",
            Step::FadeOut => "A has done its job. Take it out slowly, over a few bars, with the crossfader or its fader, then stop it so B plays alone.",
            Step::Done => "That is a mix: tempo matched, started on a phrase, the bass swapped, the old track faded out. Do it again with other tracks until the hands know it.",
        }
    }

    /// The exact keys
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Step::Pick => &["Tab  browse the library: j k move, a loads on A, b on B", "/  searches it"],
            Step::PlayA => &["h Space  focus A and play it (loading a deck focuses it; Space again pauses)"],
            Step::HearB => &["F2  headphones on B (F1 for A)", ":xfade a  crossfader to A: only A in the room"],
            Step::SyncB => &["l s  focus B and sync it to A (h s syncs A to B)"],
            Step::Phrase => &["Space  play B (it has the focus) on the phrase line (bar 1, 17, 33 ...)"],
            Step::BringIn => &["f q  pick B's low · [ down to -1 (wheel, knob), or :low b kill", ":xfade 0  crossfader to the middle"],
            Step::BassSwap => &[":low a kill  A's low down", ":low b flat  B's low back (or f q then \\ puts it back to 0)"],
            Step::FadeOut => &[":xfade b  crossfader to B  (or :fader a 0: A's fader down)", "h Space  stop A (h focuses A)"],
            Step::Done => &["F9  record your next one · :mix new blend · :help bass swap"],
        }
    }
}

/// What step `step` needs to be done, from the polled state. `in_step` is how long the step has
/// been showing (step 3 gives up after [`HEAR_SKIP`]).
pub fn is_done(step: Step, decks: &[DeckInfo], mixer: &MixerInfo, in_step: Duration) -> bool {
    let (Some(a), Some(b)) = (decks.first(), decks.get(1)) else { return false };
    match step {
        Step::Pick => a.loaded && b.loaded && a.path != b.path,
        Step::PlayA => a.playing,
        Step::HearB => b.headphones || in_step >= HEAR_SKIP,
        Step::SyncB => b.synced,
        Step::Phrase => b.playing,
        Step::BringIn => b.eq.0 <= KILLED,
        Step::BassSwap => a.eq.0 <= KILLED && b.eq.0 >= RESTORED,
        Step::FadeOut => !a.playing && (mixer.crossfader >= 0.5 || a.fader <= 0.1),
        Step::Done => false,
    }
}

/// A deck's beat grid as `GetGrid` reports it: beat `k` is at `first_beat_s + k * 60 / bpm`, a
/// bar's beat 1 when `k % beats_per_bar == downbeat_offset`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub first_beat_s: f64,
    pub downbeat_offset: u32,
    pub beats_per_bar: u32,
}

/// (bar from 1, beat in the bar from 1) of a position. Without a grid reply the first beat is
/// taken at 0 s and bars are four beats.
pub fn bar_of(position: f64, track_bpm: f64, grid: Option<Grid>) -> Option<(i64, i64)> {
    if track_bpm <= 0.0 || !track_bpm.is_finite() {
        return None;
    }
    let g = grid.unwrap_or(Grid { first_beat_s: 0.0, downbeat_offset: 0, beats_per_bar: 4 });
    let per_bar = i64::from(g.beats_per_bar.max(1));
    let k = ((position - g.first_beat_s) * track_bpm / 60.0).floor() as i64;
    let rel = k - i64::from(g.downbeat_offset);
    Some((rel.div_euclid(per_bar) + 1, rel.rem_euclid(per_bar) + 1))
}

/// Where a bar stands against the next phrase line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseCue {
    /// bars until the next phrase line, and its bar number
    Wait { bars_left: i64, next_bar: i64 },
    /// the last beats of the bar before the line: press now
    Ready { next_bar: i64 },
    /// the first beats of the phrase: press now, or wait for the next one
    Now { next_bar: i64 },
}

/// `bar` / `beat` as from [`bar_of`], phrases of [`PHRASE_BARS`]
pub fn phrase_cue(bar: i64, beat: i64, beats_per_bar: i64) -> PhraseCue {
    let into = (bar - 1).rem_euclid(PHRASE_BARS);
    let next_bar = bar - into + PHRASE_BARS;
    let late = beat > beats_per_bar / 2;
    if into == 0 && !late {
        PhraseCue::Now { next_bar }
    } else if into == PHRASE_BARS - 1 && late {
        PhraseCue::Ready { next_bar }
    } else {
        PhraseCue::Wait { bars_left: next_bar - bar, next_bar }
    }
}

/// Two tempos the tutor accepts as a pair (within [`TEMPO_WINDOW`], both with a grid)
pub fn compatible(a: f64, b: f64) -> bool {
    a > 0.0 && b > 0.0 && (b / a - 1.0).abs() <= TEMPO_WINDOW + 1e-9
}

/// One track of a proposal
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub path: String,
    pub title: String,
    pub bpm: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub a: Pick,
    pub b: Pick,
}

/// The first of `candidates` (in the order given: the most similar first) whose tempo fits the
/// seed's
pub fn choose_pair(seed: Pick, candidates: Vec<Pick>) -> Option<Proposal> {
    let b = candidates.into_iter().find(|c| c.path != seed.path && compatible(seed.bpm, c.bpm))?;
    Some(Proposal { a: seed, b })
}

/// Where the search for a pair stands
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Finding {
    #[default]
    Searching,
    Found(Proposal),
    /// no tempos known (an old codjd, nothing analysed): any two tracks will do
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub version: u32,
    /// index of the step shown last
    pub step: usize,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { version: 1, step: 0 }
    }
}

/// `$XDG_STATE_HOME/codj/tutor.json` (`~/.local/state`), else under the data directory
pub fn state_path() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::data_dir).map(|d| d.join("codj").join("tutor.json"))
}

/// Progress from `path`; a missing or unreadable file is a fresh start
pub fn load(path: &Path) -> Progress {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Progress>(&t).ok())
        .filter(|p| p.version == 1)
        .map(|p| Progress { step: p.step.min(Step::Done.index()), ..p })
        .unwrap_or_default()
}

pub fn save(path: &Path, progress: &Progress) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(progress).unwrap_or_default())?;
    std::fs::rename(tmp, path)
}

pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// What the pane draws
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TutorView {
    /// 1-based, of [`Step::COUNT`]; 0 on the goodbye
    pub n: usize,
    pub title: String,
    pub why: String,
    pub keys: Vec<String>,
    /// the step's live lines: what the screen shows now
    pub live: Vec<String>,
    pub footer: String,
}

/// The state of the tutor
pub struct Tutor {
    pub step: Step,
    pub visible: bool,
    entered: Instant,
    path: Option<PathBuf>,
    pub finding: Finding,
}

impl Tutor {
    /// Resumes from `path` (or starts at the first step), shown at once
    pub fn open(path: Option<PathBuf>, now: Instant) -> Tutor {
        let step = path.as_deref().map(load).unwrap_or_default().step;
        Tutor { step: Step::from_index(step), visible: true, entered: now, path, finding: Finding::Searching }
    }

    /// Starts over, forgetting what was saved
    pub fn restart(&mut self, now: Instant) {
        if let Some(p) = &self.path {
            forget(p);
        }
        self.goto(Step::Pick, now);
        self.visible = true;
        self.finding = Finding::Searching;
    }

    /// `:tutor` again: show it, resuming at the saved step
    pub fn resume(&mut self, now: Instant) {
        self.visible = true;
        self.entered = now;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.persist();
    }

    fn goto(&mut self, step: Step, now: Instant) {
        self.step = step;
        self.entered = now;
        self.persist();
    }

    fn persist(&self) {
        if let Some(p) = &self.path {
            let _ = save(p, &Progress { version: 1, step: self.step.index() });
        }
    }

    /// `Tab`: the step is skipped (the goodbye closes the pane)
    pub fn skip(&mut self, now: Instant) {
        if self.step == Step::Done {
            self.close();
        } else {
            self.goto(self.step.next(), now);
        }
    }

    /// Looks at the polled state: when the step's condition holds, and the step has been showing
    /// for at least [`MIN_DWELL`], the next step shows. Returns the step that was completed.
    pub fn update(&mut self, decks: &[DeckInfo], mixer: &MixerInfo, now: Instant) -> Option<Step> {
        if !self.visible || self.step == Step::Done {
            return None;
        }
        let shown = now.saturating_duration_since(self.entered);
        if shown < MIN_DWELL || !is_done(self.step, decks, mixer, shown) {
            return None;
        }
        let done = self.step;
        self.goto(done.next(), now);
        Some(done)
    }

    /// The pane's text for the current state
    pub fn view(&self, decks: &[DeckInfo], mixer: &MixerInfo, grids: [Option<Grid>; 2]) -> TutorView {
        TutorView {
            n: if self.step == Step::Done { 0 } else { self.step.index() + 1 },
            title: self.step.title().to_string(),
            why: self.step.why().to_string(),
            keys: self.step.keys().iter().map(|k| k.to_string()).collect(),
            live: live_lines(self.step, decks, mixer, grids, &self.finding),
            footer: if self.step == Step::Done { "Esc closes · :tutor reset starts over".into() } else { ":tutor next skips · Esc pauses · :tutor resumes".into() },
        }
    }
}

fn bpm(d: &DeckInfo) -> String {
    if d.has_grid() { format!("{:.1}", d.bpm) } else { "no grid".into() }
}

/// What the screen shows now, for the step: the checklist of the state it is waiting for
pub fn live_lines(step: Step, decks: &[DeckInfo], mixer: &MixerInfo, grids: [Option<Grid>; 2], finding: &Finding) -> Vec<String> {
    let blank = DeckInfo::default();
    let a = decks.first().unwrap_or(&blank);
    let b = decks.get(1).unwrap_or(&blank);
    let tick = |ok: bool| if ok { "✓" } else { "·" };
    match step {
        Step::Pick => {
            let mut lines = Vec::new();
            match finding {
                Finding::Searching => lines.push("looking for two tracks with close tempos…".to_string()),
                Finding::Found(p) => {
                    lines.push(format!("★A {} ({:.1} BPM)", p.a.title, p.a.bpm));
                    lines.push(format!("★B {} ({:.1} BPM)", p.b.title, p.b.bpm));
                }
                Finding::Nothing => lines.push("no tempos to compare: pick any two tracks".to_string()),
            }
            lines.push(format!("{} A loaded   {} B loaded", tick(a.loaded), tick(b.loaded)));
            lines
        }
        Step::PlayA => {
            let mut lines = vec![format!("{} A playing", tick(a.playing))];
            if let Some((bar, beat)) = bar_of(a.position, a.track_bpm, grids[0]) {
                lines.push(format!("A: {} BPM, bar {bar}, beat {beat}", bpm(a)));
            }
            lines
        }
        Step::HearB => {
            let mut lines = vec![format!("{} headphones on B (HP ● on deck B)", tick(b.headphones))];
            if mixer.mode == "internal" {
                lines.push("one output (internal mode): you will not hear B apart from the room; just see HP ●".into());
            }
            lines
        }
        Step::SyncB => {
            let mut lines = vec![format!("{} B synced   A {} BPM, B {} BPM", tick(b.synced), bpm(a), bpm(b))];
            if a.has_grid() && b.has_grid() && !b.synced && (b.track_bpm / a.track_bpm - 1.0).abs() > 0.16 {
                lines.push("over 16 % apart: sync will refuse; load another track or :grid b x2 | /2".into());
            }
            lines
        }
        Step::Phrase => {
            let mut lines = vec![format!("{} B playing", tick(b.playing))];
            match bar_of(a.position, a.track_bpm, grids[0]) {
                Some((bar, beat)) => {
                    let per_bar = grids[0].map_or(4, |g| i64::from(g.beats_per_bar.max(1)));
                    lines.push(match phrase_cue(bar, beat, per_bar) {
                        PhraseCue::Wait { bars_left, next_bar } => {
                            format!("A is on bar {bar}, beat {beat}: the phrase line is bar {next_bar}, in {bars_left} bar{}", if bars_left == 1 { "" } else { "s" })
                        }
                        PhraseCue::Ready { next_bar } => format!("A: bar {bar}, beat {beat}  ► press Space now: bar {next_bar} starts a phrase"),
                        PhraseCue::Now { next_bar } => format!("A: bar {bar}, beat {beat}  ► this is the phrase line: press Space now (next: bar {next_bar})"),
                    });
                }
                None => lines.push("A has no beat grid: count bars yourself, 4 beats each".into()),
            }
            if !a.playing {
                lines.push("A is not playing: x starts it".into());
            }
            lines
        }
        Step::BringIn => {
            let mut lines = vec![format!("{} B low {:+.2} (needs -1)", tick(b.eq.0 <= KILLED), b.eq.0)];
            if b.fader < 0.3 {
                lines.push("B's fader is down: :fader b full raises it".into());
            }
            if mixer.crossfader <= -0.9 {
                lines.push("the crossfader is on A, the room cannot hear B: :xfade 0 or :xfade b".into());
            }
            lines
        }
        Step::BassSwap => vec![
            format!("{} A low {:+.2} (needs -1)", tick(a.eq.0 <= KILLED), a.eq.0),
            format!("{} B low {:+.2} (needs 0)", tick(b.eq.0 >= RESTORED), b.eq.0),
        ],
        Step::FadeOut => {
            let faded = mixer.crossfader >= 0.5 || a.fader <= 0.1;
            vec![format!("{} A faded out   {} A stopped", tick(faded), tick(!a.playing))]
        }
        Step::Done => Vec::new(),
    }
}

/// Finds a track pair in a library of (path, title): `fetch` returns a path's tempo (None = no
/// grid). The first tracks are tried as the seed (at most 6), then up to 8 candidates, the ones
/// `similar` names first and the next tracks of the library after them; never more than
/// [`MAX_GRID_CALLS`] lookups in all.
pub fn find_pair(
    library: &[(String, String)],
    similar: impl FnOnce(&str) -> Vec<usize>,
    mut fetch: impl FnMut(&str) -> Option<f64>,
) -> Option<Proposal> {
    let mut calls = 0usize;
    let mut tempo = |path: &str, calls: &mut usize| {
        if *calls >= MAX_GRID_CALLS {
            return None;
        }
        *calls += 1;
        fetch(path).filter(|b| *b > 0.0)
    };
    let (seed_at, seed_bpm) = (0..library.len().min(6)).find_map(|i| tempo(&library[i].0, &mut calls).map(|b| (i, b)))?;
    let seed = Pick { path: library[seed_at].0.clone(), title: library[seed_at].1.clone(), bpm: seed_bpm };
    let mut order: Vec<usize> = similar(&seed.path).into_iter().filter(|i| *i < library.len() && *i != seed_at).collect();
    for i in 0..library.len() {
        if i != seed_at && !order.contains(&i) {
            order.push(i);
        }
    }
    let mut found = Vec::new();
    for i in order.into_iter().take(8) {
        let Some(bpm) = tempo(&library[i].0, &mut calls) else { continue };
        let pick = Pick { path: library[i].0.clone(), title: library[i].1.clone(), bpm };
        if compatible(seed.bpm, bpm) {
            found.push(pick);
            break;
        }
    }
    choose_pair(seed, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck(f: impl FnOnce(&mut DeckInfo)) -> DeckInfo {
        let mut d = DeckInfo { loaded: true, path: "/m/x.flac".into(), fader: 1.0, rate: 1.0, ..Default::default() };
        f(&mut d);
        d
    }

    fn mixer(xf: f64) -> MixerInfo {
        MixerInfo { crossfader: xf, mode: "internal".into(), ..Default::default() }
    }

    const SHORT: Duration = Duration::from_secs(2);

    /// the new keys really do what each step asks: typed into the key machinery they produce
    /// the calls that make the step's condition true
    #[test]
    fn every_step_can_be_done_with_the_keys_it_names() {
        use crate::dj::keys::{DjCall, DjKeys};
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let press = |c: KeyCode| KeyEvent { code: c, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE };
        let t0 = Instant::now();
        let at = std::cell::Cell::new(0u64);
        let mut keys = DjKeys::new(true);
        let mut decks = vec![deck(|d| d.path = "/m/a.flac".into()), deck(|d| d.path = "/m/b.flac".into())];
        let mut m = mixer(0.0);
        let type_keys = |keys: &mut DjKeys, text: &str| -> Vec<DjCall> {
            for c in text.chars() {
                at.set(at.get() + 150);
                keys.on_key(&press(KeyCode::Char(c)), t0 + Duration::from_millis(at.get()));
            }
            at.set(at.get() + 150);
            keys.tick(t0 + Duration::from_millis(at.get()))
        };
        let apply = |calls: &[DjCall], decks: &mut Vec<DeckInfo>, m: &mut MixerInfo| {
            for c in calls {
                match c {
                    DjCall::Play(d) => decks[*d as usize].playing = true,
                    DjCall::Pause(d) => decks[*d as usize].playing = false,
                    DjCall::Sync(d) => decks[*d as usize].synced = true,
                    DjCall::Set { control: "cue", deck, value } => decks[*deck as usize].headphones = *value > 0.5,
                    DjCall::Set { control: "low", deck, value } => decks[*deck as usize].eq.0 = *value,
                    DjCall::Set { control: "crossfader", value, .. } => m.crossfader = *value,
                    DjCall::Set { control: "fader", deck, value } => decks[*deck as usize].fader = *value,
                    _ => {}
                }
            }
        };
        let long = Duration::from_secs(60);
        // 2: h Space plays A, whichever deck was loaded last
        keys.set_focus(1);
        let calls = type_keys(&mut keys, "h ");
        assert_eq!(calls, vec![DjCall::Play(0)]);
        apply(&calls, &mut decks, &mut m);
        assert!(is_done(Step::PlayA, &decks, &m, long));
        // 3: F2 for headphones on B
        keys.on_key(&press(KeyCode::F(2)), t0 + Duration::from_millis(at.get() + 150));
        let calls = keys.tick(t0 + Duration::from_millis(at.get() + 300));
        at.set(at.get() + 300);
        apply(&calls, &mut decks, &mut m);
        assert!(is_done(Step::HearB, &decks, &m, long) && decks[1].headphones);
        // 4: l s syncs B to A
        keys.set_snapshot(&decks, &m);
        let calls = type_keys(&mut keys, "ls");
        assert_eq!(calls, vec![DjCall::Sync(1)]);
        apply(&calls, &mut decks, &mut m);
        assert!(is_done(Step::SyncB, &decks, &m, long));
        // 5: Space plays B, which has the focus
        keys.set_snapshot(&decks, &m);
        let calls = type_keys(&mut keys, " ");
        assert_eq!(calls, vec![DjCall::Play(1)]);
        apply(&calls, &mut decks, &mut m);
        assert!(is_done(Step::Phrase, &decks, &m, long));
        // 6: f q picks B's low, [ takes it down to -1
        keys.set_snapshot(&decks, &m);
        let calls = type_keys(&mut keys, "fq");
        assert!(calls.is_empty());
        let mut last = Vec::new();
        for _ in 0..22 {
            last = type_keys(&mut keys, "[");
            apply(&last, &mut decks, &mut m);
            keys.set_snapshot(&decks, &m);
        }
        assert!(!last.is_empty() || decks[1].eq.0 <= KILLED);
        assert!(is_done(Step::BringIn, &decks, &m, long), "{:?}", decks[1].eq);
        // 7: f d picks A's low and takes it down; B's low goes back with \ after f q
        type_keys(&mut keys, "fd");
        for _ in 0..22 {
            let calls = type_keys(&mut keys, "[");
            apply(&calls, &mut decks, &mut m);
            keys.set_snapshot(&decks, &m);
        }
        type_keys(&mut keys, "fq");
        let calls = type_keys(&mut keys, "\\");
        apply(&calls, &mut decks, &mut m);
        keys.set_snapshot(&decks, &m);
        assert!(is_done(Step::BassSwap, &decks, &m, long), "{:?} {:?}", decks[0].eq, decks[1].eq);
        // 8: the crossfader to B (as :xfade b does), then h and Space stop A
        keys.set_control(crate::dj::keys::Slot::Ctl(crate::dj::keys::Ctl::Crossfader), 1.0, t0 + Duration::from_millis(at.get() + 400));
        let calls = keys.tick(t0 + Duration::from_millis(at.get() + 600));
        at.set(at.get() + 600);
        apply(&calls, &mut decks, &mut m);
        let calls = type_keys(&mut keys, "h ");
        assert_eq!(calls, vec![DjCall::Pause(0)]);
        apply(&calls, &mut decks, &mut m);
        assert!(is_done(Step::FadeOut, &decks, &m, long));
    }

    #[test]
    fn step_one_needs_two_different_loaded_tracks() {
        let m = mixer(0.0);
        let empty = DeckInfo::default();
        let a = deck(|d| d.path = "/m/a.flac".into());
        let b = deck(|d| d.path = "/m/b.flac".into());
        assert!(!is_done(Step::Pick, &[a.clone(), empty.clone()], &m, SHORT));
        assert!(!is_done(Step::Pick, &[empty, b.clone()], &m, SHORT));
        assert!(is_done(Step::Pick, &[a.clone(), b], &m, SHORT));
        // the same file on both decks is not a pair
        assert!(!is_done(Step::Pick, &[a.clone(), a], &m, SHORT));
        assert!(!is_done(Step::Pick, &[], &m, SHORT));
    }

    #[test]
    fn steps_two_to_five_watch_play_headphones_and_sync() {
        let m = mixer(0.0);
        let idle = deck(|_| {});
        let playing = deck(|d| d.playing = true);
        assert!(!is_done(Step::PlayA, &[idle.clone(), idle.clone()], &m, SHORT));
        assert!(is_done(Step::PlayA, &[playing.clone(), idle.clone()], &m, SHORT));
        // B's playing does not complete the A step
        assert!(!is_done(Step::PlayA, &[idle.clone(), playing.clone()], &m, SHORT));
        let hp = deck(|d| d.headphones = true);
        assert!(!is_done(Step::HearB, &[idle.clone(), idle.clone()], &m, SHORT));
        assert!(is_done(Step::HearB, &[idle.clone(), hp.clone()], &m, SHORT));
        // A's headphones are not B's; without a second output the step gives up by itself
        assert!(!is_done(Step::HearB, &[hp, idle.clone()], &m, SHORT));
        assert!(is_done(Step::HearB, &[idle.clone(), idle.clone()], &m, HEAR_SKIP));
        let synced = deck(|d| d.synced = true);
        assert!(!is_done(Step::SyncB, &[synced.clone(), idle.clone()], &m, SHORT));
        assert!(is_done(Step::SyncB, &[idle.clone(), synced], &m, SHORT));
        assert!(!is_done(Step::Phrase, &[playing.clone(), idle.clone()], &m, SHORT));
        assert!(is_done(Step::Phrase, &[idle, playing], &m, SHORT));
    }

    #[test]
    fn steps_six_and_seven_watch_the_low_bands() {
        let m = mixer(0.0);
        let low = |v: f64| deck(|d| d.eq = (v, 0.0, 0.0));
        // B's low killed (-0.9 and below)
        assert!(!is_done(Step::BringIn, &[low(0.0), low(-0.85)], &m, SHORT));
        assert!(is_done(Step::BringIn, &[low(0.0), low(-0.9)], &m, SHORT));
        assert!(is_done(Step::BringIn, &[low(0.0), low(-1.0)], &m, SHORT));
        // A's low does not count for step 6
        assert!(!is_done(Step::BringIn, &[low(-1.0), low(0.0)], &m, SHORT));
        // the swap: A killed and B back at (nearly) 0
        assert!(!is_done(Step::BassSwap, &[low(0.0), low(-1.0)], &m, SHORT));
        assert!(!is_done(Step::BassSwap, &[low(-1.0), low(-1.0)], &m, SHORT));
        assert!(!is_done(Step::BassSwap, &[low(-0.5), low(0.0)], &m, SHORT));
        assert!(is_done(Step::BassSwap, &[low(-1.0), low(-0.1)], &m, SHORT));
        assert!(is_done(Step::BassSwap, &[low(-0.95), low(0.3)], &m, SHORT));
    }

    #[test]
    fn step_eight_needs_a_fade_and_a_stopped_a() {
        let a = |playing: bool, fader: f64| deck(|d| {
            d.playing = playing;
            d.fader = fader;
        });
        let b = deck(|d| d.playing = true);
        // crossfader toward B or A's fader down, then A stopped
        assert!(!is_done(Step::FadeOut, &[a(true, 1.0), b.clone()], &mixer(1.0), SHORT));
        assert!(!is_done(Step::FadeOut, &[a(false, 1.0), b.clone()], &mixer(0.0), SHORT), "a bare stop is not a fade");
        assert!(is_done(Step::FadeOut, &[a(false, 1.0), b.clone()], &mixer(0.5), SHORT));
        assert!(is_done(Step::FadeOut, &[a(false, 0.1), b.clone()], &mixer(-1.0), SHORT));
        assert!(!is_done(Step::FadeOut, &[a(true, 0.0), b], &mixer(1.0), SHORT));
        // the goodbye is never "done"
        assert!(!is_done(Step::Done, &[a(false, 0.0)], &mixer(1.0), SHORT));
    }

    #[test]
    fn bars_from_position_and_grid() {
        // 120 BPM: a beat every 0.5 s, first beat at 0.25 s, bar 1 starts on beat 0
        let g = Grid { first_beat_s: 0.25, downbeat_offset: 0, beats_per_bar: 4 };
        assert_eq!(bar_of(0.25, 120.0, Some(g)), Some((1, 1)));
        assert_eq!(bar_of(0.26 + 1.5, 120.0, Some(g)), Some((1, 4)));
        assert_eq!(bar_of(0.25 + 2.0, 120.0, Some(g)), Some((2, 1)));
        // bar 33 starts 32 bars = 128 beats = 64 s later
        assert_eq!(bar_of(0.25 + 64.0 + 0.1, 120.0, Some(g)), Some((33, 1)));
        // a downbeat offset: the first bar line is 2 beats after the first beat
        let off = Grid { downbeat_offset: 2, ..g };
        assert_eq!(bar_of(0.25 + 1.0, 120.0, Some(off)), Some((1, 1)));
        assert_eq!(bar_of(0.25, 120.0, Some(off)), Some((0, 3)));
        // without a grid reply: beats/4 from the start
        assert_eq!(bar_of(8.0, 120.0, None), Some((5, 1)));
        assert_eq!(bar_of(0.0, 0.0, None), None);
        // three beats per bar
        let waltz = Grid { beats_per_bar: 3, ..g };
        assert_eq!(bar_of(0.25 + 1.5, 120.0, Some(waltz)), Some((2, 1)));
    }

    #[test]
    fn phrase_cues() {
        use PhraseCue::*;
        assert_eq!(phrase_cue(1, 1, 4), Now { next_bar: 17 });
        assert_eq!(phrase_cue(1, 2, 4), Now { next_bar: 17 });
        // late in the phrase's first bar the next line is a whole phrase away
        assert_eq!(phrase_cue(1, 3, 4), Wait { bars_left: 16, next_bar: 17 });
        assert_eq!(phrase_cue(5, 1, 4), Wait { bars_left: 12, next_bar: 17 });
        assert_eq!(phrase_cue(16, 2, 4), Wait { bars_left: 1, next_bar: 17 });
        assert_eq!(phrase_cue(16, 3, 4), Ready { next_bar: 17 });
        assert_eq!(phrase_cue(16, 4, 4), Ready { next_bar: 17 });
        assert_eq!(phrase_cue(17, 1, 4), Now { next_bar: 33 });
        assert_eq!(phrase_cue(29, 1, 4), Wait { bars_left: 4, next_bar: 33 });
        assert_eq!(phrase_cue(32, 4, 4), Ready { next_bar: 33 });
        // before the first bar line
        assert_eq!(phrase_cue(0, 3, 4), Ready { next_bar: 1 });
        assert_eq!(phrase_cue(0, 1, 4), Wait { bars_left: 1, next_bar: 1 });
    }

    #[test]
    fn tempos_within_six_percent_pair_up() {
        assert!(compatible(128.0, 128.0));
        assert!(compatible(128.0, 128.0 * 1.06));
        assert!(compatible(128.0, 128.0 * 0.94));
        assert!(!compatible(128.0, 128.0 * 1.07));
        assert!(!compatible(128.0, 64.0));
        assert!(!compatible(0.0, 128.0) && !compatible(128.0, 0.0));
    }

    fn lib(n: usize) -> Vec<(String, String)> {
        (0..n).map(|i| (format!("/m/{i}.flac"), format!("T{i}"))).collect()
    }

    #[test]
    fn the_pair_prefers_similar_tracks_and_caps_the_calls() {
        let tempos = [128.0, 90.0, 140.0, 130.0, 127.0, 70.0];
        let calls = std::cell::Cell::new(0usize);
        let fetch = |p: &str| {
            calls.set(calls.get() + 1);
            let i: usize = p.trim_start_matches("/m/").trim_end_matches(".flac").parse().unwrap();
            tempos.get(i).copied()
        };
        // SimilarTo names track 4 before 3: 4 wins although 3 comes first in the library
        let p = find_pair(&lib(6), |_| vec![4, 3], fetch).unwrap();
        assert_eq!((p.a.title.as_str(), p.b.title.as_str()), ("T0", "T4"));
        assert!(p.b.bpm == 127.0 && calls.get() <= MAX_GRID_CALLS);
        // no similarity answer: library order, T3 (130) is the first within 6 %
        let p = find_pair(&lib(6), |_| vec![], |p| {
            let i: usize = p.trim_start_matches("/m/").trim_end_matches(".flac").parse().unwrap();
            tempos.get(i).copied()
        })
        .unwrap();
        assert_eq!(p.b.title, "T3");
        // nobody fits
        assert!(find_pair(&lib(3), |_| vec![], |p| if p.contains("/0.") { Some(128.0) } else { Some(90.0) }).is_none());
        // no tempos at all: no proposal and no more than 6 seed lookups
        let n = std::cell::Cell::new(0usize);
        assert!(find_pair(&lib(40), |_| vec![], |_| { n.set(n.get() + 1); None }).is_none());
        assert_eq!(n.get(), 6);
        // a huge library of tempos that never fit stays under the cap
        let n = std::cell::Cell::new(0usize);
        assert!(find_pair(&lib(500), |_| (1..400).collect(), |p| { n.set(n.get() + 1); if p.contains("/0.") { Some(128.0) } else { Some(60.0) } }).is_none());
        assert!(n.get() <= MAX_GRID_CALLS, "{}", n.get());
        // an empty library
        assert!(find_pair(&[], |_| vec![], |_| Some(128.0)).is_none());
    }

    #[test]
    fn the_seed_skips_tracks_without_a_grid() {
        let p = find_pair(&lib(4), |_| vec![], |p| if p.contains("/0.") { None } else { Some(128.0) }).unwrap();
        assert_eq!((p.a.title.as_str(), p.b.title.as_str()), ("T1", "T2"));
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("codj-tutor-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("state").join("tutor.json")
    }

    #[test]
    fn progress_round_trips_and_survives_garbage() {
        let path = temp("rt");
        assert_eq!(load(&path), Progress::default());
        save(&path, &Progress { version: 1, step: 5 }).unwrap();
        assert_eq!(load(&path).step, 5);
        // a step past the end is clamped, a broken file is a fresh start
        save(&path, &Progress { version: 1, step: 99 }).unwrap();
        assert_eq!(load(&path).step, Step::Done.index());
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), Progress::default());
        std::fs::write(&path, r#"{"version":7,"step":3}"#).unwrap();
        assert_eq!(load(&path), Progress::default());
        forget(&path);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn the_tutor_advances_saves_resumes_and_resets() {
        let path = temp("flow");
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let m = mixer(0.0);
        let a = deck(|d| d.path = "/m/a.flac".into());
        let b = deck(|d| d.path = "/m/b.flac".into());
        let empty = DeckInfo::default();
        let mut t = Tutor::open(Some(path.clone()), t0);
        assert_eq!(t.step, Step::Pick);
        // not done: nothing happens
        assert_eq!(t.update(&[empty.clone(), empty.clone()], &m, at(2000)), None);
        // done, but the step has only just appeared: the dwell holds it back
        assert_eq!(t.update(&[a.clone(), b.clone()], &m, at(100)), None);
        assert_eq!(t.step, Step::Pick);
        assert_eq!(t.update(&[a.clone(), b.clone()], &m, at(1500)), Some(Step::Pick));
        assert_eq!(t.step, Step::PlayA);
        assert_eq!(load(&path).step, Step::PlayA.index());
        // A plays: step 2 completes (after its dwell), the progress follows
        let playing = deck(|d| d.playing = true);
        assert_eq!(t.update(&[playing.clone(), b.clone()], &m, at(1600)), None);
        assert_eq!(t.update(&[playing.clone(), b.clone()], &m, at(3000)), Some(Step::PlayA));
        assert_eq!(t.step, Step::HearB);
        // :tutor next skips; Esc pauses and keeps the step; nothing advances while paused
        t.skip(at(3100));
        assert_eq!(t.step, Step::SyncB);
        t.close();
        assert!(!t.visible);
        let synced = deck(|d| d.synced = true);
        assert_eq!(t.update(&[a.clone(), synced.clone()], &m, at(9000)), None);
        // a new session resumes at the saved step
        let mut again = Tutor::open(Some(path.clone()), at(10_000));
        assert_eq!(again.step, Step::SyncB);
        assert!(again.visible);
        // reset starts over and forgets
        again.restart(at(10_100));
        assert_eq!(again.step, Step::Pick);
        assert_eq!(Tutor::open(Some(path.clone()), at(10_200)).step, Step::Pick);
        // skipping through to the end, the goodbye closes
        for _ in 0..Step::COUNT {
            again.skip(at(11_000));
        }
        assert_eq!(again.step, Step::Done);
        again.skip(at(11_000));
        assert!(!again.visible);
        assert_eq!(Tutor::open(Some(path.clone()), at(12_000)).step, Step::Done);
        forget(&path);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_condition_that_already_holds_advances_after_the_dwell() {
        let t0 = Instant::now();
        let m = mixer(0.0);
        let mut t = Tutor::open(None, t0);
        t.skip(t0);
        assert_eq!(t.step, Step::PlayA);
        let playing = deck(|d| d.playing = true);
        let idle = deck(|_| {});
        // the condition holds from the start: the dwell is all that waits
        assert_eq!(t.update(&[playing.clone(), idle.clone()], &m, t0 + Duration::from_millis(300)), None);
        assert_eq!(t.update(&[playing.clone(), idle.clone()], &m, t0 + Duration::from_secs(5)), Some(Step::PlayA));
    }

    #[test]
    fn every_step_teaches_with_why_keys_and_a_checklist() {
        let m = mixer(0.0);
        let d = [deck(|_| {}), deck(|_| {})];
        for step in Step::ALL {
            assert!(!step.title().is_empty());
            assert!(!step.keys().is_empty(), "{step:?}");
            let sentences = step.why().matches(". ").count() + 1;
            assert!((1..=3).contains(&sentences), "{step:?}: {sentences} sentences");
            let lines = live_lines(step, &d, &m, [None, None], &Finding::Nothing);
            assert!(step == Step::Done || !lines.is_empty(), "{step:?}");
        }
        // the exact keys of the task
        let keys = |s: Step| s.keys().join(" ");
        assert!(keys(Step::Pick).contains("Tab  browse") && keys(Step::Pick).contains("j k") && keys(Step::Pick).contains("a loads"));
        assert!(keys(Step::PlayA).contains("Space"));
        assert!(keys(Step::HearB).contains("F2") && keys(Step::HearB).contains(":xfade a"));
        assert!(keys(Step::SyncB).contains("l s"));
        assert!(keys(Step::Phrase).contains("Space"));
        assert!(keys(Step::BringIn).contains("f q") && keys(Step::BringIn).contains('[') && keys(Step::BringIn).contains(":low b kill"));
        assert!(keys(Step::BassSwap).contains(":low a kill") && keys(Step::BassSwap).contains(":low b flat") && keys(Step::BassSwap).contains("\\"));
        assert!(keys(Step::FadeOut).contains(":xfade b") && keys(Step::FadeOut).contains("h Space"));
        // no step teaches a key that is gone
        for step in Step::ALL {
            let text = step.keys().join(" ");
            for gone in ["< loads", "hold t", "hold y", "x  play", "c syncs"] {
                assert!(!text.contains(gone), "{step:?}: {gone}");
            }
        }
        assert!(keys(Step::Done).contains("F9") && keys(Step::Done).contains(":mix new blend") && keys(Step::Done).contains(":help"));
    }

    #[test]
    fn live_lines_show_the_bar_countdown_and_the_warnings() {
        let m = mixer(-1.0);
        let g = Grid { first_beat_s: 0.0, downbeat_offset: 0, beats_per_bar: 4 };
        // 120 BPM, 28 bars in: bar 29, a phrase line (bar 33) in 4 bars
        let a = deck(|d| {
            d.playing = true;
            d.track_bpm = 120.0;
            d.bpm = 120.0;
            d.beat_in_bar = 1;
            d.position = 28.0 * 2.0 + 0.1;
        });
        let b = deck(|d| d.eq = (0.0, 0.0, 0.0));
        let lines = live_lines(Step::Phrase, &[a.clone(), b.clone()], &m, [Some(g), None], &Finding::Nothing);
        assert!(lines.iter().any(|l| l.contains("bar 29, beat 1") && l.contains("bar 33") && l.contains("in 4 bars")), "{lines:?}");
        let late = DeckInfo { position: 31.0 * 2.0 + 1.6, ..a.clone() };
        let lines = live_lines(Step::Phrase, &[late, b.clone()], &m, [Some(g), None], &Finding::Nothing);
        assert!(lines.iter().any(|l| l.contains("press Space now") && l.contains("bar 33")), "{lines:?}");
        // B's fader down and the crossfader on A are called out in step 6
        let down = deck(|d| d.fader = 0.0);
        let lines = live_lines(Step::BringIn, &[a, down], &m, [None, None], &Finding::Nothing);
        assert!(lines.iter().any(|l| l.contains(":fader b full")) && lines.iter().any(|l| l.contains(":xfade 0")), "{lines:?}");
        // sync too far apart
        let (fast, slow) = (deck(|d| { d.track_bpm = 128.0; d.beat_in_bar = 1; d.bpm = 128.0 }), deck(|d| { d.track_bpm = 100.0; d.beat_in_bar = 1; d.bpm = 100.0 }));
        let lines = live_lines(Step::SyncB, &[fast, slow], &m, [None, None], &Finding::Nothing);
        assert!(lines.iter().any(|l| l.contains("over 16 %")), "{lines:?}");
        // the proposal is named with tempos
        let p = Proposal { a: Pick { path: "/a".into(), title: "Aftertekno".into(), bpm: 128.0 }, b: Pick { path: "/b".into(), title: "Smokin".into(), bpm: 130.0 } };
        let lines = live_lines(Step::Pick, &[DeckInfo::default(), DeckInfo::default()], &m, [None, None], &Finding::Found(p));
        assert!(lines[0].contains("Aftertekno") && lines[1].contains("Smokin") && lines[1].contains("130.0"), "{lines:?}");
        // step 3 on a single output says what to expect
        let lines = live_lines(Step::HearB, &[deck(|_| {}), deck(|_| {})], &m, [None, None], &Finding::Nothing);
        assert!(lines.iter().any(|l| l.contains("internal mode")), "{lines:?}");
    }
}
