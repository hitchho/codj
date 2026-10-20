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

use dj_types::{DeckInfo, MixerInfo};
use types::{OuterMusic, OuterPlaylist, PlayerStatus, PlayingStatus, Repeat, Sort};

struct Deck {
    info: DeckInfo,
    nudge: f64,
    /// hold-to-preview from the cue button is active
    preview: bool,
    /// the sync master's tempo is followed
    follow: bool,
}

struct State {
    decks: [Deck; 2],
    mixer: MixerInfo,
    at: Instant,
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
    }
}

struct Fake {
    st: Arc<Mutex<State>>,
    log: Option<String>,
}

impl Fake {
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
            let stem = std::path::Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bpm = fake_bpm(&path);
            let keep = s.decks[i].info.clone();
            s.decks[i] = Deck {
                info: DeckInfo {
                    loaded: true,
                    path: path.clone(),
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
                    // keep the mixer position of the strip
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
