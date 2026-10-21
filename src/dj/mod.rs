//! Live DJ mode: two decks, a mixer and a library on one screen, driven from the keyboard.
//!
//! - `keys`: key map, hold/tap handling and ramps (pure)
//! - `wave`: waveform decoding and drawing (pure)
//! - `render`: the screen (pure, takes a [`render::View`])
//!
//! This file ties them to codjd: a poller task reads `DjDecks`/`DjMixer` at ~30 Hz into shared
//! state, an executor task sends the calls the keys produce (coalesced), and waveforms are
//! fetched once per loaded track. None of this blocks the UI loop.

pub mod capture;
pub mod keys;
pub mod render;
pub mod wave;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{layout::Rect, Frame};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tui_input::{Input, InputRequest};
use zbus::Connection;

use crate::dj_types::{DeckInfo, MixerInfo, SetState};
use crate::types::Music;
use crate::ServerProxy;
use keys::{coalesce, DjCall, DjKeys, Nav};
use render::{Focus, View, WaveView};
use wave::Waveform;

const POLL_EVERY: Duration = Duration::from_millis(33);
const CALL_TIMEOUT: Duration = Duration::from_millis(1500);
/// Columns asked from `GetWaveform`: more than the stored 50 per second, so the full resolution
const WAVE_COLUMNS: u32 = 60_000;
const NOTE_FOR: Duration = Duration::from_secs(6);
/// the set runtime is polled every 4th tick (~7.5 Hz) while a set is active, else once a second
const SET_POLL_ACTIVE: u32 = 4;
const SET_POLL_IDLE: u32 = 30;
/// ~4 Hz while recording, once a second otherwise (to notice a recording started from elsewhere)
const REC_POLL_ACTIVE: u32 = 7;
const REC_POLL_IDLE: u32 = 30;

#[derive(Clone)]
pub enum WaveSlot {
    Empty,
    Loading(String),
    Missing(String),
    Ready(String, Arc<Waveform>),
}

impl WaveSlot {
    fn path(&self) -> Option<&str> {
        match self {
            WaveSlot::Empty => None,
            WaveSlot::Loading(p) | WaveSlot::Missing(p) | WaveSlot::Ready(p, _) => Some(p),
        }
    }
}

struct Shared {
    decks: Vec<DeckInfo>,
    mixer: MixerInfo,
    /// polling failure, cleared by the next good poll
    poll_error: Option<String>,
    /// last non-ok reply of a call
    call_error: Option<(String, Instant)>,
    waves: [WaveSlot; 2],
    /// per deck: (path, the detectors disagree on its tempo), fetched once per load
    doubt: [Option<(String, bool)>; 2],
    /// the set runtime's state, None until the first good poll
    set: Option<SetState>,
    /// codjd has no `SetState`: polling stops
    set_missing: bool,
    /// polling the set runtime failed (shown only while the deck poll works)
    set_poll_error: Option<String>,
    /// a set call finished: poll again now
    set_poll_now: bool,
    /// a record call finished: poll the recording again now
    rec_poll_now: bool,
    /// `SetRecording`: (recording, beats, events); None until the first good poll
    rec: Option<(bool, f64, u32)>,
    /// codjd has no `SetRecording`: polling stops
    rec_missing: bool,
    /// feedback of a set command (list, "armed mix 2"), shown like a note
    info: Option<(String, Instant)>,
    /// the active set's errors, drawn over the decks until Esc
    overlay: Option<Vec<String>>,
}

/// Short text for a D-Bus failure; an old codjd without the deck API gets a clear hint
pub fn describe_error(what: &str, e: &zbus::Error) -> String {
    let text = e.to_string();
    if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
        format!("codjd has no deck API ({what}): update codjd")
    } else if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        "codjd is not running".to_string()
    } else {
        format!("{what}: {text}")
    }
}

/// Like [`describe_error`] for the set runtime, which is newer than the deck API
pub fn describe_set_error(what: &str, e: &zbus::Error) -> String {
    let text = e.to_string();
    if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
        "codjd has no set runtime: update codjd".to_string()
    } else {
        describe_error(what, e)
    }
}

/// A call to the set runtime. They run one after another in their own task, so `:set x` then
/// `:arm 1` arrive in order without waiting on the deck calls.
#[derive(Debug, Clone, PartialEq)]
pub enum SetCall {
    Use(String),
    Arm(u32),
    Disarm,
    Auto(bool),
    Preview(u32, f64),
    Load(u32),
    List,
    Errors(String),
    /// capture cue / loop `name` from this deck
    Cue(u32, String),
    LoopSave(u32, String),
    GridSave(u32),
    Record(bool),
}

impl SetCall {
    fn name(&self) -> &'static str {
        match self {
            SetCall::Use(_) => "SetUse",
            SetCall::Arm(_) => "SetArm",
            SetCall::Disarm => "SetDisarm",
            SetCall::Auto(_) => "SetAuto",
            SetCall::Preview(..) => "SetPreview",
            SetCall::Load(_) => "SetLoad",
            SetCall::List => "SetList",
            SetCall::Errors(_) => "SetErrors",
            SetCall::Cue(..) => "SetCue",
            SetCall::LoopSave(..) => "SetLoopCapture",
            SetCall::GridSave(_) => "SetGridCapture",
            SetCall::Record(_) => "SetRecord",
        }
    }

    /// what to say when codjd answered "ok" (mixes and tracks are numbered from 1 on screen)
    fn done(&self) -> String {
        match self {
            SetCall::Use(n) if n.is_empty() => "no set active".into(),
            SetCall::Use(n) => format!("set {n} active"),
            SetCall::Arm(m) => format!("armed mix {}", m + 1),
            SetCall::Disarm => "disarmed".into(),
            SetCall::Auto(on) => format!("auto {}", if *on { "on" } else { "off" }),
            SetCall::Preview(m, bars) => format!("previewing mix {} from {bars} bars before", m + 1),
            SetCall::Load(t) => format!("loaded track {}", t + 1),
            SetCall::List | SetCall::Errors(_) => String::new(),
            SetCall::Cue(d, n) => format!("cue {n} saved from deck {}", deck_letter(*d)),
            SetCall::LoopSave(d, n) => format!("loop {n} saved from deck {}", deck_letter(*d)),
            SetCall::GridSave(d) => format!("grid of deck {} saved", deck_letter(*d)),
            SetCall::Record(true) => "recording: play the mix, F9 or :rec off stops".into(),
            SetCall::Record(false) => "recording stopped".into(),
        }
    }

    /// what to say when codjd answered `ok <detail>`
    fn done_with(&self, detail: &str) -> String {
        let text = capture::pretty_reply(detail);
        match self {
            SetCall::Record(false) => format!("recorded {text}"),
            SetCall::Record(true) => format!("recording: {text}"),
            _ => text,
        }
    }
}

fn deck_letter(d: u32) -> char {
    if d == 0 { 'A' } else { 'B' }
}

/// `:set` output: the sets with their error counts
pub fn format_set_list(sets: &[(String, String, u32)]) -> String {
    if sets.is_empty() {
        return "no sets found in the set folder".into();
    }
    let items: Vec<String> = sets
        .iter()
        .map(|(name, _, errors)| if *errors > 0 { format!("{name} ✗{errors}") } else { format!("{name} ok") })
        .collect();
    format!("sets: {}  (:set <name>)", items.join(" · "))
}

enum SetReply {
    Status(String),
    List(Vec<(String, String, u32)>),
    Errors(Vec<String>),
}

async fn run_set_call(proxy: &ServerProxy<'_>, call: &SetCall) -> zbus::Result<SetReply> {
    Ok(match call {
        SetCall::Use(n) => SetReply::Status(proxy.set_use(n).await?),
        SetCall::Arm(m) => SetReply::Status(proxy.set_arm(*m).await?),
        SetCall::Disarm => SetReply::Status(proxy.set_disarm().await?),
        SetCall::Auto(on) => SetReply::Status(proxy.set_auto(*on).await?),
        SetCall::Preview(m, bars) => SetReply::Status(proxy.set_preview(*m, *bars).await?),
        SetCall::Load(t) => SetReply::Status(proxy.set_load(*t).await?),
        SetCall::List => SetReply::List(proxy.set_list().await?),
        SetCall::Errors(n) => SetReply::Errors(proxy.set_errors(n).await?),
        SetCall::Cue(d, n) => SetReply::Status(proxy.set_cue(*d, n).await?),
        SetCall::LoopSave(d, n) => SetReply::Status(proxy.set_loop_capture(*d, n).await?),
        SetCall::GridSave(d) => SetReply::Status(proxy.set_grid_capture(*d).await?),
        SetCall::Record(on) => SetReply::Status(proxy.set_record(*on).await?),
    })
}

/// The connection to codjd for one DJ session: tasks end when this is dropped
pub struct Session {
    shared: Arc<Mutex<Shared>>,
    tx: mpsc::UnboundedSender<DjCall>,
    set_tx: mpsc::UnboundedSender<SetCall>,
    conn: Connection,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        // only the poller is cut off; the executor ends by itself once the channel (dropped with
        // this struct) is drained, so releases queued just before leaving still go out
        if let Some(poller) = self.tasks.first() {
            poller.abort();
        }
    }
}

async fn run_call(proxy: &ServerProxy<'_>, call: &DjCall) -> zbus::Result<String> {
    match call {
        DjCall::Load { deck, path } => proxy.dj_load(*deck, path).await,
        DjCall::Play(d) => proxy.dj_play(*d).await,
        DjCall::Pause(d) => proxy.dj_pause(*d).await,
        DjCall::Cue { deck, pressed } => proxy.dj_cue(*deck, *pressed).await,
        DjCall::Sync(d) => proxy.dj_sync(*d).await,
        DjCall::Nudge { deck, amount } => proxy.dj_nudge(*deck, *amount).await,
        DjCall::BeatJump { deck, beats } => proxy.dj_beat_jump(*deck, *beats).await,
        DjCall::Loop { deck, beats } => proxy.dj_loop(*deck, *beats).await,
        DjCall::Hotcue { deck, slot, action } => proxy.dj_hotcue(*deck, *slot, action).await,
        DjCall::Set { control, deck, value } => proxy.dj_set(control, *deck, *value).await,
    }
}

/// Reads `SetState` when it is due: on the first tick, then ~7.5 Hz while a set is active and
/// once a second otherwise (to notice a set activated from elsewhere). Failures never touch the
/// deck poll; an old codjd without the method ends the polling.
async fn poll_set(proxy: &ServerProxy<'_>, shared: &Arc<Mutex<Shared>>, tick: u32) {
    let due = {
        let mut s = shared.lock().unwrap();
        let every = if s.set.as_ref().is_some_and(|x| x.is_active()) { SET_POLL_ACTIVE } else { SET_POLL_IDLE };
        let forced = std::mem::take(&mut s.set_poll_now);
        !s.set_missing && (forced || tick % every == 0)
    };
    if !due {
        return;
    }
    let reply = tokio::time::timeout(CALL_TIMEOUT, proxy.set_state()).await;
    let mut s = shared.lock().unwrap();
    match reply {
        Ok(Ok(state)) => {
            s.set = Some(state);
            s.set_poll_error = None;
        }
        Ok(Err(e)) => {
            let text = e.to_string();
            if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
                s.set_missing = true;
                s.set = None;
                s.set_poll_error = None;
            } else {
                s.set_poll_error = Some(describe_set_error("SetState", &e));
            }
        }
        Err(_) => s.set_poll_error = Some("SetState: codjd does not answer".into()),
    }
}

/// Reads `SetRecording` when it is due: ~4 Hz while recording, once a second otherwise
async fn poll_recording(proxy: &ServerProxy<'_>, shared: &Arc<Mutex<Shared>>, tick: u32) {
    let due = {
        let mut s = shared.lock().unwrap();
        let every = if s.rec.is_some_and(|r| r.0) { REC_POLL_ACTIVE } else { REC_POLL_IDLE };
        let forced = std::mem::take(&mut s.rec_poll_now);
        !s.rec_missing && (forced || tick % every == 0)
    };
    if !due {
        return;
    }
    let reply = tokio::time::timeout(CALL_TIMEOUT, proxy.set_recording()).await;
    let mut s = shared.lock().unwrap();
    match reply {
        Ok(Ok(state)) => s.rec = Some(state),
        Ok(Err(e)) => {
            let text = e.to_string();
            if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
                s.rec_missing = true;
                s.rec = None;
            }
        }
        Err(_) => {}
    }
}

impl Session {
    pub fn start(conn: Connection) -> Session {
        let shared = Arc::new(Mutex::new(Shared {
            decks: vec![DeckInfo::default(), DeckInfo::default()],
            mixer: MixerInfo::default(),
            poll_error: None,
            call_error: None,
            waves: [WaveSlot::Empty, WaveSlot::Empty],
            doubt: [None, None],
            set: None,
            set_missing: false,
            set_poll_error: None,
            set_poll_now: false,
            rec: None,
            rec_missing: false,
            rec_poll_now: false,
            info: None,
            overlay: None,
        }));
        let (tx, mut rx) = mpsc::unbounded_channel::<DjCall>();
        let (set_tx, mut set_rx) = mpsc::unbounded_channel::<SetCall>();

        let conn_for_set = conn.clone();
        let poller = {
            let (shared, conn) = (shared.clone(), conn.clone());
            tokio::spawn(async move {
                let Ok(proxy) = ServerProxy::new(&conn).await else { return };
                let mut every = tokio::time::interval(POLL_EVERY);
                every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut tick = 0u32;
                loop {
                    every.tick().await;
                    poll_set(&proxy, &shared, tick).await;
                    poll_recording(&proxy, &shared, tick).await;
                    tick = tick.wrapping_add(1);
                    let both = tokio::time::timeout(CALL_TIMEOUT, async {
                        let decks = proxy.dj_decks().await.map_err(|e| describe_error("DjDecks", &e))?;
                        let mixer = proxy.dj_mixer().await.map_err(|e| describe_error("DjMixer", &e))?;
                        Ok::<_, String>((decks, mixer))
                    })
                    .await;
                    let mut s = shared.lock().unwrap();
                    match both {
                        Ok(Ok((mut decks, mixer))) => {
                            decks.resize_with(2, DeckInfo::default);
                            s.decks = decks;
                            s.mixer = mixer;
                            s.poll_error = None;
                        }
                        Ok(Err(e)) => s.poll_error = Some(e),
                        Err(_) => s.poll_error = Some("codjd does not answer".into()),
                    }
                }
            })
        };

        let executor = {
            let (shared, conn) = (shared.clone(), conn.clone());
            tokio::spawn(async move {
                let Ok(proxy) = ServerProxy::new(&conn).await else { return };
                while let Some(first) = rx.recv().await {
                    let mut batch = vec![first];
                    while let Ok(more) = rx.try_recv() {
                        batch.push(more);
                    }
                    for call in coalesce(batch) {
                        let reply = tokio::time::timeout(CALL_TIMEOUT, run_call(&proxy, &call)).await;
                        let problem = match reply {
                            Ok(Ok(r)) if r == "ok" || r.is_empty() => None,
                            Ok(Ok(r)) => Some(format!("{}: {r}", call.name())),
                            Ok(Err(e)) => Some(describe_error(call.name(), &e)),
                            Err(_) => Some(format!("{}: codjd does not answer", call.name())),
                        };
                        if let Some(p) = problem {
                            shared.lock().unwrap().call_error = Some((p, Instant::now()));
                        }
                    }
                }
            })
        };

        let set_executor = {
            let shared = shared.clone();
            tokio::spawn(async move {
                let Ok(proxy) = ServerProxy::new(&conn_for_set).await else { return };
                while let Some(call) = set_rx.recv().await {
                    let reply = tokio::time::timeout(CALL_TIMEOUT, run_set_call(&proxy, &call)).await;
                    let mut s = shared.lock().unwrap();
                    let now = Instant::now();
                    match reply {
                        Ok(Ok(SetReply::Status(r))) if r == "ok" || r.is_empty() => {
                            s.info = Some((call.done(), now + NOTE_FOR));
                        }
                        Ok(Ok(SetReply::Status(r))) if r.starts_with("ok ") => {
                            s.info = Some((call.done_with(&r[3..]), now + NOTE_FOR));
                        }
                        Ok(Ok(SetReply::Status(r))) => s.call_error = Some((format!("{}: {r}", call.name()), now)),
                        Ok(Ok(SetReply::List(sets))) => s.info = Some((format_set_list(&sets), now + NOTE_FOR * 2)),
                        Ok(Ok(SetReply::Errors(lines))) => {
                            s.overlay = Some(if lines.is_empty() { vec!["no errors".to_string()] } else { lines });
                        }
                        Ok(Err(e)) => s.call_error = Some((describe_set_error(call.name(), &e), now)),
                        Err(_) => s.call_error = Some((format!("{}: codjd does not answer", call.name()), now)),
                    }
                    s.set_poll_now = true;
                    s.rec_poll_now = true;
                }
            })
        };

        Session { shared, tx, set_tx, conn, tasks: vec![poller, executor, set_executor] }
    }

    pub fn set_call(&self, call: SetCall) {
        let _ = self.set_tx.send(call);
    }

    pub fn send(&self, calls: Vec<DjCall>) {
        for c in calls {
            let _ = self.tx.send(c);
        }
    }

    /// Corrects a deck's grid (`DjGridScale`); the reply shows like any call's, and the deck's
    /// tempo-doubt flag is fetched again
    fn grid_scale(&self, deck: u8, factor: f64) {
        let (shared, conn) = (self.shared.clone(), self.conn.clone());
        tokio::spawn(async move {
            let reply = async {
                let proxy = ServerProxy::new(&conn).await?;
                proxy.dj_grid_scale(deck as u32, factor).await
            }
            .await;
            let mut s = shared.lock().unwrap();
            s.doubt[deck as usize] = None;
            let text = match reply {
                Ok(r) if r.starts_with("ok") => format!("grid {}: {} BPM", ["A", "B"][deck as usize], r.trim_start_matches("ok ")),
                Ok(r) => r,
                Err(e) => describe_error("DjGridScale", &e),
            };
            s.call_error = Some((text, Instant::now()));
        });
    }

    /// Fetches the waveform for any loaded track we do not have one for yet
    fn ensure_waves(&self) {
        let mut s = self.shared.lock().unwrap();
        for i in 0..2 {
            // whether the tempo detectors disagree: shown as a "?" next to the BPM
            let want = s.decks[i].loaded.then(|| s.decks[i].path.clone());
            let have = s.doubt[i].as_ref().map(|(p, _)| p.clone());
            if let Some(p) = want.filter(|p| have.as_deref() != Some(p.as_str())) {
                s.doubt[i] = Some((p.clone(), false));
                let (shared, conn) = (self.shared.clone(), self.conn.clone());
                tokio::spawn(async move {
                    let grid = async { ServerProxy::new(&conn).await.ok()?.get_grid(&p).await.ok() }.await;
                    if let Some(g) = grid {
                        let mut s = shared.lock().unwrap();
                        if s.doubt[i].as_ref().map(|(q, _)| q.as_str()) == Some(p.as_str()) {
                            s.doubt[i] = Some((p, g.6));
                        }
                    }
                });
            }
            let d = &s.decks[i];
            let want = d.loaded.then(|| d.path.clone());
            let have = s.waves[i].path().map(str::to_string);
            match (want, have) {
                (None, _) => s.waves[i] = WaveSlot::Empty,
                (Some(p), have) if have.as_deref() != Some(&p) => {
                    s.waves[i] = WaveSlot::Loading(p.clone());
                    let (shared, conn) = (self.shared.clone(), self.conn.clone());
                    tokio::spawn(async move {
                        let bytes = async {
                            let proxy = ServerProxy::new(&conn).await.ok()?;
                            tokio::time::timeout(Duration::from_secs(20), proxy.get_waveform(&p, WAVE_COLUMNS))
                                .await
                                .ok()?
                                .ok()
                        }
                        .await;
                        let slot = match bytes.as_deref().and_then(Waveform::decode) {
                            Some(wf) => WaveSlot::Ready(p.clone(), Arc::new(wf)),
                            None => WaveSlot::Missing(p.clone()),
                        };
                        let mut s = shared.lock().unwrap();
                        // the deck may have been loaded with something else meanwhile
                        if s.waves[i].path() == Some(p.as_str()) {
                            s.waves[i] = slot;
                        }
                    });
                }
                _ => {}
            }
        }
    }
}

/// What a key did to the screen
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Exit,
}

/// The library list with its search box and the `:` command line: everything about typing and
/// browsing, kept apart from the daemon so it can be tested
pub struct Pane {
    pub focus: Focus,
    input: Input,
    /// the search that filters `lib`
    search: String,
    all: Vec<Music>,
    lib: Vec<Music>,
    sel: usize,
}

/// What a `:` command asks for
#[derive(Debug, PartialEq)]
pub enum Command {
    /// back to the normal view
    Exit,
    Load(u8),
    /// correct a deck's detected tempo by a factor; 0 resets
    Grid(u8, f64),
    /// `:set` alone: list the sets
    SetList,
    /// `:set <name>`, "" = none
    SetUse(String),
    SetArm(ArmTarget),
    SetDisarm,
    /// `:auto on|off`, bare `:auto` toggles
    SetAuto(Option<bool>),
    /// mix (0-based), bars before the mix start
    SetPreview(u32, f64),
    /// track (0-based) onto the free deck
    SetLoad(u32),
    SetErrors,
    SetEdit,
    /// `:cue <name> [a|b]`: capture the deck's position (None = the deck rule of `capture::pick_deck`)
    Cue(String, Option<u8>),
    LoopSave(String, Option<u8>),
    GridSave(Option<u8>),
    /// `:rec on|off`, bare `:rec` toggles
    Record(Option<bool>),
    Nothing,
    Unknown(String),
}

/// Which mix `:arm` means
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ArmTarget {
    /// 0-based
    Mix(u32),
    Next,
}

/// Bars before the mix start that `:preview` rolls in when none are given
pub const PREVIEW_BARS: f64 = 8.0;

/// A 1-based number as typed on screen to the 0-based index codjd wants
fn one_based(word: Option<&str>) -> Option<u32> {
    word?.parse::<u32>().ok()?.checked_sub(1)
}

/// `:` commands inside DJ mode. Quitting codj is deliberately not one of them.
pub fn parse_command(cmd: &str) -> Command {
    let mut words = cmd.split_whitespace();
    match words.next() {
        Some("q" | "quit" | "q!" | "dj" | "normal" | "back" | "x" | "wq") => Command::Exit,
        Some("load") => match words.next() {
            Some("a" | "A") => Command::Load(0),
            Some("b" | "B") => Command::Load(1),
            Some(n) => one_based(Some(n))
                .map(Command::SetLoad)
                .unwrap_or_else(|| Command::Unknown("usage: load a|b, or load <track number from 1> (set)".into())),
            None => Command::Unknown("usage: load a|b, or load <track number from 1> (set)".into()),
        },
        Some("grid") => {
            let deck = match words.next() {
                Some("a" | "A") => Some(0),
                Some("b" | "B") => Some(1),
                _ => None,
            };
            let factor = match words.next() {
                Some("x2" | "*2" | "double") => Some(2.),
                Some("/2" | "half") => Some(0.5),
                Some("x1.5" | "*1.5") => Some(1.5),
                Some("/1.5") => Some(2. / 3.),
                Some("reset") => Some(0.),
                _ => None,
            };
            match (deck, factor) {
                (Some(d), Some(f)) => Command::Grid(d, f),
                _ => Command::Unknown("usage: grid a|b x2|/2|x1.5|/1.5|reset".into()),
            }
        }
        Some("set") => {
            let name = words.collect::<Vec<_>>().join(" ");
            match name.as_str() {
                "" => Command::SetList,
                "none" | "off" | "-" => Command::SetUse(String::new()),
                _ => Command::SetUse(name),
            }
        }
        Some("arm") => match words.next() {
            Some("next" | "n") => Command::SetArm(ArmTarget::Next),
            w => one_based(w)
                .map(|m| Command::SetArm(ArmTarget::Mix(m)))
                .unwrap_or_else(|| Command::Unknown("usage: arm <mix number from 1>|next".into())),
        },
        Some("disarm") => Command::SetDisarm,
        Some("auto") => match words.next() {
            None => Command::SetAuto(None),
            Some("on" | "1" | "true") => Command::SetAuto(Some(true)),
            Some("off" | "0" | "false") => Command::SetAuto(Some(false)),
            Some(_) => Command::Unknown("usage: auto on|off".into()),
        },
        Some("preview") => {
            let mix = words.next();
            let bars = match words.next() {
                None => Some(PREVIEW_BARS),
                Some(b) => b.parse::<f64>().ok().filter(|b| *b > 0.0 && b.is_finite()),
            };
            match (one_based(mix), bars) {
                (Some(m), Some(b)) => Command::SetPreview(m, b),
                _ => Command::Unknown(format!("usage: preview <mix number from 1> [bars, default {PREVIEW_BARS}]")),
            }
        }
        Some("errors") => Command::SetErrors,
        Some("edit") => Command::SetEdit,
        Some(w @ ("cue" | "loopsave" | "gridsave" | "rec" | "record")) => {
            capture::parse(w, &words.collect::<Vec<_>>()).unwrap_or(Command::Nothing)
        }
        None => Command::Nothing,
        Some(other) => Command::Unknown(format!(
            "unknown DJ command: {other} (q: back, load a|b|<track>, grid a|b x2|/2|.., set [name], arm <n|next>, disarm, auto on|off, preview <n> [bars], errors, edit, cue <name> [a|b], loopsave <name> [a|b], gridsave [a|b], rec [on|off])"
        )),
    }
}

impl Pane {
    pub fn new(library: Vec<Music>) -> Self {
        Pane {
            focus: Focus::Decks,
            input: Input::default(),
            search: String::new(),
            lib: library.clone(),
            all: library,
            sel: 0,
        }
    }

    pub fn typing(&self) -> bool {
        self.focus != Focus::Decks
    }

    fn refilter(&mut self) {
        self.lib = crate::search::search_musics(&self.search, &self.all);
        self.sel = self.sel.min(self.lib.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<&Music> {
        self.lib.get(self.sel)
    }

    pub fn selected_path(&self) -> Option<String> {
        self.selected().map(|m| <&std::path::Path>::from(&m.path).to_string_lossy().into_owned())
    }

    pub fn start_search(&mut self) {
        self.input = Input::new(self.search.clone());
        self.focus = Focus::Search;
    }

    pub fn start_command(&mut self) {
        self.input = Input::default();
        self.focus = Focus::Command;
    }

    pub fn scroll(&mut self, by: isize) {
        let last = self.lib.len().saturating_sub(1) as isize;
        self.sel = (self.sel as isize + by).clamp(0, last) as usize;
    }

    /// A key while the search box or command line is open. Returns the command line's text when
    /// it was submitted.
    pub fn handle_typing(&mut self, key: &KeyEvent) -> Option<String> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let searching = self.focus == Focus::Search;
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        let mut submitted = None;
        match key.code {
            KeyCode::Esc => {
                if searching {
                    self.search.clear();
                    self.refilter();
                }
                self.focus = Focus::Decks;
            }
            KeyCode::Tab if searching => self.focus = Focus::Decks,
            KeyCode::Enter => {
                if !searching {
                    submitted = Some(self.input.value().trim().to_string());
                }
                self.focus = Focus::Decks;
            }
            KeyCode::Up if searching => self.scroll(-1),
            KeyCode::Down if searching => self.scroll(1),
            KeyCode::Char(c) if plain => {
                self.input.handle(InputRequest::InsertChar(c));
            }
            KeyCode::Backspace => {
                self.input.handle(InputRequest::DeletePrevChar);
            }
            KeyCode::Delete => {
                self.input.handle(InputRequest::DeleteNextChar);
            }
            KeyCode::Left => {
                self.input.handle(InputRequest::GoToPrevChar);
            }
            KeyCode::Right => {
                self.input.handle(InputRequest::GoToNextChar);
            }
            _ => {}
        }
        if searching && self.focus == Focus::Search {
            self.search = self.input.value().to_string();
            self.refilter();
            self.sel = 0;
        }
        submitted
    }
}

/// The mix `F6` / `:arm next` should arm, or why none: the one after a running mix; else the
/// first mix whose outgoing track is on a deck (preferring a playing one)
pub fn next_mix(set: &SetState, decks: &[DeckInfo]) -> Result<u32, String> {
    if !set.is_active() {
        return Err("no active set (:set <name>)".into());
    }
    if let Some(armed) = set.armed_mix() {
        if !set.running {
            return Err(format!("mix {} is already armed (F8 disarms)", armed + 1));
        }
        return if armed + 1 < set.mixes.len() {
            Ok(armed as u32 + 1)
        } else {
            Err("that was the last mix".into())
        };
    }
    let on_deck = |track: &str| {
        let path = set.track_index(track).map(|i| set.tracks[i].2.as_str())?;
        decks.iter().position(|d| d.loaded && d.path == path)
    };
    let mut first = None;
    for (i, mix) in set.mixes.iter().enumerate() {
        if let Some(deck) = on_deck(&mix.0) {
            if decks[deck].playing {
                return Ok(i as u32);
            }
            first.get_or_insert(i as u32);
        }
    }
    first.ok_or_else(|| "no mix starts from a track on a deck: load one with :load <n>".into())
}

/// Line to run `$EDITOR` on a file through the shell, so `EDITOR="nvim -u NONE"` works too:
/// `$EDITOR`, else `$VISUAL`, else `vi`. The file is passed as `$0`.
pub fn editor_line(editor: Option<&str>, visual: Option<&str>) -> String {
    let pick = |v: Option<&str>| v.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    format!("{} \"$0\"", pick(editor).or_else(|| pick(visual)).unwrap_or_else(|| "vi".into()))
}

/// The keyboard-enhancement levels codj has pushed, bottom first, in the order they were pushed:
/// the base one from startup, DJ mode's key-release reporting, and the plain one while typing.
/// Suspending for the editor pops them all and pushes exactly these back.
pub fn kbd_levels(enhanced: bool, dj_pushed: bool, text_pushed: bool) -> Vec<crossterm::event::KeyboardEnhancementFlags> {
    use crossterm::event::KeyboardEnhancementFlags as F;
    let mut levels = Vec::new();
    if enhanced {
        levels.push(F::DISAMBIGUATE_ESCAPE_CODES);
    }
    if dj_pushed {
        levels.push(F::DISAMBIGUATE_ESCAPE_CODES | F::REPORT_EVENT_TYPES | F::REPORT_ALL_KEYS_AS_ESCAPE_CODES);
    }
    if text_pushed {
        levels.push(F::DISAMBIGUATE_ESCAPE_CODES);
    }
    levels
}

pub struct DjScreen {
    session: Session,
    keys: DjKeys,
    pane: Pane,
    note: Option<(String, Instant)>,
    /// `:edit` asked for the editor on this file; main suspends the terminal and runs it
    edit_request: Option<std::path::PathBuf>,
}

impl DjScreen {
    pub fn new(conn: Connection, library: Vec<Music>, release_events: bool) -> Self {
        let note = (!release_events).then(|| {
            (
                "No key-release events in this terminal: cue toggles, nudge is a short bend, holds step per key repeat. Use kitty/foot/WezTerm/Ghostty for hold."
                    .to_string(),
                Instant::now() + Duration::from_secs(12),
            )
        });
        DjScreen {
            session: Session::start(conn),
            keys: DjKeys::new(release_events),
            pane: Pane::new(library),
            note,
            edit_request: None,
        }
    }

    /// the search box or command line has the keyboard (text is typed, not performed)
    pub fn typing(&self) -> bool {
        self.pane.typing()
    }

    fn set_note(&mut self, text: impl Into<String>) {
        self.note = Some((text.into(), Instant::now() + NOTE_FOR));
    }

    /// The file `:edit` wants opened, once
    pub fn take_edit_request(&mut self) -> Option<std::path::PathBuf> {
        self.edit_request.take()
    }

    /// Message after the editor ran
    pub fn edited(&mut self, text: impl Into<String>) {
        self.set_note(text);
    }

    /// the set runtime's state and the decks, as last polled
    fn set_snapshot(&self) -> (Option<SetState>, Vec<DeckInfo>, bool) {
        let s = self.session.shared.lock().unwrap();
        (s.set.clone(), s.decks.clone(), s.set_missing)
    }

    /// The active set, or a note saying why there is none
    fn active_set(&mut self) -> Option<(SetState, Vec<DeckInfo>)> {
        let (set, decks, missing) = self.set_snapshot();
        match set.filter(|s| s.is_active()) {
            Some(set) => Some((set, decks)),
            None => {
                self.set_note(if missing {
                    "codjd has no set runtime: update codjd"
                } else {
                    "no active set (:set lists them, :set <name> activates one)"
                });
                None
            }
        }
    }

    /// a 0-based mix index that exists in the active set
    fn check_mix(&mut self, mix: u32) -> Option<()> {
        let (set, _) = self.active_set()?;
        if (mix as usize) < set.mixes.len() {
            Some(())
        } else {
            self.set_note(format!("no mix {} (the set has {})", mix + 1, set.mixes.len()));
            None
        }
    }

    fn set_arm(&mut self, target: ArmTarget) {
        let Some((set, decks)) = self.active_set() else { return };
        let mix = match target {
            ArmTarget::Mix(m) => Ok(m),
            ArmTarget::Next => next_mix(&set, &decks),
        };
        match mix {
            Ok(m) if (m as usize) < set.mixes.len() => self.session.set_call(SetCall::Arm(m)),
            Ok(m) => self.set_note(format!("no mix {} (the set has {})", m + 1, set.mixes.len())),
            Err(e) => self.set_note(e),
        }
    }

    fn set_auto(&mut self, on: Option<bool>) {
        let Some((set, _)) = self.active_set() else { return };
        self.session.set_call(SetCall::Auto(on.unwrap_or(!set.auto)));
    }

    fn set_edit(&mut self) {
        let Some((set, _)) = self.active_set() else { return };
        if set.file.is_empty() {
            self.set_note("the active set has no file");
        } else {
            self.edit_request = Some(set.file.into());
        }
    }

    fn set_errors(&mut self) {
        let Some((set, _)) = self.active_set() else { return };
        self.session.set_call(SetCall::Errors(set.name));
    }

    /// the deck to capture from: the one named, else by the rule in `capture::pick_deck`
    fn capture_deck(&mut self, named: Option<u8>) -> Option<u32> {
        if let Some(d) = named {
            return Some(d as u32);
        }
        let (decks, mixer) = {
            let s = self.session.shared.lock().unwrap();
            (s.decks.clone(), s.mixer.clone())
        };
        match capture::pick_deck(&decks, &mixer) {
            Ok(d) => Some(d),
            Err(e) => {
                self.set_note(e);
                None
            }
        }
    }

    fn capture(&mut self, make: impl FnOnce(u32) -> SetCall, named: Option<u8>) {
        if self.active_set().is_none() {
            return;
        }
        if let Some(deck) = self.capture_deck(named) {
            self.session.set_call(make(deck));
        }
    }

    /// `None` toggles by the last polled state
    fn record(&mut self, on: Option<bool>) {
        if self.active_set().is_none() {
            return;
        }
        let recording = self.session.shared.lock().unwrap().rec.is_some_and(|r| r.0);
        self.session.set_call(SetCall::Record(on.unwrap_or(!recording)));
    }

    fn load(&mut self, deck: u8) {
        match (self.pane.selected_path(), self.pane.selected()) {
            (Some(path), Some(m)) => {
                let name = m.title.clone();
                self.session.send(vec![DjCall::Load { deck: deck as u32, path }]);
                self.set_note(format!("loading {name} onto deck {}", if deck == 0 { "A" } else { "B" }));
            }
            _ => self.set_note("nothing selected in the library"),
        }
    }

    /// Handles a key; performance keys are queued for the next [`tick`]
    pub fn handle_key(&mut self, key: &KeyEvent, now: Instant) -> Outcome {
        // Esc closes the error overlay first
        if key.code == KeyCode::Esc && key.kind != KeyEventKind::Release {
            if self.session.shared.lock().unwrap().overlay.take().is_some() {
                return Outcome::Stay;
            }
        }
        if self.pane.typing() {
            return match self.pane.handle_typing(key) {
                Some(cmd) => self.run_command(&cmd),
                None => Outcome::Stay,
            };
        }
        let Some(nav) = self.keys.on_key(key, now) else {
            return Outcome::Stay;
        };
        match nav {
            Nav::Exit => return Outcome::Exit,
            // typing switches key-release reporting off: end holds now, their release would
            // never arrive
            Nav::Command => {
                self.settle(now);
                self.pane.start_command()
            }
            Nav::Search => {
                self.settle(now);
                self.pane.start_search()
            }
            Nav::LibUp => self.pane.scroll(-1),
            Nav::LibDown => self.pane.scroll(1),
            Nav::LibPageUp => self.pane.scroll(-5),
            Nav::LibPageDown => self.pane.scroll(5),
            Nav::Load(d) => self.load(d),
            Nav::SetArmNext => self.set_arm(ArmTarget::Next),
            Nav::SetAuto => self.set_auto(None),
            Nav::SetDisarm => {
                if self.active_set().is_some() {
                    self.session.set_call(SetCall::Disarm);
                }
            }
            Nav::RecordToggle => self.record(None),
            Nav::LoadSmart => {
                // the deck that is not playing; both idle: A
                let playing = |i: usize| self.session.shared.lock().unwrap().decks[i].playing;
                let deck = if playing(0) && !playing(1) { 1 } else { 0 };
                self.load(deck);
            }
        }
        Outcome::Stay
    }

    /// Ends all holds and sends whatever is pending, at once
    pub fn settle(&mut self, now: Instant) {
        self.keys.release_all();
        let calls = self.keys.tick(now);
        self.session.send(calls);
    }

    fn run_command(&mut self, cmd: &str) -> Outcome {
        match parse_command(cmd) {
            Command::Exit => return Outcome::Exit,
            Command::Load(d) => self.load(d),
            Command::Grid(d, f) => self.session.grid_scale(d, f),
            Command::SetList => self.session.set_call(SetCall::List),
            Command::SetUse(name) => self.session.set_call(SetCall::Use(name)),
            Command::SetArm(t) => self.set_arm(t),
            Command::SetDisarm => {
                if self.active_set().is_some() {
                    self.session.set_call(SetCall::Disarm);
                }
            }
            Command::SetAuto(on) => self.set_auto(on),
            Command::SetPreview(m, bars) => {
                if self.check_mix(m).is_some() {
                    self.session.set_call(SetCall::Preview(m, bars));
                }
            }
            Command::SetLoad(t) => {
                if let Some((set, _)) = self.active_set() {
                    if (t as usize) < set.tracks.len() {
                        self.session.set_call(SetCall::Load(t));
                    } else {
                        self.set_note(format!("no track {} (the set has {})", t + 1, set.tracks.len()));
                    }
                }
            }
            Command::SetErrors => self.set_errors(),
            Command::SetEdit => self.set_edit(),
            Command::Cue(name, d) => self.capture(|deck| SetCall::Cue(deck, name), d),
            Command::LoopSave(name, d) => self.capture(|deck| SetCall::LoopSave(deck, name), d),
            Command::GridSave(d) => self.capture(SetCall::GridSave, d),
            Command::Record(on) => self.record(on),
            Command::Nothing => {}
            Command::Unknown(msg) => self.set_note(msg),
        }
        Outcome::Stay
    }

    /// Call every loop iteration: advances ramps, sends calls, fetches waveforms
    pub fn tick(&mut self, now: Instant) {
        let (decks, mixer) = {
            let s = self.session.shared.lock().unwrap();
            (s.decks.clone(), s.mixer.clone())
        };
        self.keys.set_snapshot(&decks, &mixer);
        let calls = self.keys.tick(now);
        self.session.send(calls);
        self.session.ensure_waves();
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area: Rect = frame.area();
        let s = self.session.shared.lock().unwrap();
        let wave_view = |slot: &'_ WaveSlot| match slot {
            WaveSlot::Empty => (None, WaveKind::None),
            WaveSlot::Loading(_) => (None, WaveKind::Loading),
            WaveSlot::Missing(_) => (None, WaveKind::Missing),
            WaveSlot::Ready(_, wf) => (Some(wf.clone()), WaveKind::Ready),
        };
        let waves: Vec<_> = s.waves.iter().map(wave_view).collect();
        let doubt = [0, 1].map(|i| s.doubt[i].as_ref().is_some_and(|(_, d)| *d));
        let view_of = |i: usize| match (&waves[i].0, &waves[i].1) {
            (Some(wf), _) => WaveView::Ready(wf),
            (None, WaveKind::Loading) => WaveView::Loading,
            (None, WaveKind::Missing) => WaveView::Missing,
            _ => WaveView::None,
        };
        let link_ok = s.poll_error.is_none();
        let error = s
            .poll_error
            .clone()
            .or_else(|| s.set_poll_error.clone())
            .or_else(|| s.call_error.as_ref().filter(|(_, t)| t.elapsed() < NOTE_FOR).map(|(e, _)| e.clone()));
        // the newer of the screen's note and a set command's feedback
        let now = Instant::now();
        let note = [self.note.as_ref(), s.info.as_ref()]
            .into_iter()
            .flatten()
            .filter(|(_, until)| now < *until)
            .max_by_key(|(_, until)| *until)
            .map(|(n, _)| n.as_str());
        let view = View {
            decks: &s.decks,
            mixer: &s.mixer,
            waves: [view_of(0), view_of(1)],
            doubt,
            link_ok,
            error: error.as_deref(),
            note,
            focus: self.pane.focus,
            input: self.pane.input.value(),
            search: &self.pane.search,
            lib: &self.pane.lib,
            lib_total: self.pane.all.len(),
            lib_sel: self.pane.sel,
            release_events: self.keys.release_events(),
            set: s.set.as_ref().filter(|x| x.is_active()),
            rec: s.rec.filter(|r| r.0).map(|r| (r.1, r.2)),
            overlay: s.overlay.as_deref(),
        };
        render::draw(frame.buffer_mut(), area, &view);
    }
}

enum WaveKind {
    None,
    Loading,
    Missing,
    Ready,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn music(title: &str, artist: &str) -> Music {
        Music {
            title: title.into(),
            artist: artist.into(),
            path: std::path::PathBuf::from(format!("/m/{title}.flac")).into(),
            ..Default::default()
        }
    }

    fn pane() -> Pane {
        Pane::new(vec![
            music("Aftertekno", "Pneumatix"),
            music("Smokin", "Aasi"),
            music("Hot minute", "Nlogax"),
        ])
    }

    fn type_str(p: &mut Pane, s: &str) {
        for c in s.chars() {
            p.handle_typing(&key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn search_filters_with_the_library_regex() {
        let mut p = pane();
        p.start_search();
        assert!(p.typing());
        type_str(&mut p, "artist:^n");
        assert_eq!(p.lib.len(), 1);
        assert_eq!(p.selected().unwrap().title, "Hot minute");
        // Enter keeps the filter and gives the keyboard back to the decks
        assert_eq!(p.handle_typing(&key(KeyCode::Enter)), None);
        assert!(!p.typing());
        assert_eq!(p.lib.len(), 1);
        // Tab edits it again, Esc clears it
        p.start_search();
        assert_eq!(p.input.value(), "artist:^n");
        p.handle_typing(&key(KeyCode::Esc));
        assert_eq!(p.lib.len(), 3);
        assert!(!p.typing());
    }

    #[test]
    fn typed_performance_letters_go_to_the_search_box() {
        let mut p = pane();
        p.start_search();
        type_str(&mut p, "zxcv");
        assert_eq!(p.input.value(), "zxcv");
        assert!(p.lib.is_empty());
        p.handle_typing(&key(KeyCode::Backspace));
        assert_eq!(p.input.value(), "zxc");
    }

    #[test]
    fn release_events_do_not_type() {
        let mut p = pane();
        p.start_search();
        let mut e = key(KeyCode::Char('a'));
        e.kind = KeyEventKind::Release;
        p.handle_typing(&e);
        assert_eq!(p.input.value(), "");
    }

    #[test]
    fn scrolling_stays_in_range() {
        let mut p = pane();
        p.scroll(-1);
        assert_eq!(p.sel, 0);
        p.scroll(10);
        assert_eq!(p.sel, 2);
        assert_eq!(p.selected_path().unwrap(), "/m/Hot minute.flac");
    }

    #[test]
    fn command_line_returns_the_command_on_enter_only() {
        let mut p = pane();
        p.start_command();
        assert_eq!(p.handle_typing(&key(KeyCode::Char('q'))), None);
        assert_eq!(p.handle_typing(&key(KeyCode::Enter)), Some("q".to_string()));
        assert!(!p.typing());
        p.start_command();
        type_str(&mut p, "load a");
        p.handle_typing(&key(KeyCode::Esc));
        assert!(!p.typing());
    }

    #[test]
    fn commands() {
        assert_eq!(parse_command("q"), Command::Exit);
        assert_eq!(parse_command("dj"), Command::Exit);
        assert_eq!(parse_command("load b"), Command::Load(1));
        assert_eq!(parse_command("load A"), Command::Load(0));
        assert_eq!(parse_command(""), Command::Nothing);
        assert!(matches!(parse_command("load"), Command::Unknown(_)));
        assert_eq!(parse_command("grid a x2"), Command::Grid(0, 2.));
        assert_eq!(parse_command("grid B /1.5"), Command::Grid(1, 2. / 3.));
        assert_eq!(parse_command("grid a reset"), Command::Grid(0, 0.));
        assert!(matches!(parse_command("grid c x2"), Command::Unknown(_)));
        assert!(matches!(parse_command("qa!"), Command::Unknown(_)));
    }

    #[test]
    fn capture_commands_and_replies() {
        assert_eq!(parse_command("cue drop"), Command::Cue("drop".into(), None));
        assert_eq!(parse_command("cue drop b"), Command::Cue("drop".into(), Some(1)));
        assert_eq!(parse_command("loopsave brk a"), Command::LoopSave("brk".into(), Some(0)));
        assert_eq!(parse_command("gridsave"), Command::GridSave(None));
        assert_eq!(parse_command("rec"), Command::Record(None));
        assert_eq!(parse_command("rec off"), Command::Record(Some(false)));
        assert!(matches!(parse_command("cue"), Command::Unknown(_)));
        assert_eq!(SetCall::Record(false).done_with("mix 2: aft -> smk, 16 bars, 9 items"), "recorded mix 2: aft → smk, 16 bars, 9 items");
        assert_eq!(SetCall::Cue(0, "drop".into()).done_with("cue drop -> aft bar 33"), "cue drop → aft bar 33");
        assert_eq!(SetCall::Cue(1, "drop".into()).name(), "SetCue");
    }

    #[test]
    fn set_commands() {
        assert_eq!(parse_command("set"), Command::SetList);
        assert_eq!(parse_command("set demo"), Command::SetUse("demo".into()));
        assert_eq!(parse_command("set kaiku friday"), Command::SetUse("kaiku friday".into()));
        assert_eq!(parse_command("set none"), Command::SetUse(String::new()));
        // mixes and tracks are numbered from 1 on screen, 0-based on the bus
        assert_eq!(parse_command("arm 1"), Command::SetArm(ArmTarget::Mix(0)));
        assert_eq!(parse_command("arm next"), Command::SetArm(ArmTarget::Next));
        assert!(matches!(parse_command("arm"), Command::Unknown(_)));
        assert!(matches!(parse_command("arm 0"), Command::Unknown(_)));
        assert!(matches!(parse_command("arm x"), Command::Unknown(_)));
        assert_eq!(parse_command("disarm"), Command::SetDisarm);
        assert_eq!(parse_command("auto on"), Command::SetAuto(Some(true)));
        assert_eq!(parse_command("auto off"), Command::SetAuto(Some(false)));
        assert_eq!(parse_command("auto"), Command::SetAuto(None));
        assert!(matches!(parse_command("auto maybe"), Command::Unknown(_)));
        assert_eq!(parse_command("preview 2"), Command::SetPreview(1, 8.0));
        assert_eq!(parse_command("preview 2 4"), Command::SetPreview(1, 4.0));
        assert_eq!(parse_command("preview 1 0.5"), Command::SetPreview(0, 0.5));
        assert!(matches!(parse_command("preview"), Command::Unknown(_)));
        assert!(matches!(parse_command("preview 1 -3"), Command::Unknown(_)));
        assert!(matches!(parse_command("preview 1 nan"), Command::Unknown(_)));
        // `:load` takes a deck letter or a track number
        assert_eq!(parse_command("load 3"), Command::SetLoad(2));
        assert_eq!(parse_command("load a"), Command::Load(0));
        assert!(matches!(parse_command("load 0"), Command::Unknown(_)));
        assert!(matches!(parse_command("load zz"), Command::Unknown(_)));
        assert_eq!(parse_command("errors"), Command::SetErrors);
        assert_eq!(parse_command("edit"), Command::SetEdit);
    }

    fn set_with(mixes: &[(&str, &str)]) -> SetState {
        let tracks: Vec<_> = ["a", "b", "c"].iter().map(|id| (id.to_string(), id.to_uppercase(), format!("/m/{id}.flac"))).collect();
        SetState {
            name: "demo".into(),
            file: "/s/demo.codj".into(),
            tracks,
            mixes: mixes.iter().map(|(f, t)| (f.to_string(), t.to_string(), 64.0, 128.0)).collect(),
            armed: -1,
            out_deck: -1,
            ..Default::default()
        }
    }

    fn deck(path: &str, playing: bool) -> DeckInfo {
        DeckInfo { loaded: !path.is_empty(), path: path.into(), playing, ..Default::default() }
    }

    #[test]
    fn next_mix_picks_the_mix_that_can_run() {
        let set = set_with(&[("a", "b"), ("b", "c")]);
        // nothing on the decks
        assert!(next_mix(&set, &[deck("", false), deck("", false)]).unwrap_err().contains("load"));
        // the first track is loaded: mix 1
        assert_eq!(next_mix(&set, &[deck("/m/a.flac", false), deck("", false)]), Ok(0));
        // B is the one playing, so mix 2 wins over the loaded-but-idle A
        assert_eq!(next_mix(&set, &[deck("/m/a.flac", false), deck("/m/b.flac", true)]), Ok(1));
        // inactive set
        assert!(next_mix(&SetState::default(), &[]).is_err());
        // an armed mix waiting to start is not re-armed by a second press
        let mut armed = set.clone();
        armed.armed = 0;
        assert!(next_mix(&armed, &[deck("/m/a.flac", true), deck("/m/b.flac", false)]).unwrap_err().contains("already armed"));
        // a running one: the next
        armed.running = true;
        assert_eq!(next_mix(&armed, &[]), Ok(1));
        armed.armed = 1;
        assert!(next_mix(&armed, &[]).unwrap_err().contains("last mix"));
    }

    #[test]
    fn set_list_text() {
        assert_eq!(format_set_list(&[]), "no sets found in the set folder");
        let text = format_set_list(&[("demo".into(), "/s/demo.codj".into(), 0), ("kaiku".into(), "/s/k.codj".into(), 2)]);
        assert!(text.contains("demo ok") && text.contains("kaiku ✗2"), "{text}");
    }

    #[test]
    fn set_errors_say_to_update_codjd() {
        let e = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.UnknownMethod").unwrap(),
            Some("Unknown method 'SetState'".into()),
            zbus::message::Message::method_call("/", "x").unwrap().build(&()).unwrap(),
        );
        assert_eq!(describe_set_error("SetState", &e), "codjd has no set runtime: update codjd");
    }

    #[test]
    fn editor_choice_and_keyboard_levels() {
        assert_eq!(editor_line(Some("nvim"), Some("code")), "nvim \"$0\"");
        assert_eq!(editor_line(Some("  "), Some("nano -w")), "nano -w \"$0\"");
        assert_eq!(editor_line(None, None), "vi \"$0\"");
        use crossterm::event::KeyboardEnhancementFlags as F;
        assert!(kbd_levels(false, false, false).is_empty());
        assert_eq!(kbd_levels(true, false, false), vec![F::DISAMBIGUATE_ESCAPE_CODES]);
        let full = kbd_levels(true, true, true);
        assert_eq!(full.len(), 3);
        assert!(full[1].contains(F::REPORT_EVENT_TYPES) && !full[2].contains(F::REPORT_EVENT_TYPES));
    }

    #[test]
    fn friendly_errors() {
        let e = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.UnknownMethod").unwrap(),
            Some("Unknown method 'DjDecks'".into()),
            zbus::message::Message::method_call("/", "x").unwrap().build(&()).unwrap(),
        );
        assert_eq!(describe_error("DjDecks", &e), "codjd has no deck API (DjDecks): update codjd");
    }
}
