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
#[path = "../src/types.rs"]
mod types;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dj_types::{DeckInfo, MixerInfo, SetState};
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

struct State {
    decks: [Deck; 2],
    mixer: MixerInfo,
    at: Instant,
    set: SetRt,
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
}

impl Fake {
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

    async fn set_state(&self) -> SetState {
        let mut st = self.st.lock().unwrap();
        st.advance();
        st.set_state()
    }

    async fn set_list(&self) -> Vec<(String, String, u32)> {
        self.log("SetList".into());
        vec![
            ("demo".into(), "/fake/sets/demo.codj".into(), 0),
            ("broken".into(), "/fake/sets/broken.codj".into(), 2),
        ]
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
        })),
        log: std::env::var("CODJ_FAKE_LOG").ok(),
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
