//! MCP server over stdio (`musa mcp [--live]`) so agents can read the library and drive musad.
//! Runs before any terminal setup; stdout carries only protocol, everything else goes to stderr.
//!
//! Without `--live`, tools that change what the audience hears (player deck, master, crossfader)
//! are refused. Queue edits, reads and deck 1 (the cue/preview deck) are always allowed.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use zbus::Connection;

use crate::{
    search,
    types::{Music, OuterMusic},
    ServerProxy,
};

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;
const DECK_CONTROLS: [&str; 4] = ["rate", "gain", "fader", "cue"];
const GLOBAL_CONTROLS: [&str; 3] = ["crossfader", "master", "cue_mix"];

// ---- pure helpers (unit tested) ----

/// What a state-changing tool does, as far as the live guardrail cares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect<'a> {
    /// reads, queue edits: always allowed
    Safe,
    /// plays/seeks/changes the player (deck 0) or master volume
    Player,
    /// `deck_*` call on a deck
    Deck(u32),
    /// `deck_set` with this control and deck
    DeckSet(&'a str, u32),
}

/// True when the call changes what the audience hears and needs `--live`.
pub fn needs_live(effect: Effect) -> bool {
    match effect {
        Effect::Safe => false,
        Effect::Player => true,
        Effect::Deck(deck) => deck == 0,
        Effect::DeckSet(control, deck) => match control {
            "master" | "crossfader" => true,
            // headphone mix only reaches the cue output
            "cue_mix" => false,
            _ => deck == 0,
        },
    }
}

pub fn refusal(tool: &str) -> String {
    format!(
        "{tool} refused: it changes what the audience hears and this MCP server is not live. \
         Restart it with `musa mcp --live` to allow it. Reads, queue edits and deck 1 (cue deck) \
         are always allowed."
    )
}

pub fn check_deck(deck: u32) -> Result<u32, String> {
    if deck > 1 {
        return Err(format!("deck must be 0 or 1, got {deck}"));
    }
    Ok(deck)
}

pub fn check_unit(value: f64, what: &str) -> Result<f64, String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!("{what} must be between 0 and 1, got {value}"));
    }
    Ok(value)
}

pub fn check_seconds(value: f64) -> Result<f64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("seconds must be a finite number >= 0, got {value}"));
    }
    Ok(value)
}

pub fn check_control(control: &str, value: f64) -> Result<(), String> {
    if !DECK_CONTROLS.contains(&control) && !GLOBAL_CONTROLS.contains(&control) {
        return Err(format!(
            "unknown control {control:?}; use rate, gain, fader, cue, crossfader, master or cue_mix"
        ));
    }
    if !value.is_finite() {
        return Err(format!("value must be a finite number, got {value}"));
    }
    Ok(())
}

pub fn clamp_limit(limit: Option<u32>) -> usize {
    (limit.map(|l| l as usize).unwrap_or(DEFAULT_LIMIT)).clamp(1, MAX_LIMIT)
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TrackRow {
    pub index: usize,
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub length_s: u64,
    pub path: String,
}

impl TrackRow {
    fn new(index: usize, title: &str, artist: &str, genre: &str, length: Duration, path: String) -> Self {
        TrackRow {
            index,
            title: title.to_string(),
            artist: artist.to_string(),
            genre: genre.to_string(),
            length_s: length.as_secs(),
            path,
        }
    }
}

fn music_row(m: &Music) -> TrackRow {
    let path: &std::path::Path = (&m.path).into();
    TrackRow::new(m.index, &m.title, &m.artist, &m.genre, m.length, path.to_string_lossy().into_owned())
}

fn outer_row(index: usize, m: &OuterMusic) -> TrackRow {
    let path: &std::path::Path = (&m.path).into();
    TrackRow::new(index, &m.title, &m.artist, &m.genre, m.length, path.to_string_lossy().into_owned())
}

/// The playlist as `Music` with its position as `index` (what `play_index`/`queue_add` take)
fn indexed(list: &[OuterMusic]) -> Vec<Music> {
    list.iter()
        .enumerate()
        .map(|(index, m)| Music {
            title: m.title.clone(),
            length: m.length,
            path: m.path.clone(),
            artist: m.artist.clone(),
            genre: m.genre.clone(),
            index,
        })
        .collect()
}

/// Library listing: filtered like the TUI search, capped at `limit`
pub fn library_json(list: &[Music], query: Option<&str>, limit: usize) -> Value {
    let matches = match query {
        Some(q) => search::search_musics(q, list),
        None => list.to_vec(),
    };
    let total = matches.len();
    let tracks: Vec<TrackRow> = matches.iter().take(limit).map(music_row).collect();
    json!({ "total": total, "returned": tracks.len(), "tracks": tracks })
}

pub fn features_json(path: &str, f: (f64, f64, bool)) -> Value {
    if !f.2 {
        return json!({ "path": path, "analyzed": false });
    }
    json!({
        "path": path,
        "analyzed": true,
        "bpm": (f.0 * 10.0).round() / 10.0,
        "bpm_note": "rough estimate, can be off by half or double",
        "loudness_db": (f.1 * 10.0).round() / 10.0,
    })
}

pub fn decks_json(decks: &[(f64, bool, f64, bool)]) -> Value {
    let decks: Vec<Value> = decks
        .iter()
        .enumerate()
        .map(|(deck, d)| {
            json!({
                "deck": deck,
                "position_s": d.0,
                "playing": d.1,
                "decoded_s": d.2,
                "decode_finished": d.3,
            })
        })
        .collect();
    json!({ "decks": decks })
}

pub fn health_json(h: (u64, u64, u64, u32)) -> Value {
    json!({
        "xruns": h.0,
        "second_output_underruns": h.1,
        "second_output_skips": h.2,
        "sample_rate": h.3,
    })
}

/// "2026-10-20T12:34:56Z" from unix seconds
pub fn timestamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // civil from days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn log_change(what: &str) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    eprintln!("[{}] musa mcp: {what}", timestamp(now));
}

fn to_text(v: Value) -> Result<String, String> {
    serde_json::to_string(&v).map_err(|e| e.to_string())
}

// ---- tool arguments ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LibraryArgs {
    /// Case-insensitive regex over title, artist or file name; `title:`, `artist:` and `genre:` prefixes restrict it to one field
    pub query: Option<String>,
    /// Max tracks to return (default 50, max 500)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathArgs {
    /// Absolute path of a track, as returned by `library`
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathCountArgs {
    /// Absolute path of the seed track
    pub path: String,
    /// How many tracks
    pub n: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IndexArgs {
    /// Track index as returned by `library`
    pub index: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SeekArgs {
    /// Seconds from the start of the track
    pub seconds: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VolumeArgs {
    /// 0.0 to 1.0
    pub volume: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueueAddArgs {
    /// Track index as returned by `library`
    pub index: u32,
    /// Put it at the front of the queue so it plays next (default: append)
    pub front: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueueRemoveArgs {
    /// Position in the queue as returned by `queue` (0 is next)
    pub position: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeckArgs {
    /// 0 (A, also the player) or 1 (B, cue/preview)
    pub deck: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeckLoadArgs {
    /// 0 (A, also the player) or 1 (B, cue/preview)
    pub deck: u32,
    /// Absolute path of the track
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeckSeekArgs {
    /// 0 (A, also the player) or 1 (B, cue/preview)
    pub deck: u32,
    /// Seconds from the start
    pub seconds: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeckSetArgs {
    /// rate, gain, fader, cue (per deck); crossfader, master, cue_mix (deck ignored)
    pub control: String,
    /// 0 or 1 (ignored by crossfader, master, cue_mix)
    pub deck: u32,
    /// New value of the control
    pub value: f64,
}

// ---- server ----

#[derive(Clone)]
pub struct Musa {
    live: bool,
    proxy: Arc<Mutex<Option<ServerProxy<'static>>>>,
    tool_router: ToolRouter<Self>,
}

const NOT_RUNNING: &str = "musad is not running (or not reachable on the session D-Bus). \
Start it, then retry; this server does not start it.";

fn dbus_err(e: zbus::Error) -> String {
    let text = e.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") || text.contains("not provided by any") {
        return NOT_RUNNING.to_string();
    }
    format!("musad call failed: {text}")
}

/// Runs a D-Bus call with a timeout
macro_rules! ask {
    ($self:ident, $p:ident => $call:expr) => {{
        let $p = $self.proxy().await?;
        match tokio::time::timeout(CALL_TIMEOUT, $call).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                $self.forget().await;
                return Err(dbus_err(e));
            }
            Err(_) => {
                $self.forget().await;
                return Err("musad did not answer in time".to_string());
            }
        }
    }};
}

impl Musa {
    pub fn new(live: bool) -> Self {
        Musa {
            live,
            proxy: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    /// Connected proxy; probes musad so a missing daemon gives one clear message
    async fn proxy(&self) -> Result<ServerProxy<'static>, String> {
        let mut slot = self.proxy.lock().await;
        if let Some(p) = slot.as_ref() {
            return Ok(p.clone());
        }
        let conn = Connection::session()
            .await
            .map_err(|e| format!("can't reach the session D-Bus: {e}"))?;
        let p = ServerProxy::new(&conn).await.map_err(dbus_err)?;
        match tokio::time::timeout(CALL_TIMEOUT, p.get_playing_index()).await {
            Ok(Ok(_)) => {}
            _ => return Err(NOT_RUNNING.to_string()),
        }
        *slot = Some(p.clone());
        Ok(p)
    }

    async fn forget(&self) {
        *self.proxy.lock().await = None;
    }

    /// Applies the live guardrail and logs the call; Err is the message for the agent
    fn gate(&self, tool: &str, effect: Effect, args: &str) -> Result<(), String> {
        if needs_live(effect) && !self.live {
            log_change(&format!("REFUSED {tool} {args} (not live)"));
            return Err(refusal(tool));
        }
        log_change(&format!("{tool} {args}"));
        Ok(())
    }

    async fn playlist(&self) -> Result<Vec<OuterMusic>, String> {
        Ok(ask!(self, p => p.playlist()).musics)
    }

    async fn check_index(&self, index: u32) -> Result<(), String> {
        let len = ask!(self, p => p.playlist()).musics.len();
        if index as usize >= len {
            return Err(format!("index {index} is out of range, the library has {len} tracks"));
        }
        Ok(())
    }
}

fn reply_ok(reply: String) -> Result<String, String> {
    if reply == "ok" {
        to_text(json!({ "ok": true }))
    } else {
        Err(reply)
    }
}

#[tool_router]
impl Musa {
    #[tool(description = "List library tracks (the current playlist). Optional regex query like the TUI search (title:/artist:/genre: prefixes). Default 50 results.")]
    async fn library(&self, Parameters(a): Parameters<LibraryArgs>) -> Result<String, String> {
        let list = indexed(&self.playlist().await?);
        to_text(library_json(&list, a.query.as_deref(), clamp_limit(a.limit)))
    }

    #[tool(description = "Player state: status, current track, position and length in seconds, volume (0..1), repeat mode, queue length.")]
    async fn now_playing(&self) -> Result<String, String> {
        let st = ask!(self, p => p.player_status());
        let pos = ask!(self, p => p.played_duration());
        let repeat = ask!(self, p => p.get_repeat());
        let queue = ask!(self, p => p.get_queue());
        let track = outer_row(st.index as usize, &st.music);
        let stopped = matches!(st.status, crate::types::PlayingStatus::Stopped);
        to_text(json!({
            "status": st.status,
            "track": if stopped { Value::Null } else { json!(track) },
            "position_s": pos,
            "length_s": if stopped { Value::Null } else { json!(st.music.length.as_secs()) },
            "volume": st.volume,
            "repeat": repeat,
            "queue_length": queue.len(),
        }))
    }

    #[tool(description = "The play queue, next track first. `position` is what queue_remove takes.")]
    async fn queue(&self) -> Result<String, String> {
        let q = ask!(self, p => p.get_queue());
        let rows: Vec<Value> = q
            .iter()
            .enumerate()
            .map(|(pos, m)| {
                let mut v = json!(outer_row(pos, m));
                v["position"] = v["index"].take();
                v.as_object_mut().map(|o| o.remove("index"));
                v
            })
            .collect();
        to_text(json!({ "length": rows.len(), "queue": rows }))
    }

    #[tool(description = "Analysis of one track: bpm (rough estimate, may be half or double), loudness in dB, analyzed flag.")]
    async fn track_features(&self, Parameters(a): Parameters<PathArgs>) -> Result<String, String> {
        let f = ask!(self, p => p.get_features(&a.path));
        to_text(features_json(&a.path, f))
    }

    #[tool(description = "The n tracks that sound most like the track at `path`, nearest first. Empty until `path` is analyzed.")]
    async fn similar(&self, Parameters(a): Parameters<PathCountArgs>) -> Result<String, String> {
        let n = a.n.clamp(1, MAX_LIMIT as u32);
        let idx = ask!(self, p => p.similar_to(&a.path, n));
        let list = indexed(&self.playlist().await?);
        let rows: Vec<TrackRow> = idx.iter().filter_map(|&i| list.get(i as usize)).map(music_row).collect();
        to_text(json!({ "returned": rows.len(), "tracks": rows }))
    }

    #[tool(description = "Background analysis progress: analyzed and queued file counts (equal when idle).")]
    async fn analysis_status(&self) -> Result<String, String> {
        let (done, queued) = ask!(self, p => p.analysis_status());
        to_text(json!({ "analyzed": done, "queued": queued, "idle": done == queued }))
    }

    #[tool(description = "LIVE: play the library track at `index` now on the player deck.")]
    async fn play_index(&self, Parameters(a): Parameters<IndexArgs>) -> Result<String, String> {
        self.gate("play_index", Effect::Player, &format!("index={}", a.index))?;
        self.check_index(a.index).await?;
        ask!(self, p => p.play_from_index(a.index));
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "LIVE: toggle play/pause on the player deck.")]
    async fn toggle(&self) -> Result<String, String> {
        self.gate("toggle", Effect::Player, "")?;
        ask!(self, p => p.toggle_play());
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "LIVE: skip to the next track.")]
    async fn next(&self) -> Result<String, String> {
        self.gate("next", Effect::Player, "")?;
        ask!(self, p => p.play_next());
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "LIVE: go to the previous track.")]
    async fn previous(&self) -> Result<String, String> {
        self.gate("previous", Effect::Player, "")?;
        ask!(self, p => p.play_previous());
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "LIVE: seek the playing track to `seconds` from its start.")]
    async fn seek(&self, Parameters(a): Parameters<SeekArgs>) -> Result<String, String> {
        let s = check_seconds(a.seconds)?;
        self.gate("seek", Effect::Player, &format!("seconds={s}"))?;
        ask!(self, p => p.seek(s));
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "LIVE: set the player volume, 0.0 to 1.0.")]
    async fn set_volume(&self, Parameters(a): Parameters<VolumeArgs>) -> Result<String, String> {
        let v = check_unit(a.volume, "volume")?;
        self.gate("set_volume", Effect::Player, &format!("volume={v}"))?;
        ask!(self, p => p.volume(v as f32));
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "Add the library track at `index` to the queue (end, or front to play next).")]
    async fn queue_add(&self, Parameters(a): Parameters<QueueAddArgs>) -> Result<String, String> {
        let front = a.front.unwrap_or(false);
        self.gate("queue_add", Effect::Safe, &format!("index={} front={front}", a.index))?;
        self.check_index(a.index).await?;
        ask!(self, p => p.queue_add(a.index, front));
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "Remove the queued track at `position` (see `queue`).")]
    async fn queue_remove(&self, Parameters(a): Parameters<QueueRemoveArgs>) -> Result<String, String> {
        self.gate("queue_remove", Effect::Safe, &format!("position={}", a.position))?;
        let len = ask!(self, p => p.get_queue()).len();
        if a.position as usize >= len {
            return Err(format!("position {} is out of range, the queue has {len} tracks", a.position));
        }
        ask!(self, p => p.queue_remove(a.position));
        to_text(json!({ "ok": true }))
    }

    #[tool(description = "Queue the n tracks most like the track at `path` (skips queued ones and duplicate songs). Returns how many were added.")]
    async fn queue_similar(&self, Parameters(a): Parameters<PathCountArgs>) -> Result<String, String> {
        let n = a.n.clamp(1, MAX_LIMIT as u32);
        self.gate("queue_similar", Effect::Safe, &format!("path={} n={n}", a.path))?;
        let added = ask!(self, p => p.queue_similar(&a.path, n));
        to_text(json!({ "added": added }))
    }

    #[tool(description = "Per deck (0 = A/player, 1 = B/cue): position, playing, decoded seconds, decode finished.")]
    async fn deck_state(&self) -> Result<String, String> {
        let d = ask!(self, p => p.dj_state());
        to_text(decks_json(&d))
    }

    #[tool(description = "Load a track on a deck, paused at the start. Deck 0 is LIVE (the player deck); deck 1 is the cue deck.")]
    async fn deck_load(&self, Parameters(a): Parameters<DeckLoadArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        self.gate("deck_load", Effect::Deck(deck), &format!("deck={deck} path={}", a.path))?;
        reply_ok(ask!(self, p => p.dj_load(deck, &a.path)))
    }

    #[tool(description = "Start a deck. Deck 0 is LIVE (the player deck); deck 1 is the cue deck.")]
    async fn deck_play(&self, Parameters(a): Parameters<DeckArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        self.gate("deck_play", Effect::Deck(deck), &format!("deck={deck}"))?;
        reply_ok(ask!(self, p => p.dj_play(deck)))
    }

    #[tool(description = "Pause a deck. Deck 0 is LIVE (the player deck); deck 1 is the cue deck.")]
    async fn deck_pause(&self, Parameters(a): Parameters<DeckArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        self.gate("deck_pause", Effect::Deck(deck), &format!("deck={deck}"))?;
        reply_ok(ask!(self, p => p.dj_pause(deck)))
    }

    #[tool(description = "Seek a deck to `seconds`. Deck 0 is LIVE (the player deck); deck 1 is the cue deck.")]
    async fn deck_seek(&self, Parameters(a): Parameters<DeckSeekArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        let s = check_seconds(a.seconds)?;
        self.gate("deck_seek", Effect::Deck(deck), &format!("deck={deck} seconds={s}"))?;
        reply_ok(ask!(self, p => p.dj_seek(deck, s)))
    }

    #[tool(description = "Set a mixer control: rate, gain, fader, cue (per deck), crossfader, master, cue_mix (deck ignored). Deck 0 controls, master and crossfader are LIVE; deck 1 and cue_mix are always allowed.")]
    async fn deck_set(&self, Parameters(a): Parameters<DeckSetArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        check_control(&a.control, a.value)?;
        self.gate(
            "deck_set",
            Effect::DeckSet(&a.control, deck),
            &format!("control={} deck={deck} value={}", a.control, a.value),
        )?;
        reply_ok(ask!(self, p => p.dj_set(&a.control, deck, a.value)))
    }

    #[tool(description = "Audio engine health: xruns, second-output underruns and skips, sample rate.")]
    async fn audio_health(&self) -> Result<String, String> {
        let h = ask!(self, p => p.audio_health());
        to_text(health_json(h))
    }

    #[tool(description = "Names of the audio output devices.")]
    async fn audio_devices(&self) -> Result<String, String> {
        let d = ask!(self, p => p.audio_devices());
        to_text(json!({ "devices": d }))
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "musa",
    instructions = "Control musa/musad. `library` lists tracks; their `index` is what play_index and queue_add take. Tools marked LIVE are refused unless the server was started with `musa mcp --live`."
)]
impl ServerHandler for Musa {}

/// `musa mcp [--live]`; returns the exit code
pub async fn run(args: &[String]) -> i32 {
    let mut live = false;
    for a in args {
        match a.as_str() {
            "--live" => live = true,
            other => {
                eprintln!("usage: musa mcp [--live]\nunknown option {other}");
                return 2;
            }
        }
    }
    eprintln!(
        "musa mcp: serving on stdio ({})",
        if live { "LIVE: player and master controls allowed" } else { "safe mode, restart with --live to allow player controls" }
    );
    let service = match Musa::new(live).serve(rmcp::transport::stdio()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("musa mcp: {e}");
            return 1;
        }
    };
    match service.waiting().await {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("musa mcp: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music(index: usize, title: &str, artist: &str, secs: u64) -> Music {
        Music {
            title: title.into(),
            artist: artist.into(),
            genre: "techno".into(),
            length: Duration::from_secs(secs),
            index,
            ..Music::default()
        }
    }

    #[test]
    fn guardrail_decisions() {
        use Effect::*;
        assert!(!needs_live(Safe));
        assert!(needs_live(Player));
        assert!(needs_live(Deck(0)) && !needs_live(Deck(1)));
        assert!(needs_live(DeckSet("master", 1)));
        assert!(needs_live(DeckSet("crossfader", 1)));
        assert!(needs_live(DeckSet("gain", 0)) && needs_live(DeckSet("rate", 0)));
        assert!(!needs_live(DeckSet("gain", 1)) && !needs_live(DeckSet("cue", 1)));
        assert!(!needs_live(DeckSet("cue_mix", 0)));
        assert!(refusal("toggle").contains("--live"));
    }

    #[test]
    fn argument_validation() {
        assert!(check_deck(1).is_ok() && check_deck(2).is_err());
        assert!(check_unit(0.5, "volume").is_ok());
        assert!(check_unit(1.5, "volume").is_err() && check_unit(-0.1, "volume").is_err());
        assert!(check_unit(f64::NAN, "volume").is_err());
        assert!(check_seconds(0.0).is_ok() && check_seconds(-1.0).is_err());
        assert!(check_seconds(f64::INFINITY).is_err());
        assert!(check_control("rate", 1.0).is_ok() && check_control("master", 0.0).is_ok());
        assert!(check_control("bogus", 1.0).is_err());
        assert!(check_control("gain", f64::NAN).is_err());
        assert_eq!(clamp_limit(None), 50);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(100000)), MAX_LIMIT);
    }

    #[test]
    fn library_filters_and_caps() {
        let list = vec![
            music(0, "Aftertekno", "Pneumatix", 437),
            music(1, "Other", "Someone", 100),
            music(2, "After Hours", "Pneumatix", 200),
        ];
        let v = library_json(&list, None, 2);
        assert_eq!(v["total"], 3);
        assert_eq!(v["returned"], 2);
        let v = library_json(&list, Some("^after"), 50);
        assert_eq!(v["total"], 2);
        assert_eq!(v["tracks"][0]["index"], 0);
        assert_eq!(v["tracks"][0]["length_s"], 437);
        assert_eq!(v["tracks"][1]["title"], "After Hours");
        let v = library_json(&list, Some("artist:someone"), 50);
        assert_eq!(v["total"], 1);
        assert_eq!(v["tracks"][0]["index"], 1);
        assert!(v["tracks"][0]["path"].is_string());
    }

    #[test]
    fn json_shaping() {
        let v = features_json("/m/a.flac", (127.96, -8.04, true));
        assert_eq!(v["analyzed"], true);
        assert_eq!(v["bpm"], 128.0);
        assert_eq!(v["loudness_db"], -8.0);
        assert_eq!(features_json("/m/a.flac", (0., 0., false))["analyzed"], false);
        let v = decks_json(&[(1.5, true, 30.0, false), (0.0, false, 0.0, true)]);
        assert_eq!(v["decks"][1]["deck"], 1);
        assert_eq!(v["decks"][0]["playing"], true);
        assert_eq!(health_json((1, 2, 3, 48000))["sample_rate"], 48000);
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(timestamp(1_782_000_000 + 86400 * 60), timestamp(1_782_000_000 + 86400 * 60));
    }

    #[test]
    fn dbus_errors_are_clear() {
        let e = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.ServiceUnknown").unwrap(),
            None,
            zbus::message::Message::method_call("/a", "B").unwrap().build(&()).unwrap(),
        );
        assert!(dbus_err(e).contains("not running"));
    }

    #[tokio::test]
    async fn live_refusal_happens_before_dbus() {
        // no musad needed: the guardrail answers first
        let s = Musa::new(false);
        let err = s.toggle().await.unwrap_err();
        assert!(err.contains("--live"));
        let err = s.set_volume(Parameters(VolumeArgs { volume: 2.0 })).await.unwrap_err();
        assert!(err.contains("between 0 and 1"));
    }
}
