//! Live DJ mode: two decks, a mixer and a library on one screen, driven from the keyboard.
//!
//! - `keys`: key map, hold/tap handling and ramps (pure)
//! - `wave`: waveform decoding and drawing (pure)
//! - `render`: the screen (pure, takes a [`render::View`])
//!
//! This file ties them to codjd: a poller task reads `DjDecks`/`DjMixer` at ~30 Hz into shared
//! state, an executor task sends the calls the keys produce (coalesced), and waveforms are
//! fetched once per loaded track. None of this blocks the UI loop.

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

use crate::dj_types::{DeckInfo, MixerInfo};
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

/// The connection to codjd for one DJ session: tasks end when this is dropped
pub struct Session {
    shared: Arc<Mutex<Shared>>,
    tx: mpsc::UnboundedSender<DjCall>,
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

impl Session {
    pub fn start(conn: Connection) -> Session {
        let shared = Arc::new(Mutex::new(Shared {
            decks: vec![DeckInfo::default(), DeckInfo::default()],
            mixer: MixerInfo::default(),
            poll_error: None,
            call_error: None,
            waves: [WaveSlot::Empty, WaveSlot::Empty],
            doubt: [None, None],
        }));
        let (tx, mut rx) = mpsc::unbounded_channel::<DjCall>();

        let poller = {
            let (shared, conn) = (shared.clone(), conn.clone());
            tokio::spawn(async move {
                let Ok(proxy) = ServerProxy::new(&conn).await else { return };
                let mut every = tokio::time::interval(POLL_EVERY);
                every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    every.tick().await;
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

        Session { shared, tx, conn, tasks: vec![poller, executor] }
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
    Nothing,
    Unknown(String),
}

/// `:` commands inside DJ mode. Quitting codj is deliberately not one of them.
pub fn parse_command(cmd: &str) -> Command {
    let mut words = cmd.split_whitespace();
    match words.next() {
        Some("q" | "quit" | "q!" | "dj" | "normal" | "back" | "x" | "wq") => Command::Exit,
        Some("load") => match words.next() {
            Some("a" | "A") => Command::Load(0),
            Some("b" | "B") => Command::Load(1),
            _ => Command::Unknown("usage: load a|b".into()),
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
        None => Command::Nothing,
        Some(other) => Command::Unknown(format!(
            "unknown DJ command: {other} (q: back, load a|b, grid a|b x2|/2|x1.5|/1.5|reset)"
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

pub struct DjScreen {
    session: Session,
    keys: DjKeys,
    pane: Pane,
    note: Option<(String, Instant)>,
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
        }
    }

    /// the search box or command line has the keyboard (text is typed, not performed)
    pub fn typing(&self) -> bool {
        self.pane.typing()
    }

    fn set_note(&mut self, text: impl Into<String>) {
        self.note = Some((text.into(), Instant::now() + NOTE_FOR));
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
            .or_else(|| s.call_error.as_ref().filter(|(_, t)| t.elapsed() < NOTE_FOR).map(|(e, _)| e.clone()));
        let note = self
            .note
            .as_ref()
            .filter(|(_, until)| Instant::now() < *until)
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
    fn friendly_errors() {
        let e = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.UnknownMethod").unwrap(),
            Some("Unknown method 'DjDecks'".into()),
            zbus::message::Message::method_call("/", "x").unwrap().build(&()).unwrap(),
        );
        assert_eq!(describe_error("DjDecks", &e), "codjd has no deck API (DjDecks): update codjd");
    }
}
