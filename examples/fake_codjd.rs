//! A stand-in for codjd that serves just enough of `net.rotko.Codj.Player` for the DJ screen:
//! the deck API of `docs/dj-api.md` with plausible, changing state, `GetWaveform` with a
//! synthetic track, and a small library. Every state-changing call is appended to a log file.
//!
//! It must run on an isolated bus, never the real session bus:
//!
//!   dbus-run-session -- env CODJ_FAKE_ISOLATED=1 CODJ_FAKE_LOG=/tmp/calls.log \
//!       cargo run --example fake_codjd
//!
//! (the same bus then carries `codj`, so run it from the same `dbus-run-session`).

#![allow(dead_code)]

#[path = "../src/dj_types.rs"]
mod dj_types;
#[path = "../src/dj/patchdef.rs"]
mod patchdef;
#[path = "../src/types.rs"]
mod types;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dj_types::{DeckInfo, MixerInfo, PatchSlot, SetState};
use serde_json::{json, Value};
use types::{OuterMusic, OuterPlaylist, PlayerStatus, PlayingStatus, Repeat, Sort};

struct Deck {
    info: DeckInfo,
    nudge: f64,
    /// hold-to-preview from the cue button is active
    preview: bool,
    /// the sync master's tempo is followed
    follow: bool,
}

/// The fake set runtime: sets "demo" (3 tracks, 2 mixes) and "broken" (the same with 2 errors)
#[derive(Default)]
struct SetRt {
    active: String,
    armed: i32,
    running: bool,
    out_deck: i32,
    auto: bool,
}

/// (id, title, path)
const SET_TRACKS: [(&str, &str, &str); 3] = [
    ("warm", "Aftertekno", "/fake/Aftertekno.flac"),
    ("mid", "Smokin", "/fake/Smokin.flac"),
    ("peak", "Kaiku", "/fake/Kaiku.flac"),
];
/// (from, to, start beat, length in beats): 16 and 8 bars, starting early so a test sees them run
const SET_MIXES: [(&str, &str, f64, f64); 2] = [("warm", "mid", 8.0, 64.0), ("mid", "peak", 8.0, 32.0)];
/// per mix: (mix beat, text) in order
const SET_ITEMS: [&[(f64, &str)]; 2] = [
    &[(0.0, "B low kill"), (0.0, "bars 0–16 · B fader 0 → 1"), (32.0, "swap low"), (32.0, "bars 8–16 · A filter → −0.6"), (56.0, "A loop tail"), (64.0, "A stop")],
    &[(0.0, "B low kill"), (16.0, "swap low"), (24.0, "A loop 2 beats"), (32.0, "A stop")],
];

/// The text of the set "demo" as the fake serves it through SetSource / SetWrite
const DEMO_TEXT: &str = "# demo\nset \"demo: 3 tracks\"\n\ntrack warm \"Aftertekno\"\n  cue drop bar 33\n\ntrack mid \"Smokin\"\n\nmix warm -> mid at bar 3 over 16 bars\n  bar 0 mid play\n";

struct State {
    decks: [Deck; 2],
    mixer: MixerInfo,
    at: Instant,
    set: SetRt,
    /// the demo set's file text
    text: String,
    /// recording a mix since this instant
    recording: Option<Instant>,
}

impl State {
    fn spb(d: &DeckInfo) -> f64 {
        60.0 / d.track_bpm
    }

    /// moves time forward and recomputes the derived fields
    fn advance(&mut self) {
        let dt = self.at.elapsed().as_secs_f64();
        self.at = Instant::now();
        let master_bpm = [self.decks[1].info.bpm, self.decks[0].info.bpm];
        let master_playing = [self.decks[1].info.playing, self.decks[0].info.playing];
        for (i, deck) in self.decks.iter_mut().enumerate() {
            let d = &mut deck.info;
            if !d.loaded {
                continue;
            }
            if deck.follow && master_playing[i] && d.track_bpm > 0.0 {
                d.rate = master_bpm[i] / d.track_bpm / (1.0 + deck.nudge * 0.04);
            }
            let speed = d.rate * (1.0 + deck.nudge * 0.04);
            if d.playing {
                d.position += dt * speed;
                if d.loop_beats > 0.0 && d.position >= d.loop_end {
                    d.position = d.loop_start + (d.position - d.loop_end);
                }
                if d.position >= d.duration {
                    d.position = d.duration;
                    d.playing = false;
                }
            }
            d.decoded = d.duration.min(d.decoded + dt * 600.0);
            d.bpm = d.track_bpm * speed;
            let beats = d.position / Self::spb(d);
            d.beat_phase = beats.fract();
            d.beat_in_bar = (beats.floor() as u32 % 4) + 1;
        }
        self.advance_set();
    }

    fn track_deck(&self, id: &str) -> Option<usize> {
        let path = SET_TRACKS.iter().find(|t| t.0 == id)?.2;
        self.decks.iter().position(|d| d.info.loaded && d.info.path == path)
    }

    /// beats since the armed mix's start on the outgoing deck's grid
    fn mix_beat(&self) -> f64 {
        let (Some(m), Some(deck)) = (usize::try_from(self.set.armed).ok(), usize::try_from(self.set.out_deck).ok()) else {
            return 0.0;
        };
        let d = &self.decks[deck].info;
        d.position / Self::spb(d) - SET_MIXES[m].2
    }

    /// starts the armed mix when its time comes, finishes it, arms the next one in auto mode
    fn advance_set(&mut self) {
        let Some(m) = usize::try_from(self.set.armed).ok() else { return };
        let beat = self.mix_beat();
        if !self.set.running && beat >= 0.0 {
            self.set.running = true;
            let incoming = 1 - self.set.out_deck as usize;
            self.decks[incoming].info.playing = true;
            self.decks[incoming].info.fader = 1.0;
        }
        if self.set.running && beat >= SET_MIXES[m].3 {
            let out = self.set.out_deck as usize;
            self.decks[out].info.playing = false;
            self.set.armed = -1;
            self.set.running = false;
            self.set.out_deck = -1;
            if self.set.auto {
                if let Some(next) = SET_MIXES.get(m + 1).and_then(|n| self.track_deck(n.0).map(|_| m + 1)) {
                    let _ = self.arm(next);
                }
            }
        }
    }

    /// loads the track onto the deck, as `DjLoad` does
    fn load_track(&mut self, deck: usize, path: &str) {
        let bpm = fake_bpm(path);
        let keep = self.decks[deck].info.clone();
        let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.decks[deck] = Deck {
            info: DeckInfo {
                loaded: true,
                path: path.into(),
                title: stem,
                artist: "Fake Artist".into(),
                duration: 240.0,
                rate: 1.0,
                bpm,
                track_bpm: bpm,
                beat_phase: 0.0,
                beat_in_bar: 1,
                hotcues: vec![-1.0; 8],
                key: ["8A", "9A", "5B", "12A"][(bpm * 10.0) as usize % 4].into(),
                gain_db: -1.5,
                eq: keep.eq,
                filter: keep.filter,
                fader: keep.fader.max(0.0),
                headphones: keep.headphones,
                ..Default::default()
            },
            nudge: 0.0,
            preview: false,
            follow: false,
        };
        // loading onto either deck of a running mix disarms it
        if self.set.running {
            self.set.armed = -1;
            self.set.running = false;
            self.set.out_deck = -1;
        }
    }

    fn arm(&mut self, mix: usize) -> Result<(), String> {
        if self.set.active.is_empty() {
            return Err("no active set".into());
        }
        let Some(&(from, to, ..)) = SET_MIXES.get(mix) else {
            return Err(format!("no mix {mix}"));
        };
        let Some(out) = self.track_deck(from) else {
            return Err(format!("load {from} first"));
        };
        let to_path = SET_TRACKS.iter().find(|t| t.0 == to).unwrap().2;
        let incoming = 1 - out;
        if self.decks[incoming].info.path != to_path {
            self.load_track(incoming, to_path);
        }
        self.decks[incoming].info.playing = false;
        self.decks[incoming].info.synced = true;
        self.decks[incoming].info.fader = 0.0;
        self.set.armed = mix as i32;
        self.set.running = false;
        self.set.out_deck = out as i32;
        Ok(())
    }

    fn set_state(&mut self) -> SetState {
        let active = !self.set.active.is_empty();
        let m = usize::try_from(self.set.armed).ok();
        let beat = self.mix_beat();
        SetState {
            name: self.set.active.clone(),
            file: if active { format!("/fake/sets/{}.codj", self.set.active) } else { String::new() },
            errors: if self.set.active == "broken" { 2 } else { 0 },
            tracks: if active { SET_TRACKS.iter().map(|t| (t.0.into(), t.1.into(), t.2.into())).collect() } else { vec![] },
            mixes: if active { SET_MIXES.iter().map(|m| (m.0.into(), m.1.into(), m.2, m.3)).collect() } else { vec![] },
            armed: self.set.armed,
            running: self.set.running,
            out_deck: self.set.out_deck,
            mix_beat: if m.is_some() { beat } else { 0.0 },
            auto: self.set.auto,
            next_items: m
                .map(|m| SET_ITEMS[m].iter().filter(|(b, _)| *b >= beat.max(0.0) && (*b > beat || !self.set.running)).take(8).map(|(b, t)| (*b, t.to_string())).collect())
                .unwrap_or_default(),
        }
    }
}

struct Fake {
    st: Arc<Mutex<State>>,
    log: Option<String>,
    /// set name -> text, for SetSource / SetCheck / SetWrite
    sources: Arc<Mutex<std::collections::BTreeMap<String, String>>>,
    patches: Arc<Mutex<Patches>>,
}

/// One fake patch slot: the loaded patch is a JSON working copy with an undo stack
#[derive(Default)]
struct PSlot {
    loaded: bool,
    name: String,
    doc: Value,
    dirty: bool,
    live: bool,
    undo: Vec<Value>,
    redo: Vec<Value>,
    /// sequencer name -> (queued pattern, cycle it was queued in)
    queued: std::collections::BTreeMap<String, (u32, u64)>,
    fader: f64,
    headphones: bool,
}

/// patch name -> file text, and the 4 slots; the step counter runs at 8 steps a second
struct Patches {
    files: std::collections::BTreeMap<String, String>,
    slots: [PSlot; 4],
    t0: Instant,
}

const STEPS_PER_S: f64 = 8.0;

fn is_seq(n: &Value) -> bool {
    matches!(n["type"].as_str(), Some("MonoSeq" | "GateSeq"))
}

/// the fake's PatchCheck: JSON, one AudioOut, known types
fn patch_check(text: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return vec!["patch: not valid JSON".into()];
    };
    let Some(nodes) = v["nodes"].as_object() else {
        return vec!["patch: no nodes object".into()];
    };
    let mut errors = Vec::new();
    for (id, n) in nodes {
        match n["type"].as_str() {
            Some(t) if patchdef::is_known_type(t) => {}
            Some(t) => errors.push(format!("{id}: unknown type {t}")),
            None => errors.push(format!("{id}: no type")),
        }
    }
    if !nodes.values().any(|n| n["type"] == "AudioOut") {
        errors.push("patch: no AudioOut node".into());
    }
    errors
}

impl Patches {
    fn cycle_len_s(steps: f64) -> f64 {
        steps / STEPS_PER_S
    }

    /// applies queued pattern switches whose bar has come
    fn settle(&mut self) {
        let t = self.t0.elapsed().as_secs_f64();
        for slot in self.slots.iter_mut().filter(|s| s.loaded) {
            let due: Vec<(String, u32)> = slot
                .queued
                .iter()
                .filter(|(name, (_, cycle))| {
                    let steps = steps_of(&slot.doc, name);
                    (t / Self::cycle_len_s(steps)) as u64 > *cycle
                })
                .map(|(n, (p, _))| (n.clone(), *p))
                .collect();
            for (name, p) in due {
                slot.queued.remove(&name);
                if let Some(n) = seq_mut(&mut slot.doc, &name) {
                    n["params"]["pattern"] = json!(p);
                }
            }
        }
    }

    fn snapshot(&mut self) -> Vec<PatchSlot> {
        self.settle();
        let t = self.t0.elapsed().as_secs_f64();
        self.slots
            .iter()
            .map(|s| {
                if !s.loaded {
                    return PatchSlot::default();
                }
                let nodes = s.doc["nodes"].as_object().cloned().unwrap_or_default();
                let knobs = nodes
                    .values()
                    .filter(|n| n["type"] == "Knob")
                    .map(|n| {
                        let p = &n["params"];
                        (n["name"].as_str().unwrap_or("").to_string(), p["value"].as_f64().unwrap_or(0.0), p["min"].as_f64().unwrap_or(0.0), p["max"].as_f64().unwrap_or(1.0))
                    })
                    .collect();
                let sequencers = nodes
                    .values()
                    .filter(|n| is_seq(n))
                    .map(|n| {
                        let name = n["name"].as_str().unwrap_or("").to_string();
                        let steps = n["params"]["steps"].as_f64().unwrap_or(16.0);
                        let step = ((t * STEPS_PER_S) as u64 % steps as u64) as u32;
                        let queued = s.queued.get(&name).map_or(u32::MAX, |q| q.0);
                        (name, n["params"]["pattern"].as_u64().unwrap_or(0) as u32, queued, step)
                    })
                    .collect();
                PatchSlot {
                    loaded: true,
                    name: s.name.clone(),
                    title: s.doc["title"].as_str().unwrap_or("").to_string(),
                    dirty: s.dirty,
                    live: s.live,
                    cpu: 0.04 + 0.01 * ((t * 3.0).sin() + 1.0),
                    muted_reason: String::new(),
                    knobs,
                    sequencers,
                    fader: s.fader,
                    headphones: s.headphones,
                }
            })
            .collect()
    }
}

fn steps_of(doc: &Value, seq: &str) -> f64 {
    doc["nodes"].as_object().and_then(|n| n.values().find(|n| is_seq(n) && n["name"] == seq)).and_then(|n| n["params"]["steps"].as_f64()).unwrap_or(16.0)
}

fn seq_mut<'a>(doc: &'a mut Value, name: &str) -> Option<&'a mut Value> {
    doc["nodes"].as_object_mut()?.values_mut().find(|n| is_seq(n) && n["name"] == name)
}

/// Applies one `PatchAction` op to a slot's working copy; the message is the error
fn apply_action(slot: &mut PSlot, a: &Value, cycle_of: &dyn Fn(f64) -> u64) -> Result<bool, String> {
    let op = a["op"].as_str().ok_or("no op")?;
    let before = slot.doc.clone();
    let mut undoable = true;
    match op {
        "knob" => {
            let name = a["name"].as_str().ok_or("no name")?;
            let value = a["value"].as_f64().ok_or("no value")?;
            let n = slot.doc["nodes"].as_object_mut().unwrap().values_mut().find(|n| n["type"] == "Knob" && n["name"] == name).ok_or(format!("no knob {name}"))?;
            let (lo, hi) = (n["params"]["min"].as_f64().unwrap_or(0.0), n["params"]["max"].as_f64().unwrap_or(1.0));
            n["params"]["value"] = json!(value.clamp(lo, hi));
        }
        "cell" => {
            let seq = a["seq"].as_str().ok_or("no seq")?;
            let (p, step, row) = (a["pattern"].as_u64().ok_or("no pattern")? as usize, a["step"].as_u64().ok_or("no step")?, a["row"].as_u64().ok_or("no row")?);
            let on = a["on"].as_bool().ok_or("no on")?;
            let vel = a["velocity"].as_f64().unwrap_or(1.0);
            if p > 7 {
                return Err("pattern 0..7".into());
            }
            let n = seq_mut(&mut slot.doc, seq).ok_or(format!("no sequencer {seq}"))?;
            let gate = n["type"] == "GateSeq";
            let pats = n["params"]["patterns"].as_array_mut().ok_or("no patterns")?;
            while pats.len() <= p {
                pats.push(json!({"steps": 16, "cells": []}));
            }
            let cells = pats[p]["cells"].as_array_mut().ok_or("no cells")?;
            cells.retain(|c| !(c[0].as_u64() == Some(step) && c[1].as_u64() == Some(row)));
            if on {
                cells.push(if gate { json!([step, row, vel]) } else { json!([step, row]) });
            }
        }
        "pattern" => {
            undoable = false;
            let seq = a["seq"].as_str().ok_or("no seq")?.to_string();
            let p = a["pattern"].as_u64().ok_or("no pattern")? as u32;
            if p > 7 {
                return Err("pattern 0..7".into());
            }
            let steps = steps_of(&slot.doc, &seq);
            let n = seq_mut(&mut slot.doc, &seq).ok_or(format!("no sequencer {seq}"))?;
            if a["now"].as_bool().unwrap_or(false) {
                n["params"]["pattern"] = json!(p);
                slot.queued.remove(&seq);
            } else {
                slot.queued.insert(seq, (p, cycle_of(steps)));
            }
        }
        "set_param" => {
            let id = a["id"].as_str().ok_or("no id")?;
            let n = slot.doc["nodes"].get_mut(id).ok_or(format!("{id}: no such node"))?;
            n["params"][a["param"].as_str().ok_or("no param")?] = a["value"].clone();
        }
        "set_input" | "connect" | "disconnect" => {
            let to = a["to"].as_array().ok_or("no to")?;
            let id = to[0].as_str().unwrap_or("");
            let input = to[1].as_str().unwrap_or("").to_string();
            let n = slot.doc["nodes"].get_mut(id).ok_or(format!("{id}: no such node"))?;
            match op {
                "disconnect" => {
                    if let Some(ins) = n["ins"].as_object_mut() {
                        ins.remove(&input);
                    }
                }
                "set_input" => n["ins"][&input] = a["value"].clone(),
                _ => n["ins"][&input] = a["from"].clone(),
            }
        }
        "add_node" => {
            let id = a["id"].as_str().ok_or("no id")?.to_string();
            let ty = a["type"].as_str().ok_or("no type")?;
            if !patchdef::is_known_type(ty) {
                return Err(format!("{id}: unknown type {ty}"));
            }
            slot.doc["nodes"][&id] = json!({"type": ty, "name": a["name"], "params": a["params"]});
        }
        "remove_node" => {
            let id = a["id"].as_str().ok_or("no id")?;
            slot.doc["nodes"].as_object_mut().unwrap().remove(id).ok_or(format!("{id}: no such node"))?;
        }
        "rename" => {
            let id = a["id"].as_str().ok_or("no id")?;
            slot.doc["nodes"].get_mut(id).ok_or(format!("{id}: no such node"))?["name"] = a["name"].clone();
        }
        other => return Err(format!("unknown op {other}")),
    }
    if undoable {
        slot.undo.push(before);
        slot.redo.clear();
        slot.dirty = true;
    }
    Ok(undoable)
}

const DEMO_SOURCE: &str = "# demo set\ntrack warm \"Aftertekno\"\ntrack mid \"Smokin\"\ntrack peak \"Kaiku\"\nmix warm -> mid at bar 2 for 16 bars\n  bars 0-16 B fader 0 -> 1\n  bar 8 swap low\nmix mid -> peak at bar 2 for 8 bars\n  bar 0 B low kill\n";
const BROKEN_SOURCE: &str = "# broken set\ntrack warm \"Aftertekno\"\nmix warm -> mid2 ERROR\n  bar 40 ERROR\n";

fn sha(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// the fake's "check": an error for every line containing `ERROR` (editor e2e) or `BAD`
/// (MCP e2e)
fn fake_check(text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            if let Some(c) = l.find("ERROR") {
                Some(format!("fake.codj:{}:{}: ERROR marker found: {}", i + 1, c + 1, l.trim()))
            } else {
                l.find("BAD").map(|c| format!("demo.codj:{}:{}: syntax error near BAD", i + 1, c + 1))
            }
        })
        .collect()
}

impl Fake {
    /// the active set, or the error to reply with
    fn need_set(&self) -> Result<std::sync::MutexGuard<'_, State>, String> {
        let st = self.st.lock().unwrap();
        if st.set.active.is_empty() {
            Err("no active set".into())
        } else {
            Ok(st)
        }
    }

    fn use_set(&self, st: &mut State, name: &str) -> String {
        st.set = SetRt { active: name.to_string(), armed: -1, out_deck: -1, ..Default::default() };
        "ok".into()
    }

    fn log(&self, line: String) {
        if let Some(path) = &self.log {
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    fn with<R>(&self, deck: u32, f: impl FnOnce(&mut State, usize) -> R) -> Result<R, String> {
        if deck > 1 {
            return Err(format!("no deck {deck}"));
        }
        let mut st = self.st.lock().unwrap();
        st.advance();
        Ok(f(&mut st, deck as usize))
    }

    fn loaded(&self, deck: u32, f: impl FnOnce(&mut State, usize) -> String) -> String {
        match self.with(deck, |s, i| {
            if s.decks[i].info.loaded {
                f(s, i)
            } else {
                "nothing loaded".to_string()
            }
        }) {
            Ok(r) => r,
            Err(e) => e,
        }
    }
}

/// (name, explanation, default bars)
const RECIPES: [(&str, &str, u32); 3] = [
    ("blend", "bring B in with its bass killed, swap the bass halfway, fade A out", 16),
    ("swap", "a short handover: one bass swap, no long fade", 8),
    ("filter-fade", "sweep A's filter away while B fades in", 16),
];

/// a plausible commented block for a recipe
fn recipe_text(recipe: &str, from: &str, to: &str, bars: u32) -> String {
    let Some(r) = RECIPES.iter().find(|r| r.0 == recipe) else {
        return format!("unknown recipe {recipe:?}");
    };
    let half = bars / 2;
    let body = match recipe {
        "swap" => format!("  bar 0        {to}.low kill\n  bar {half}       swap low\n  bar {bars}       {from} stop\n"),
        "filter-fade" => format!("  bars 0..{bars}   {from}.filter 0 -> -0.6 smooth; {to}.fader 0 -> 1\n  bar {bars}       {from} stop\n"),
        _ => format!(
            "  bars 0..{half}   {to}.low kill; {to}.fader 0 -> 1\n  bar {half}       swap low\n  bars {half}..{bars}  {from}.filter 0 -> -0.6 smooth; {from}.fader 1 -> 0\n  bar {bars}       {from} stop\n"
        ),
    };
    format!("# {recipe}: {}\nmix {from} -> {to} at bar 3 over {bars} bars\n  {to} play from bar 1\n{body}", r.1)
}

fn library() -> Vec<OuterMusic> {
    [
        ("Aftertekno", "Pneumatix", 128.0),
        ("Smokin", "Aasi & Swooh", 130.0),
        ("Hot minute", "Nlogax", 126.0),
        ("Kaiku", "Rotko", 124.0),
        ("Nightdrive", "Fake Artist", 132.0),
        ("Pallas", "Fake Artist", 127.5),
    ]
    .into_iter()
    .map(|(t, a, _)| OuterMusic {
        title: t.into(),
        length: Duration::from_secs(240),
        path: std::path::PathBuf::from(format!("/fake/{t}.flac")).into(),
        artist: a.into(),
        genre: "techno".into(),
    })
    .collect()
}

fn fake_bpm(path: &str) -> f64 {
    let h: u32 = path.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    124.0 + (h % 80) as f64 / 10.0
}

#[zbus::interface(name = "net.rotko.Codj.Player")]
impl Fake {
    async fn get_playing_index(&self) -> u32 {
        0
    }

    async fn playlist(&self) -> OuterPlaylist {
        OuterPlaylist {
            musics: library(),
            sort: Sort::default(),
            repeat: Repeat::default(),
            playing_index: 0,
            name: "default".into(),
        }
    }

    async fn played_duration(&self) -> f32 {
        0.0
    }

    async fn timer(&self) -> (f32, f32) {
        (0.0, 240.0)
    }

    async fn playing(&self) -> OuterMusic {
        library().remove(0)
    }

    async fn get_volume(&self) -> f32 {
        0.5
    }

    async fn get_repeat(&self) -> Repeat {
        Repeat::default()
    }

    async fn get_sort(&self) -> Sort {
        Sort::default()
    }

    async fn get_playlists_names(&self) -> Vec<String> {
        vec!["default".into()]
    }

    async fn analysis_status(&self) -> (u32, u32) {
        (6, 6)
    }

    async fn player_status(&self) -> PlayerStatus {
        PlayerStatus { status: PlayingStatus::Pausing, music: library().remove(0), volume: 0.5, index: 0 }
    }

    async fn dj_load(&self, deck: u32, path: String) -> String {
        self.log(format!("DjLoad {deck} {path}"));
        if path.contains("bad") {
            return "cannot open file".into();
        }
        match self.with(deck, |s, i| {
            s.load_track(i, &path);
            "ok".to_string()
        }) {
            Ok(r) => r,
            Err(e) => e,
        }
    }

    async fn dj_play(&self, deck: u32) -> String {
        self.log(format!("DjPlay {deck}"));
        self.loaded(deck, |s, i| {
            s.decks[i].info.playing = true;
            "ok".into()
        })
    }

    async fn dj_pause(&self, deck: u32) -> String {
        self.log(format!("DjPause {deck}"));
        self.loaded(deck, |s, i| {
            s.decks[i].info.playing = false;
            "ok".into()
        })
    }

    async fn dj_seek(&self, deck: u32, seconds: f64) -> String {
        self.log(format!("DjSeek {deck} {seconds}"));
        self.loaded(deck, |s, i| {
            s.decks[i].info.position = seconds.clamp(0.0, s.decks[i].info.duration);
            "ok".into()
        })
    }

    async fn dj_set(&self, control: String, deck: u32, value: f64) -> String {
        self.log(format!("DjSet {control} {deck} {value:.4}"));
        match self.with(deck, |s, i| {
            let d = &mut s.decks[i];
            match control.as_str() {
                "rate" => {
                    d.info.rate = value;
                    d.follow = false;
                    d.info.synced = false;
                }
                "gain" => d.info.gain_db = value,
                "fader" => d.info.fader = value,
                "cue" => d.info.headphones = value > 0.5,
                "low" => d.info.eq.0 = value,
                "mid" => d.info.eq.1 = value,
                "high" => d.info.eq.2 = value,
                "filter" => d.info.filter = value,
                "crossfader" => s.mixer.crossfader = value,
                "master" => s.mixer.master = value,
                "cue_mix" => s.mixer.cue_mix = value,
                // not part of MixerInfo: only logged, so the client's edits can be checked
                "sync_range" => {}
                "quantize" => s.mixer.quantize = value > 0.5,
                other => return format!("unknown control {other}"),
            }
            "ok".to_string()
        }) {
            Ok(r) => r,
            Err(e) => e,
        }
    }

    async fn dj_cue(&self, deck: u32, pressed: bool) -> String {
        self.log(format!("DjCue {deck} {pressed}"));
        self.loaded(deck, |s, i| {
            let d = &mut s.decks[i];
            if pressed {
                if d.info.playing {
                    d.info.position = d.info.cue_point;
                    d.info.playing = false;
                } else {
                    d.info.cue_point = d.info.position;
                    d.info.playing = true;
                    d.preview = true;
                }
            } else if d.preview {
                d.preview = false;
                d.info.position = d.info.cue_point;
                d.info.playing = false;
            }
            "ok".into()
        })
    }

    async fn dj_auto_mix(&self, from: u32, to: u32) -> String {
        self.log(format!("DjAutoMix {from} {to}"));
        "ok blend · 32 bars · bar 97\nset \"automix\"\n".into()
    }

    async fn automix(&self, on: bool) -> String {
        self.log(format!("Automix {on}"));
        "ok".into()
    }

    async fn automix_state(&self) -> (bool, String, f64, String) {
        (true, "Next Song".into(), 72., "blend · 32 bars".into())
    }

    async fn dj_sync(&self, deck: u32) -> String {
        self.log(format!("DjSync {deck}"));
        self.loaded(deck, |s, i| {
            let master = s.decks[1 - i].info.clone();
            if !master.loaded || master.track_bpm <= 0.0 {
                return "no beat grid".into();
            }
            let d = &mut s.decks[i];
            d.info.rate = master.bpm / d.info.track_bpm;
            d.follow = true;
            d.info.synced = true;
            // one-time phase alignment to the master
            let spb = State::spb(&d.info);
            let beats = d.info.position / spb;
            d.info.position = (beats.floor() + master.beat_phase) * spb;
            "ok".into()
        })
    }

    async fn dj_nudge(&self, deck: u32, amount: f64) -> String {
        self.log(format!("DjNudge {deck} {amount}"));
        self.loaded(deck, |s, i| {
            s.decks[i].nudge = amount.clamp(-1.0, 1.0);
            "ok".into()
        })
    }

    async fn dj_beat_jump(&self, deck: u32, beats: f64) -> String {
        self.log(format!("DjBeatJump {deck} {beats}"));
        self.loaded(deck, |s, i| {
            let d = &mut s.decks[i].info;
            d.position = (d.position + beats * State::spb(d)).clamp(0.0, d.duration);
            "ok".into()
        })
    }

    async fn dj_loop(&self, deck: u32, beats: f64) -> String {
        self.log(format!("DjLoop {deck} {beats}"));
        self.loaded(deck, |s, i| {
            let d = &mut s.decks[i].info;
            if beats <= 0.0 {
                d.loop_beats = 0.0;
            } else {
                let spb = State::spb(d);
                if d.loop_beats <= 0.0 {
                    d.loop_start = (d.position / spb).floor() * spb;
                }
                d.loop_beats = beats;
                d.loop_end = d.loop_start + beats * spb;
            }
            "ok".into()
        })
    }

    async fn dj_hotcue(&self, deck: u32, slot: u32, action: String) -> String {
        self.log(format!("DjHotcue {deck} {slot} {action}"));
        self.loaded(deck, |s, i| {
            let d = &mut s.decks[i].info;
            let Some(cue) = d.hotcues.get_mut(slot as usize) else {
                return format!("no slot {slot}");
            };
            match action.as_str() {
                "set" => *cue = d.position,
                "jump" if *cue >= 0.0 => d.position = *cue,
                "jump" => return "empty slot".into(),
                "clear" => *cue = -1.0,
                other => return format!("unknown action {other}"),
            }
            "ok".into()
        })
    }

    async fn set_use(&self, name: String) -> String {
        self.log(format!("SetUse {name}"));
        let mut st = self.st.lock().unwrap();
        match name.as_str() {
            "" | "demo" | "broken" => {}
            "demo.codj" => return self.use_set(&mut st, "demo"),
            other => return format!("no set named {other}"),
        }
        self.use_set(&mut st, &name)
    }

    async fn set_arm(&self, mix: u32) -> String {
        self.log(format!("SetArm {mix}"));
        let mut st = self.st.lock().unwrap();
        st.advance();
        st.arm(mix as usize).err().unwrap_or_else(|| "ok".into())
    }

    async fn set_disarm(&self) -> String {
        self.log("SetDisarm".into());
        let mut st = self.st.lock().unwrap();
        st.set.armed = -1;
        st.set.running = false;
        st.set.out_deck = -1;
        "ok".into()
    }

    async fn set_auto(&self, on: bool) -> String {
        self.log(format!("SetAuto {on}"));
        self.st.lock().unwrap().set.auto = on;
        "ok".into()
    }

    async fn set_preview(&self, mix: u32, bars_before: f64) -> String {
        self.log(format!("SetPreview {mix} {bars_before}"));
        let mut st = self.st.lock().unwrap();
        st.advance();
        let Some(&(from, ..)) = SET_MIXES.get(mix as usize) else { return format!("no mix {mix}") };
        if st.set.active.is_empty() {
            return "no active set".into();
        }
        let path = SET_TRACKS.iter().find(|t| t.0 == from).unwrap().2;
        let out = st.track_deck(from).unwrap_or_else(|| {
            let free = if st.decks[0].info.playing { 1 } else { 0 };
            st.load_track(free, path);
            free
        });
        if let Err(e) = st.arm(mix as usize) {
            return e;
        }
        let d = &mut st.decks[out].info;
        let spb = State::spb(d);
        d.position = ((SET_MIXES[mix as usize].2 - bars_before * 4.0) * spb).max(0.0);
        d.playing = true;
        "ok".into()
    }

    async fn set_load(&self, track: u32) -> String {
        self.log(format!("SetLoad {track}"));
        let mut st = self.st.lock().unwrap();
        st.advance();
        if st.set.active.is_empty() {
            return "no active set".into();
        }
        let Some(&(_, _, path)) = SET_TRACKS.get(track as usize) else { return format!("no track {track}") };
        let free = if st.decks[0].info.playing && !st.decks[1].info.playing { 1 } else { 0 };
        st.load_track(free, path);
        "ok".into()
    }

    async fn set_cue(&self, deck: u32, name: String) -> String {
        self.log(format!("SetCue {deck} {name}"));
        let mut st = match self.need_set() { Ok(s) => s, Err(e) => return e };
        st.advance();
        let Some(d) = st.decks.get(deck as usize).map(|d| d.info.clone()).filter(|d| d.loaded) else { return "nothing loaded".into() };
        let bar = (d.position * 2.0 / 4.0) as u32 + 1;
        st.text.push_str(&format!("  cue {name} bar {bar}\n"));
        format!("ok cue {name} -> {} bar {bar}", d.title)
    }

    async fn set_loop_capture(&self, deck: u32, name: String) -> String {
        self.log(format!("SetLoopCapture {deck} {name}"));
        let mut st = match self.need_set() { Ok(s) => s, Err(e) => return e };
        st.advance();
        let Some(d) = st.decks.get(deck as usize).map(|d| d.info.clone()).filter(|d| d.loaded) else { return "nothing loaded".into() };
        if d.loop_beats <= 0.0 {
            return "no active loop on that deck".into();
        }
        st.text.push_str(&format!("  loop {name} bar 1 len {} beats\n", d.loop_beats));
        format!("ok loop {name} -> {} len {} beats", d.title, d.loop_beats)
    }

    async fn set_grid_capture(&self, deck: u32) -> String {
        self.log(format!("SetGridCapture {deck}"));
        let mut st = match self.need_set() { Ok(s) => s, Err(e) => return e };
        st.advance();
        let Some(d) = st.decks.get(deck as usize).map(|d| d.info.clone()).filter(|d| d.loaded) else { return "nothing loaded".into() };
        st.text.push_str(&format!("  grid {:.1}\n", d.bpm));
        format!("ok grid {:.1} -> {}", d.bpm, d.title)
    }

    async fn set_record(&self, on: bool) -> String {
        self.log(format!("SetRecord {on}"));
        let mut st = match self.need_set() { Ok(s) => s, Err(e) => return e };
        match (on, st.recording) {
            (true, Some(_)) => "already recording".into(),
            (true, None) => {
                st.recording = Some(Instant::now());
                "ok".into()
            }
            (false, None) => "not recording".into(),
            (false, Some(_)) => {
                st.recording = None;
                "ok mix 2: aft -> smk, 16 bars, 9 items".into()
            }
        }
    }

    /// 2 beats and 3 events per second since the start
    async fn set_recording(&self) -> (bool, f64, u32) {
        let st = self.st.lock().unwrap();
        match st.recording {
            Some(t) => (true, t.elapsed().as_secs_f64() * 2.0, (t.elapsed().as_secs_f64() * 3.0) as u32),
            None => (false, 0.0, 0),
        }
    }

    async fn set_state(&self) -> SetState {
        let mut st = self.st.lock().unwrap();
        st.advance();
        st.set_state()
    }

    async fn set_list(&self) -> Vec<(String, String, u32)> {
        self.log("SetList".into());
        self.sources
            .lock()
            .unwrap()
            .iter()
            .map(|(n, t)| (n.clone(), format!("/fake/sets/{n}.codj"), fake_check(t).len() as u32))
            .collect()
    }

    async fn set_source(&self, name: String) -> zbus::fdo::Result<(String, String)> {
        self.log(format!("SetSource {name}"));
        match self.sources.lock().unwrap().get(&name) {
            Some(t) => Ok((t.clone(), sha(t))),
            None => Err(zbus::fdo::Error::Failed(format!("no set named {name}"))),
        }
    }

    async fn set_check(&self, text: String) -> Vec<String> {
        let errors = fake_check(&text);
        self.log(format!("SetCheck lines={} errors={}", text.lines().count(), errors.len()));
        errors
    }

    async fn set_write(&self, name: String, text: String, base_hash: String, author: String) -> String {
        let mut sources = self.sources.lock().unwrap();
        let escaped = text.replace('\n', "\\n");
        let head = format!("SetWrite {name} base={base_hash} author={author}");
        match sources.get(&name) {
            Some(cur) if sha(cur) != base_hash => {
                self.log(format!("{head} REFUSED stale"));
                return "set changed since you read it".into();
            }
            None if !base_hash.is_empty() => {
                self.log(format!("{head} REFUSED no such set"));
                return format!("no set named {name}");
            }
            _ => {}
        }
        let errors = fake_check(&text);
        if let Some(first) = errors.first() {
            self.log(format!("{head} REFUSED errors={}", errors.len()));
            return format!("refused: {} error(s): {first}", errors.len());
        }
        let hash = sha(&text);
        self.log(format!("{head} ok hash={} text={escaped}", &hash[..8]));
        sources.insert(name, text);
        format!("ok {hash}")
    }

    async fn set_errors(&self, name: String) -> Vec<String> {
        self.log(format!("SetErrors {name}"));
        if name == "broken" {
            vec![
                "/fake/sets/broken.codj:12:5: unknown track \"mid2\" (did you mean \"mid\"?)".into(),
                "/fake/sets/broken.codj:20:1: bar 40 is past the end of the mix (32 bars)".into(),
            ]
        } else {
            vec![]
        }
    }

    /// (name, explanation, default bars)
    async fn set_recipes(&self) -> Vec<(String, String, u32)> {
        self.log("SetRecipes".into());
        RECIPES.iter().map(|r| (r.0.to_string(), r.1.to_string(), r.2)).collect()
    }

    async fn set_recipe_text(&self, recipe: String, from_id: String, to_id: String, bars: u32) -> String {
        self.log(format!("SetRecipeText {recipe} {from_id} {to_id} {bars}"));
        recipe_text(&recipe, &from_id, &to_id, bars)
    }

    async fn set_warnings(&self, name: String) -> Vec<String> {
        self.log(format!("SetWarnings {name}"));
        if name == "demo" {
            vec![format!("/fake/sets/demo.codj:8:1: mix 2 (mid -> peak) is only 8 bars: short for a bass swap")]
        } else {
            vec![]
        }
    }

    /// (bpm, first beat s, downbeat offset, beats per bar, confidence, variable, octave doubt, analyzed)
    async fn get_grid(&self, path: String) -> (f64, f64, u32, u32, f64, bool, bool, bool) {
        self.log(format!("GetGrid {path}"));
        (fake_bpm(&path), 0.0, 0, 4, 0.9, false, false, true)
    }

    /// playlist indices of the n tracks closest in tempo to `path`
    async fn similar_to(&self, path: String, n: u32) -> Vec<u32> {
        self.log(format!("SimilarTo {path} {n}"));
        let me = fake_bpm(&path);
        let mut all: Vec<(u32, f64)> = library()
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let p = <&std::path::Path>::from(&m.path).to_string_lossy().into_owned();
                (p != path).then(|| (i as u32, (fake_bpm(&p) - me).abs()))
            })
            .collect();
        all.sort_by(|a, b| a.1.total_cmp(&b.1));
        all.into_iter().take(n as usize).map(|(i, _)| i).collect()
    }

    async fn patch_list(&self) -> Vec<(String, String)> {
        self.log("PatchList".into());
        self.patches
            .lock()
            .unwrap()
            .files
            .iter()
            .map(|(n, t)| (n.clone(), serde_json::from_str::<Value>(t).ok().and_then(|v| v["title"].as_str().map(str::to_string)).unwrap_or_default()))
            .collect()
    }

    async fn patch_source(&self, name: String) -> zbus::fdo::Result<(String, String)> {
        self.log(format!("PatchSource {name}"));
        match self.patches.lock().unwrap().files.get(&name) {
            Some(t) => Ok((t.clone(), sha(t))),
            None => Err(zbus::fdo::Error::Failed(format!("no patch named {name}"))),
        }
    }

    async fn patch_check(&self, json: String) -> Vec<String> {
        let errors = patch_check(&json);
        self.log(format!("PatchCheck bytes={} errors={}", json.len(), errors.len()));
        errors
    }

    async fn patch_write(&self, name: String, json: String, base_hash: String, author: String) -> String {
        let mut p = self.patches.lock().unwrap();
        let head = format!("PatchWrite {name} base={base_hash} author={author}");
        match p.files.get(&name) {
            Some(cur) if sha(cur) != base_hash => {
                self.log(format!("{head} REFUSED stale"));
                return "set changed since you read it".into();
            }
            None if !base_hash.is_empty() => {
                self.log(format!("{head} REFUSED no such patch"));
                return format!("no patch named {name}");
            }
            _ => {}
        }
        let errors = patch_check(&json);
        if let Some(first) = errors.first() {
            self.log(format!("{head} REFUSED errors={}", errors.len()));
            return format!("refused: {} error(s): {first}", errors.len());
        }
        let hash = sha(&json);
        self.log(format!("{head} ok hash={} text={}", &hash[..8], json.replace('\n', "\\n")));
        p.files.insert(name, json);
        format!("ok {hash}")
    }

    async fn patch_import(&self, ncft_path: String, name: String, author: String) -> String {
        self.log(format!("PatchImport {ncft_path} {name} author={author}"));
        if !ncft_path.ends_with(".ncft") || !std::path::Path::new(&ncft_path).exists() {
            return format!("cannot read {ncft_path}");
        }
        let text = patchdef::template("acid", &name).unwrap();
        let hash = sha(&text);
        self.patches.lock().unwrap().files.insert(name, text);
        format!("ok {hash} (dropped MidiIn, Scope)")
    }

    async fn patch_load(&self, slot: u32, name: String) -> String {
        self.log(format!("PatchLoad {slot} {name}"));
        let mut p = self.patches.lock().unwrap();
        let Some(text) = p.files.get(&name).cloned() else { return format!("no patch named {name}") };
        let Some(s) = p.slots.get_mut(slot as usize) else { return format!("no slot {slot}") };
        let Ok(doc) = serde_json::from_str::<Value>(&text) else { return "invalid patch".into() };
        *s = PSlot { loaded: true, name, doc, fader: 1.0, headphones: true, ..Default::default() };
        "ok".into()
    }

    async fn patch_unload(&self, slot: u32) -> String {
        self.log(format!("PatchUnload {slot}"));
        match self.patches.lock().unwrap().slots.get_mut(slot as usize) {
            Some(s) => {
                *s = PSlot::default();
                "ok".into()
            }
            None => format!("no slot {slot}"),
        }
    }

    async fn patch_action(&self, slot: u32, action_json: String) -> String {
        self.log(format!("PatchAction {slot} {action_json}"));
        let Ok(a) = serde_json::from_str::<Value>(&action_json) else { return "action is not valid JSON".into() };
        let mut p = self.patches.lock().unwrap();
        let t0 = p.t0;
        let Some(s) = p.slots.get_mut(slot as usize).filter(|s| s.loaded) else { return format!("slot {slot} is empty") };
        let cycle_of = move |steps: f64| (t0.elapsed().as_secs_f64() / Patches::cycle_len_s(steps)) as u64;
        match apply_action(s, &a, &cycle_of) {
            Ok(_) => "ok".into(),
            Err(e) => e,
        }
    }

    async fn patch_undo(&self, slot: u32) -> String {
        self.log(format!("PatchUndo {slot}"));
        let mut p = self.patches.lock().unwrap();
        let Some(s) = p.slots.get_mut(slot as usize).filter(|s| s.loaded) else { return format!("slot {slot} is empty") };
        match s.undo.pop() {
            Some(prev) => {
                s.redo.push(std::mem::replace(&mut s.doc, prev));
                s.dirty = !s.undo.is_empty();
                "ok".into()
            }
            None => "nothing to undo".into(),
        }
    }

    async fn patch_redo(&self, slot: u32) -> String {
        self.log(format!("PatchRedo {slot}"));
        let mut p = self.patches.lock().unwrap();
        let Some(s) = p.slots.get_mut(slot as usize).filter(|s| s.loaded) else { return format!("slot {slot} is empty") };
        match s.redo.pop() {
            Some(next) => {
                s.undo.push(std::mem::replace(&mut s.doc, next));
                s.dirty = true;
                "ok".into()
            }
            None => "nothing to redo".into(),
        }
    }

    async fn patch_save(&self, slot: u32, name: String, author: String) -> String {
        self.log(format!("PatchSave {slot} {name} author={author}"));
        let mut p = self.patches.lock().unwrap();
        let Some(s) = p.slots.get_mut(slot as usize).filter(|s| s.loaded) else { return format!("slot {slot} is empty") };
        let text = serde_json::to_string_pretty(&s.doc).unwrap() + "\n";
        s.dirty = false;
        s.undo.clear();
        s.redo.clear();
        s.name = name.clone();
        let hash = sha(&text);
        p.files.insert(name, text);
        format!("ok {hash}")
    }

    async fn patch_live(&self, slot: u32, live: bool) -> String {
        self.log(format!("PatchLive {slot} {live}"));
        match self.patches.lock().unwrap().slots.get_mut(slot as usize).filter(|s| s.loaded) {
            Some(s) => {
                s.live = live;
                s.headphones = !live;
                "ok".into()
            }
            None => format!("slot {slot} is empty"),
        }
    }

    async fn patch_state(&self) -> Vec<PatchSlot> {
        self.patches.lock().unwrap().snapshot()
    }

    async fn dj_decks(&self) -> Vec<DeckInfo> {
        let mut st = self.st.lock().unwrap();
        st.advance();
        st.decks.iter().map(|d| d.info.clone()).collect()
    }

    async fn dj_mixer(&self) -> MixerInfo {
        self.st.lock().unwrap().mixer.clone()
    }

    async fn get_waveform(&self, path: String, columns: u32) -> Vec<u8> {
        self.log(format!("GetWaveform {path} {columns}"));
        if path.contains("nowave") {
            return Vec::new();
        }
        let n = (240 * 50).min(columns as usize);
        let bpm = fake_bpm(&path);
        let mut out = (n as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&240_000u32.to_le_bytes());
        for i in 0..n {
            let t = i as f64 / 50.0;
            let beat = (t * bpm / 60.0).fract();
            // kick on the beat, hats off the beat, a breakdown every 64 beats
            let breakdown = ((t * bpm / 60.0) / 32.0) as u32 % 4 == 3;
            let kick = if beat < 0.15 && !breakdown { 1.0 } else { 0.25 };
            let hat = if (beat - 0.5).abs() < 0.08 { 0.9 } else { 0.35 };
            let b = |x: f64| (x.clamp(0.0, 1.0) * 255.0) as u8;
            out.extend_from_slice(&[
                b(kick),
                b(0.5 + 0.2 * (t * 0.7).sin()),
                b(hat),
                b(kick * 0.7),
                b(0.35 + 0.1 * (t * 0.7).sin()),
                b(hat * 0.6),
            ]);
        }
        out
    }
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    if std::env::var("CODJ_FAKE_ISOLATED").as_deref() != Ok("1") {
        eprintln!("refusing to run: start it under `dbus-run-session -- env CODJ_FAKE_ISOLATED=1 ...`");
        std::process::exit(2);
    }
    let blank = || Deck { info: DeckInfo { hotcues: vec![-1.0; 8], rate: 1.0, fader: 1.0, ..Default::default() }, nudge: 0.0, preview: false, follow: false };
    let fake = Fake {
        st: Arc::new(Mutex::new(State {
            decks: [blank(), blank()],
            mixer: MixerInfo { mode: "internal".into(), master: 1.0, quantize: true, ..Default::default() },
            at: Instant::now(),
            set: SetRt { armed: -1, out_deck: -1, ..Default::default() },
            text: DEMO_TEXT.to_string(),
            recording: None,
        })),
        log: std::env::var("CODJ_FAKE_LOG").ok(),
        patches: Arc::new(Mutex::new(Patches {
            files: [("acid", "acid line"), ("pad1", "slow pad")]
                .into_iter()
                .map(|(n, t)| (n.to_string(), patchdef::template(if n == "acid" { "acid" } else { "pad" }, t).unwrap()))
                .collect(),
            slots: Default::default(),
            t0: Instant::now(),
        })),
        sources: Arc::new(Mutex::new(
            [("demo".to_string(), DEMO_SOURCE.to_string()), ("broken".to_string(), BROKEN_SOURCE.to_string())].into_iter().collect(),
        )),
    };
    let _conn = zbus::connection::Builder::session()?
        .name("net.rotko.Codj")?
        .serve_at("/net/rotko/Codj", fake)?
        .build()
        .await?;
    eprintln!("fake codjd up");
    std::future::pending::<()>().await;
    Ok(())
}
