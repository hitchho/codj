//! MCP server over stdio (`codj mcp [--live]`) so agents can read the library and drive codjd.
//! Runs before any terminal setup; stdout carries only protocol, everything else goes to stderr.
//!
//! Without `--live`, tools that change what the audience hears (player deck, master, crossfader)
//! are refused. Queue edits, reads and deck 1 (the cue/preview deck) are always allowed.
//! Set tools: reading, checking and writing set files, capturing cues and recording never change
//! what is playing and are always allowed; arming, previewing, loading and auto mode need `--live`.

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
    /// the set runtime starts or stops audio: arm, disarm, auto, preview, load
    SetRuntime,
    /// loading, unloading or sending a patch slot live: changes what the audience hears
    Patch,
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
        Effect::SetRuntime | Effect::Patch => true,
    }
}

pub fn refusal(tool: &str) -> String {
    format!(
        "{tool} refused: it changes what the audience hears and this MCP server is not live. \
         Restart it with `codj mcp --live` to allow it. Reads, queue edits, deck 1 (cue deck) \
         set editing (sources, writes, cues, recording) and patch files and edits (list, source, check, write, \
         import, state, node types, action) are always allowed."
    )
}

/// A patch slot, 0-based: P1 is 0
pub fn check_slot(slot: u32) -> Result<u32, String> {
    if slot as usize >= crate::dj::patchdef::SLOTS {
        return Err(format!("slot must be 0..3 (P1..P4), got {slot}"));
    }
    Ok(slot)
}

/// Ops of `PatchAction` (the contract's action list)
pub const PATCH_OPS: [&str; 10] = ["add_node", "remove_node", "connect", "disconnect", "set_input", "set_param", "knob", "cell", "pattern", "rename"];

/// The action as the JSON text codjd wants: an object with a known `op` (a JSON string holding one
/// is accepted too, models like to double-encode)
pub fn check_patch_action(action: &Value) -> Result<String, String> {
    let owned;
    let obj = match action {
        Value::String(text) => {
            owned = serde_json::from_str::<Value>(text).map_err(|e| format!("action is a string that is not JSON: {e}"))?;
            &owned
        }
        other => other,
    };
    let Some(op) = obj.get("op").and_then(Value::as_str) else {
        return Err(format!("action needs an \"op\": one of {}", PATCH_OPS.join(", ")));
    };
    if !PATCH_OPS.contains(&op) {
        return Err(format!("unknown op {op:?}; use one of {}", PATCH_OPS.join(", ")));
    }
    Ok(obj.to_string())
}

/// Largest patch file `patch_write` / `patch_check` take
pub const MAX_PATCH_BYTES: usize = 1 << 20;

/// A patch file's text: JSON, not absurdly large
pub fn check_patch_json(text: &str) -> Result<(), String> {
    if text.len() > MAX_PATCH_BYTES {
        return Err(format!("the patch is {} bytes, the limit is {MAX_PATCH_BYTES}", text.len()));
    }
    serde_json::from_str::<Value>(text).map(|_| ()).map_err(|e| format!("json is not valid JSON: {e}"))
}

/// A patch name: letters, digits, - and _
pub fn check_patch_name(name: &str) -> Result<(), String> {
    if crate::dj::patchdef::valid_name(name) {
        Ok(())
    } else {
        Err(format!("bad patch name {name:?}: letters, digits, - and _ only (at most 64)"))
    }
}

/// `PatchState` as JSON for agents; `queued` is null when no pattern is queued
pub fn patch_state_json(slots: &[crate::dj_types::PatchSlot]) -> Value {
    let rows: Vec<Value> = slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if !s.loaded {
                return json!({ "slot": i, "label": format!("P{}", i + 1), "loaded": false });
            }
            json!({
                "slot": i,
                "label": format!("P{}", i + 1),
                "loaded": true,
                "name": s.name,
                "title": s.title,
                "unsaved_edits": s.dirty,
                "live": s.live,
                "cpu": s.cpu,
                "muted_reason": s.muted_reason,
                "knobs": s.knobs.iter().map(|k| json!({ "name": k.0, "value": k.1, "min": k.2, "max": k.3 })).collect::<Vec<_>>(),
                "sequencers": s.sequencers.iter().map(|q| json!({
                    "name": q.0,
                    "pattern": q.1,
                    "pattern_letter": crate::dj::patch::pattern_letter(q.1 as usize).to_string(),
                    "queued": (q.2 != crate::dj::patch::NONE_QUEUED).then_some(q.2),
                    "step": q.3,
                })).collect::<Vec<_>>(),
                "fader": s.fader,
                "headphones": s.headphones,
            })
        })
        .collect();
    json!({ "slots": rows })
}

/// Result of `patch_write`, like [`write_result`] but pointing at the patch tools
pub fn patch_write_result(reply: &str) -> Result<Value, String> {
    match reply.strip_prefix("ok") {
        Some(rest) if rest.is_empty() || rest.starts_with(' ') => {
            let hash = rest.trim();
            Ok(if hash.is_empty() { json!({ "ok": true }) } else { json!({ "ok": true, "hash": hash }) })
        }
        _ if reply.contains("changed") => Err(format!(
            "{reply}. The patch was edited since you read it (vim, codj or another agent). Call patch_source again, \
             apply your change to that JSON and call patch_write with the new hash."
        )),
        _ => Err(reply.to_string()),
    }
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

/// `agent:<name>` for the set history, from the MCP client's name. Only `[A-Za-z0-9._-]` survive
/// (anything else, spaces and colons included, becomes `-`), at most 40 characters; the name ends
/// up in a git commit message, so no control characters or line breaks can pass.
pub fn agent_author(client: Option<&str>) -> String {
    let mut name = String::new();
    for c in client.unwrap_or("").chars() {
        let c = if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '-' };
        if c == '-' && (name.is_empty() || name.ends_with('-')) {
            continue;
        }
        name.push(c);
    }
    let name: String = name.trim_end_matches('-').chars().take(40).collect();
    if name.is_empty() {
        "agent:unknown".to_string()
    } else {
        format!("agent:{name}")
    }
}

/// Reply of a set call: `ok` or `ok <detail>` is success, anything else the reason
pub fn reply_detail(reply: &str) -> Result<Value, String> {
    if reply == "ok" {
        Ok(json!({ "ok": true }))
    } else if let Some(detail) = reply.strip_prefix("ok ") {
        Ok(json!({ "ok": true, "detail": detail.trim() }))
    } else {
        Err(reply.to_string())
    }
}

/// Result of `set_write`: `ok <new hash>` becomes `{ok, hash}`; a refusal because the file changed
/// tells the agent what to do next
pub fn write_result(reply: &str) -> Result<Value, String> {
    match reply.strip_prefix("ok") {
        Some(rest) if rest.is_empty() || rest.starts_with(' ') => {
            let hash = rest.trim();
            Ok(if hash.is_empty() { json!({ "ok": true }) } else { json!({ "ok": true, "hash": hash }) })
        }
        _ if reply.contains("changed") => Err(format!(
            "{reply}. The set was edited since you read it (vim, codj or another agent). Call set_source again, \
             apply your change to that text and call set_write with the new hash."
        )),
        _ => Err(reply.to_string()),
    }
}

/// Validates a cue or loop name (a set-language identifier)
pub fn check_name(name: &str) -> Result<(), String> {
    if crate::dj::capture::valid_name(name) {
        Ok(())
    } else {
        Err(format!("bad name {name:?}: letters, digits, _ and - only, starting with a letter or _"))
    }
}

/// The set language in one screen, for `set_language_help`
pub const SET_LANGUAGE_HELP: &str = r#"codj set language (.codj files). One statement per line, '#' starts a comment, indentation is only for reading. Everything is counted in bars and beats, never seconds.
Identifiers: [A-Za-z_][A-Za-z0-9_-]*. Strings: "..." with " and \.

  set "kaiku friday"                      optional display name
  track aft "Pneumatix - Aftertekno.flac" id, then a path relative to the library, an absolute path, or a search ("artist - title")
    grid 112 @ 0.012s beat1 1             optional: bpm, first beat time, which beat of the bar it is; or x2 /2 x1.5 /1.5 to scale the detected tempo
    gain -1.5dB                           optional trim
    cue drop  bar 33                      named position; "bar 1 beat 1" is the first downbeat; beat may be fractional
    loop break bar 97 len 8 bars          named loop (len k beats|bars)
  mix aft -> smk at aft.outro over 32 bars    outgoing -> incoming, starts at a cue or "bar N [beat M]" of the outgoing track, runs N bars
    smk play from intro                   incoming starts at mix bar 0 from a cue or bar (add "unsynced" to opt out of tempo sync)
    bars 0..16   smk.low kill; smk.fader 0 -> 1     ramp over mix bars 0..16; items separated by ';'
    bar 16       swap low                 at mix bar 16: outgoing low killed, incoming low back to 0 in the same frame
    bars 16..32  aft.filter 0 -> -0.6 smooth; aft.fader 1 -> 0
    bar 32       aft stop

Inside a mix, bar numbers count from the mix start on the OUTGOING track's grid (it is the master); they may be fractional (bar 15.5).
Items: <t>.<control> <from> -> <to> [linear|smooth|exp] (ramp, only in a "bars" line; <from> may be _ = from wherever it is) | <t>.<control> = <v> |
<t>.<band> kill | swap <band> | <t> stop | <t> loop <name>|<k> beats|off | <t> jump <cue>|<+-k> beats | xfade <from> -> <to> | xfade = <v>.
Controls: fader 0..1, low/mid/high -1 (kill)..0 (unity)..+1, filter -1 (low-pass)..0 (off)..+1 (high-pass), gain in dB. <t> is a track id.
Tracks without a beat grid can be in a set but a mix touching them is an error ("analyze it or add a grid line").
Errors read file:line:col: message. A file with errors is not loaded; the last good version stays active. Reserved words (set track mix bar bars swap) can't be track ids.
Prefer replacing one track or mix block over rewriting the file, so diffs stay small."#;

fn log_change(what: &str) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    eprintln!("[{}] codj mcp: {what}", timestamp(now));
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetNameArgs {
    /// Set name, file name or stem, as listed by `sets_list`
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetCheckArgs {
    /// The complete text of the set file to validate (nothing is saved)
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetWriteArgs {
    /// Set to write: its name, file name or stem from `sets_list`; a new name creates a new file
    pub name: String,
    /// The complete new text of the file
    pub text: String,
    /// The `hash` that `set_source` returned for the version this text is based on ("" only when creating a new file)
    pub base_hash: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetCaptureArgs {
    /// 0 (A) or 1 (B)
    pub deck: u32,
    /// Cue or loop name: letters, digits, _ and -, starting with a letter or _. An existing name is replaced
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetGridArgs {
    /// 0 (A) or 1 (B)
    pub deck: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetRecordArgs {
    /// true starts recording a mix, false stops it and writes the `mix` block into the active set
    pub on: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetMixArgs {
    /// Mix number, 0-based in file order (the first `mix` block is 0)
    pub mix: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetPreviewArgs {
    /// Mix number, 0-based in file order
    pub mix: u32,
    /// Bars before the mix start to roll in (default 8)
    pub bars_before: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetTrackArgs {
    /// Track number, 0-based in file order (the first `track` line is 0)
    pub track: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetAutoArgs {
    /// true: after a mix ends, arm the next one whose outgoing track is on air
    pub on: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchNameArgs {
    /// Patch name as listed by `patch_list` (the file name without .json)
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchCheckArgs {
    /// The complete JSON text of the patch file to validate (nothing is saved)
    pub json: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchWriteArgs {
    /// Patch to write: letters, digits, - and _; a new name creates a new file
    pub name: String,
    /// The complete new JSON text of the patch file
    pub json: String,
    /// The `hash` that `patch_source` returned for the version this JSON is based on ("" only when creating a new patch)
    pub base_hash: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchImportArgs {
    /// Absolute path of a NoiseCraft .ncft file
    pub path: String,
    /// Name of the new patch: letters, digits, - and _
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchLoadArgs {
    /// Slot 0..3 (P1..P4)
    pub slot: u32,
    /// Patch name from `patch_list`
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchSlotArgs {
    /// Slot 0..3 (P1..P4)
    pub slot: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchLiveArgs {
    /// Slot 0..3 (P1..P4)
    pub slot: u32,
    /// true sends the slot to the master (the audience hears it), false back to headphones only
    pub live: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchActionArgs {
    /// Slot 0..3 (P1..P4) holding the patch to edit
    pub slot: u32,
    /// One edit as an object: {"op":"knob","name":"cutoff","value":0.7}, {"op":"cell","seq":"drums","pattern":0,"step":4,"row":1,"on":true,"velocity":0.8}, {"op":"pattern","seq":"drums","pattern":1,"now":false}, {"op":"set_param","id":"2","param":"scale","value":"D dorian"}, {"op":"set_input","to":["6","reso"],"value":0.4}, {"op":"connect","from":["9","out"],"to":["6","cutoff"]}, {"op":"disconnect","to":["6","cutoff"]}, {"op":"add_node","id":"9","type":"Sine","name":"lfo","params":{}}, {"op":"remove_node","id":"9"}, {"op":"rename","id":"3","name":"bass"}
    pub action: Value,
}

// ---- server ----

#[derive(Clone)]
pub struct Codj {
    live: bool,
    /// `clientInfo.name` of the MCP client, from `initialize`
    client: Arc<std::sync::Mutex<Option<String>>>,
    proxy: Arc<Mutex<Option<ServerProxy<'static>>>>,
    tool_router: ToolRouter<Self>,
}

const NOT_RUNNING: &str = "codjd is not running (or not reachable on the session D-Bus). \
Start it, then retry; this server does not start it.";

fn dbus_err(e: zbus::Error) -> String {
    let text = e.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") || text.contains("not provided by any") {
        return NOT_RUNNING.to_string();
    }
    format!("codjd call failed: {text}")
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
                return Err("codjd did not answer in time".to_string());
            }
        }
    }};
}

impl Codj {
    pub fn new(live: bool) -> Self {
        Codj {
            live,
            client: Arc::new(std::sync::Mutex::new(None)),
            proxy: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    /// Connected proxy; probes codjd so a missing daemon gives one clear message
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

    /// who the set history names for this client's writes
    fn author(&self) -> String {
        agent_author(self.client.lock().unwrap().as_deref())
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

/// `SetState` as JSON for agents
pub fn set_state_json(s: &crate::dj_types::SetState) -> Value {
    json!({
        "name": s.name,
        "file": s.file,
        "errors": s.errors,
        "tracks": s.tracks.iter().enumerate().map(|(i, t)| json!({ "index": i, "id": t.0, "title": t.1, "path": t.2 })).collect::<Vec<_>>(),
        "mixes": s.mixes.iter().enumerate().map(|(i, m)| json!({ "index": i, "from": m.0, "to": m.1, "start_beat": m.2, "length_beats": m.3 })).collect::<Vec<_>>(),
        "armed": s.armed_mix(),
        "running": s.running,
        "out_deck": s.out_deck,
        "mix_beat": s.mix_beat,
        "auto": s.auto,
        "next_items": s.next_items.iter().map(|(b, t)| json!({ "mix_beat": b, "text": t })).collect::<Vec<_>>(),
    })
}

fn reply_ok(reply: String) -> Result<String, String> {
    if reply == "ok" {
        to_text(json!({ "ok": true }))
    } else {
        Err(reply)
    }
}

#[tool_router]
impl Codj {
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

    #[tool(description = "List the set files (.codj) in the set folder: name, file and error count. Start here, then read one with set_source. Sets are text files describing tracks, cues, loops and the mixes between them in bars and beats; set_language_help has the syntax.")]
    async fn sets_list(&self) -> Result<String, String> {
        let sets = ask!(self, p => p.set_list());
        let rows: Vec<Value> = sets.iter().map(|(name, file, errors)| json!({ "name": name, "file": file, "errors": errors })).collect();
        to_text(json!({ "sets": rows }))
    }

    #[tool(description = "State of the set runtime: the active set, its tracks (id, title, path) and mixes (from, to, start beat, length in beats) in file order (indexes are 0-based and are what set_arm, set_preview and set_load take), the armed mix, whether it runs, auto mode and upcoming automation.")]
    async fn set_state(&self) -> Result<String, String> {
        let s = ask!(self, p => p.set_state());
        to_text(set_state_json(&s))
    }

    #[tool(description = "Read a set file as it is on disk: returns {text, hash}. WORKFLOW for changing a set: set_source -> edit the text (prefer replacing one track or mix block, keep comments and formatting) -> set_check -> set_write with base_hash = this hash. If set_write says the set changed, read it again with set_source, redo your edit on the new text, and write with the new hash. Keep the hash: it is what protects edits made meanwhile in vim, in codj or by another agent.")]
    async fn set_source(&self, Parameters(a): Parameters<SetNameArgs>) -> Result<String, String> {
        let (text, hash) = ask!(self, p => p.set_source(&a.name));
        to_text(json!({ "name": a.name, "text": text, "hash": hash }))
    }

    #[tool(description = "Validate set text exactly as if it were saved (parse, library search, grids, compile) without saving: returns {valid, errors} with every error as `file:line:col: message`. Call it after every edit and fix all errors before set_write (which refuses text with errors). Language: `track <id> \"<file or search>\"` with indented `grid`/`gain`/`cue <name> bar N [beat M]`/`loop <name> bar N len K bars` lines; `mix <a> -> <b> at <a>.<cue>|bar N over K bars` with `<b> play from <cue>`, `bar N <item>; ...` and `bars I..J <t>.<control> <from> -> <to> [linear|smooth|exp]` lines (controls fader low mid high filter gain; `<t>.<band> kill`, `swap <band>`, `<t> stop`). Bar numbers inside a mix count from its start on the outgoing track's grid. Full syntax: set_language_help.")]
    async fn set_check(&self, Parameters(a): Parameters<SetCheckArgs>) -> Result<String, String> {
        let errors = ask!(self, p => p.set_check(&a.text));
        to_text(json!({ "valid": errors.is_empty(), "errors": errors }))
    }

    #[tool(description = "Save a set file after checking it. Needs `base_hash`, the hash from the set_source you based the text on; if the file changed since (vim, codj, another agent) it is refused with \"set changed since you read it\" and you must set_source again, redo the edit on the new text and retry. Text with errors is refused too (run set_check first). A new `name` creates a new file (base_hash \"\"). The write is attributed to you in the set's history. Writing never changes what is playing: a running mix keeps its plan, the edit applies to the next arm. Set language in short: `track <id> \"<file>\"` + `cue <name> bar N`, `loop <name> bar N len K bars`, `grid <bpm>`; `mix <a> -> <b> at <a>.<cue> over K bars` + `<b> play from <cue>`, `bars I..J <t>.<control> <from> -> <to>`, `bar N swap low`, `<t> stop` (see set_language_help).")]
    async fn set_write(&self, Parameters(a): Parameters<SetWriteArgs>) -> Result<String, String> {
        let author = self.author();
        self.gate("set_write", Effect::Safe, &format!("name={} base_hash={} author={author} bytes={}", a.name, a.base_hash, a.text.len()))?;
        let reply = ask!(self, p => p.set_write(&a.name, &a.text, &a.base_hash, &author));
        to_text(write_result(&reply)?)
    }

    #[tool(description = "A cheat sheet of the set language (.codj files): tracks, cues, loops, grids, mixes, items, controls and the rules for bar numbers. Read it before writing a set.")]
    async fn set_language_help(&self) -> Result<String, String> {
        Ok(SET_LANGUAGE_HELP.to_string())
    }

    #[tool(description = "Make a set active (by name, file name or stem; \"\" = none). Cue, loop, grid and record tools and the set_arm family work on the active set. Does not change what is playing.")]
    async fn set_use(&self, Parameters(a): Parameters<SetNameArgs>) -> Result<String, String> {
        self.gate("set_use", Effect::Safe, &format!("name={}", a.name))?;
        to_text(reply_detail(&ask!(self, p => p.set_use(&a.name)))?)
    }

    #[tool(description = "Capture the deck's current position as cue `name` in the active set (a `cue <name> bar N [beat M]` line in that track's block; an existing cue of that name is replaced; a track not yet in the set is added). Deck 0 = A, 1 = B.")]
    async fn set_cue(&self, Parameters(a): Parameters<SetCaptureArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        check_name(&a.name)?;
        self.gate("set_cue", Effect::Safe, &format!("deck={deck} name={}", a.name))?;
        to_text(reply_detail(&ask!(self, p => p.set_cue(deck, &a.name)))?)
    }

    #[tool(description = "Capture the deck's active loop as loop `name` in the active set (a `loop <name> bar N len K beats|bars` line). Set the loop on the deck first. Deck 0 = A, 1 = B.")]
    async fn set_loop_capture(&self, Parameters(a): Parameters<SetCaptureArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        check_name(&a.name)?;
        self.gate("set_loop_capture", Effect::Safe, &format!("deck={deck} name={}", a.name))?;
        to_text(reply_detail(&ask!(self, p => p.set_loop_capture(deck, &a.name)))?)
    }

    #[tool(description = "Write the deck's current beat grid (after any hand correction) into the active set as a `grid` line of that track. Deck 0 = A, 1 = B.")]
    async fn set_grid_capture(&self, Parameters(a): Parameters<SetGridArgs>) -> Result<String, String> {
        let deck = check_deck(a.deck)?;
        self.gate("set_grid_capture", Effect::Safe, &format!("deck={deck}"))?;
        to_text(reply_detail(&ask!(self, p => p.set_grid_capture(deck)))?)
    }

    #[tool(description = "Start (on=true) or stop (on=false) recording a mix into the active set: while recording, every control change on either deck is logged; stopping writes it as a `mix` block (replacing an existing mix of the same two tracks). Returns the summary, e.g. \"mix 2: aft -> smk, 16 bars, 9 items\". Recording only observes; it does not play anything.")]
    async fn set_record(&self, Parameters(a): Parameters<SetRecordArgs>) -> Result<String, String> {
        self.gate("set_record", Effect::Safe, &format!("on={}", a.on))?;
        to_text(reply_detail(&ask!(self, p => p.set_record(a.on)))?)
    }

    #[tool(description = "LIVE: arm mix `mix` (0-based, file order) of the active set: loads the incoming track on the other deck and schedules the plan; it then runs by itself when the outgoing deck reaches the mix start. The outgoing track must be on a deck.")]
    async fn set_arm(&self, Parameters(a): Parameters<SetMixArgs>) -> Result<String, String> {
        self.gate("set_arm", Effect::SetRuntime, &format!("mix={}", a.mix))?;
        reply_ok(ask!(self, p => p.set_arm(a.mix)))
    }

    #[tool(description = "LIVE: cancel the armed or running mix's remaining automation (controls stay where they are).")]
    async fn set_disarm(&self) -> Result<String, String> {
        self.gate("set_disarm", Effect::SetRuntime, "")?;
        reply_ok(ask!(self, p => p.set_disarm()))
    }

    #[tool(description = "LIVE: auto mode on/off: after a mix ends, arm the next one whose outgoing track is now on air.")]
    async fn set_auto(&self, Parameters(a): Parameters<SetAutoArgs>) -> Result<String, String> {
        self.gate("set_auto", Effect::SetRuntime, &format!("on={}", a.on))?;
        reply_ok(ask!(self, p => p.set_auto(a.on)))
    }

    #[tool(description = "LIVE: audition a mix: loads both tracks, puts the outgoing deck `bars_before` bars (default 8) before the mix start, arms and plays.")]
    async fn set_preview(&self, Parameters(a): Parameters<SetPreviewArgs>) -> Result<String, String> {
        let bars = a.bars_before.unwrap_or(8.0);
        if !(bars.is_finite() && bars > 0.0) {
            return Err("bars_before must be a positive number".to_string());
        }
        self.gate("set_preview", Effect::SetRuntime, &format!("mix={} bars_before={bars}", a.mix))?;
        reply_ok(ask!(self, p => p.set_preview(a.mix, bars)))
    }

    #[tool(description = "LIVE: load track `track` (0-based, file order) of the active set onto the free deck, with the set's grid and gain overrides.")]
    async fn set_load(&self, Parameters(a): Parameters<SetTrackArgs>) -> Result<String, String> {
        self.gate("set_load", Effect::SetRuntime, &format!("track={}", a.track))?;
        reply_ok(ask!(self, p => p.set_load(a.track)))
    }

    #[tool(description = "List the patch files (synth / drum machine definitions, JSON) in the patches folder: name and title. Patch decks are instruments built from a graph of nodes that play in time with the decks. WORKFLOW to make one: patch_node_types (what nodes exist) -> write the JSON -> patch_check -> patch_write -> (live only) patch_load -> patch_action for knobs and sequencer cells -> patch_live. Read an existing one with patch_source.")]
    async fn patch_list(&self) -> Result<String, String> {
        let files = ask!(self, p => p.patch_list());
        let rows: Vec<Value> = files.iter().map(|(name, title)| json!({ "name": name, "title": title })).collect();
        to_text(json!({ "patches": rows }))
    }

    #[tool(description = "Read a patch file as it is on disk: returns {name, json, hash}. To change a patch: patch_source -> edit the JSON (keep node ids and names stable so a loaded patch keeps its state) -> patch_check -> patch_write with base_hash = this hash. If patch_write says the patch changed, read it again and redo your edit. Keep the hash: it protects edits made meanwhile in vim, in codj or by another agent.")]
    async fn patch_source(&self, Parameters(a): Parameters<PatchNameArgs>) -> Result<String, String> {
        check_patch_name(&a.name)?;
        let (text, hash) = ask!(self, p => p.patch_source(&a.name));
        to_text(json!({ "name": a.name, "json": text, "hash": hash }))
    }

    #[tool(description = "Validate patch JSON exactly as if it were saved, without saving: returns {valid, errors}, every error as `<node id or name>: message` (or `patch: message` for the whole file). Call it after every edit and fix all errors before patch_write. Rules: exactly one AudioOut; every `ins` entry is [source node id, output name] or a number; Knob nodes need a unique name; a feedback loop must pass through a Delay or Hold. patch_node_types lists the node types with their inputs, outputs and params.")]
    async fn patch_check(&self, Parameters(a): Parameters<PatchCheckArgs>) -> Result<String, String> {
        check_patch_json(&a.json)?;
        let errors = ask!(self, p => p.patch_check(&a.json));
        to_text(json!({ "valid": errors.is_empty(), "errors": errors }))
    }

    #[tool(description = "Save a patch file after checking it. Needs `base_hash`, the hash from the patch_source you based the JSON on (\"\" only when creating a new patch); if the file changed since, it is refused and you must patch_source again, redo the edit and retry. JSON with errors is refused too (run patch_check first). The write is attributed to you in the history. Writing a file does not change what is playing: a slot that has this patch loaded keeps its program until it is loaded again (patch_load). Format: {\"codj_patch\":1,\"title\":\"...\",\"nodes\":{\"1\":{\"type\":\"Clock\",\"name\":\"clock\",\"params\":{\"source\":\"master\"}},\"2\":{\"type\":\"Saw\",\"ins\":{\"freq\":[\"3\",\"freq\"]}}}}.")]
    async fn patch_write(&self, Parameters(a): Parameters<PatchWriteArgs>) -> Result<String, String> {
        check_patch_name(&a.name)?;
        check_patch_json(&a.json)?;
        let author = self.author();
        self.gate("patch_write", Effect::Safe, &format!("name={} base_hash={} author={author} bytes={}", a.name, a.base_hash, a.json.len()))?;
        let reply = ask!(self, p => p.patch_write(&a.name, &a.json, &a.base_hash, &author));
        to_text(patch_write_result(&reply)?)
    }

    #[tool(description = "Import a NoiseCraft (.ncft) file as a patch named `name`: node types, params and connections map 1:1 where the node exists; MidiIn, ClockOut and Scope are dropped, modules are flattened. Returns {ok, detail} with the notes about what was dropped. Does not change what is playing.")]
    async fn patch_import(&self, Parameters(a): Parameters<PatchImportArgs>) -> Result<String, String> {
        check_patch_name(&a.name)?;
        if !a.path.ends_with(".ncft") || !a.path.starts_with('/') {
            return Err(format!("path must be an absolute path of a .ncft file, got {:?}", a.path));
        }
        let author = self.author();
        self.gate("patch_import", Effect::Safe, &format!("path={} name={} author={author}", a.path, a.name))?;
        to_text(reply_detail(&ask!(self, p => p.patch_import(&a.path, &a.name, &author)))?)
    }

    #[tool(description = "State of the four patch slots P1..P4 (slot 0..3): loaded patch name and title, unsaved_edits, live (the audience hears it) or cue-only (headphones), cpu share (0..1), muted_reason (e.g. \"cpu overrun\"), knobs (name, value, min, max), sequencers (name, pattern 0..7 = A..H, queued pattern or null, current step) and the slot's fader. Knob names here are what patch_action's knob op takes.")]
    async fn patch_state(&self) -> Result<String, String> {
        let slots = ask!(self, p => p.patch_state());
        to_text(patch_state_json(&slots))
    }

    #[tool(description = "The catalog of node types for writing patch JSON: for each type its inputs (with defaults), outputs, params and a note, plus the file format rules. Call this first when you write or change a patch. Signals are plain numbers: frequencies in Hz, gates 0 or 1, levels -1..1; a sequencer's gate output drives an ADSR or a drum sound, its freq output an oscillator.")]
    async fn patch_node_types(&self) -> Result<String, String> {
        to_text(crate::dj::patchdef::catalog_json())
    }

    #[tool(description = "Edit the patch loaded in `slot` live, one op at a time: knob (turn a knob by name), cell (turn a sequencer step on or off, with velocity), pattern (switch the sequencer's pattern, queued to the next bar unless now:true), set_param, set_input, connect, disconnect, add_node, remove_node, rename. Returns {ok} or the reason. The change applies at once and stays an unsaved edit of the slot (undo in codj, patch_write the JSON to keep it). Knob, cell and pattern edits never glitch; a structural edit recompiles the slot and drops it back to cue-only. Edits of a slot that is live (patch_state says so) are heard by the audience at once, so prefer cue-only slots while you experiment. Look at patch_state for knob and sequencer names.")]
    async fn patch_action(&self, Parameters(a): Parameters<PatchActionArgs>) -> Result<String, String> {
        let slot = check_slot(a.slot)?;
        let json = check_patch_action(&a.action)?;
        self.gate("patch_action", Effect::Safe, &format!("slot={slot} action={json}"))?;
        to_text(reply_detail(&ask!(self, p => p.patch_action(slot, &json)))?)
    }

    #[tool(description = "LIVE: compile and load patch `name` into `slot` (0..3), replacing what was there. It starts cue-only (headphones), so loading is silent for the audience until patch_live. Loading into a slot that is live changes what the audience hears, hence LIVE.")]
    async fn patch_load(&self, Parameters(a): Parameters<PatchLoadArgs>) -> Result<String, String> {
        let slot = check_slot(a.slot)?;
        check_patch_name(&a.name)?;
        self.gate("patch_load", Effect::Patch, &format!("slot={slot} name={}", a.name))?;
        to_text(reply_detail(&ask!(self, p => p.patch_load(slot, &a.name)))?)
    }

    #[tool(description = "LIVE: unload the patch of `slot` (0..3). Silences it for the audience if it was live.")]
    async fn patch_unload(&self, Parameters(a): Parameters<PatchSlotArgs>) -> Result<String, String> {
        let slot = check_slot(a.slot)?;
        self.gate("patch_unload", Effect::Patch, &format!("slot={slot}"))?;
        to_text(reply_detail(&ask!(self, p => p.patch_unload(slot)))?)
    }

    #[tool(description = "LIVE: send the patch in `slot` to the master (live=true: the audience hears it) or back to headphones only (live=false). Tweak a patch in cue-only first, then go live; the slot has an output limiter and is muted by itself if it uses too much cpu.")]
    async fn patch_live(&self, Parameters(a): Parameters<PatchLiveArgs>) -> Result<String, String> {
        let slot = check_slot(a.slot)?;
        self.gate("patch_live", Effect::Patch, &format!("slot={slot} live={}", a.live))?;
        to_text(reply_detail(&ask!(self, p => p.patch_live(slot, a.live)))?)
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
    name = "codj",
    instructions = "Control codj/codjd. `library` lists tracks; their `index` is what play_index and queue_add take. Tools marked LIVE are refused unless the server was started with `codj mcp --live`. Sets: sets_list, set_source, set_check, set_write (with base_hash) and the cue/loop/grid/record tools edit set files and never change what is playing; set_language_help has the syntax. Patch decks (synths and drum machines that play with the decks): patch_node_types -> write the JSON -> patch_check -> patch_write (with base_hash) -> patch_load (LIVE) -> patch_action for knobs and sequencer cells -> patch_live (LIVE); patch_list, patch_source, patch_import, patch_state read and edit files and state."
)]
impl ServerHandler for Codj {
    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, rmcp::ErrorData> {
        *self.client.lock().unwrap() = Some(request.client_info.name.clone());
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }
}

/// `codj mcp [--live]`; returns the exit code
pub async fn run(args: &[String]) -> i32 {
    let mut live = false;
    for a in args {
        match a.as_str() {
            "--live" => live = true,
            other => {
                eprintln!("usage: codj mcp [--live]\nunknown option {other}");
                return 2;
            }
        }
    }
    eprintln!(
        "codj mcp: serving on stdio ({})",
        if live { "LIVE: player and master controls allowed" } else { "safe mode, restart with --live to allow player controls" }
    );
    let service = match Codj::new(live).serve(rmcp::transport::stdio()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("codj mcp: {e}");
            return 1;
        }
    };
    match service.waiting().await {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("codj mcp: {e}");
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
    fn patch_tool_guardrail() {
        use Effect::*;
        assert!(needs_live(Patch));
        // reads, files and live edits (knob, cell, pattern) are always allowed
        assert!(!needs_live(Safe));
        assert!(refusal("patch_live").contains("--live") && refusal("patch_live").contains("patch files"));
    }

    #[test]
    fn patch_argument_validation() {
        assert_eq!(check_slot(0), Ok(0));
        assert_eq!(check_slot(3), Ok(3));
        assert!(check_slot(4).unwrap_err().contains("0..3"));
        assert!(check_patch_name("acid-line_2").is_ok());
        for bad in ["", "../x", "a b", "a/b", "x.json"] {
            assert!(check_patch_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_patch_json("{\"nodes\":{}}").is_ok());
        assert!(check_patch_json("{nodes").unwrap_err().contains("not valid JSON"));
        assert!(check_patch_json(&" ".repeat(MAX_PATCH_BYTES + 1)).unwrap_err().contains("limit"));
        // an action is an object with a known op, or a string holding one
        let knob = json!({"op": "knob", "name": "cutoff", "value": 0.5});
        assert_eq!(serde_json::from_str::<Value>(&check_patch_action(&knob).unwrap()).unwrap(), knob);
        assert_eq!(serde_json::from_str::<Value>(&check_patch_action(&Value::String(knob.to_string())).unwrap()).unwrap(), knob);
        assert!(check_patch_action(&json!({"name": "x"})).unwrap_err().contains("\"op\""));
        assert!(check_patch_action(&json!({"op": "explode"})).unwrap_err().contains("unknown op"));
        assert!(check_patch_action(&json!("not json")).unwrap_err().contains("not JSON"));
        assert!(check_patch_action(&json!(7)).is_err());
        for op in PATCH_OPS {
            assert!(check_patch_action(&json!({ "op": op })).is_ok(), "{op}");
        }
    }

    #[test]
    fn patch_state_and_write_replies() {
        let slots = vec![
            crate::dj_types::PatchSlot {
                loaded: true,
                name: "d1".into(),
                live: true,
                knobs: vec![("cutoff".into(), 0.3, 0.0, 1.0)],
                sequencers: vec![("drums".into(), 1, u32::MAX, 5), ("bass".into(), 0, 2, 0)],
                ..Default::default()
            },
            crate::dj_types::PatchSlot::default(),
        ];
        let v = patch_state_json(&slots);
        assert_eq!(v["slots"][0]["label"], "P1");
        assert_eq!(v["slots"][0]["knobs"][0]["name"], "cutoff");
        assert_eq!(v["slots"][0]["sequencers"][0]["pattern_letter"], "B");
        assert!(v["slots"][0]["sequencers"][0]["queued"].is_null(), "u32::MAX means none");
        assert_eq!(v["slots"][0]["sequencers"][1]["queued"], 2);
        assert_eq!(v["slots"][1]["loaded"], false);
        assert_eq!(patch_write_result("ok abc").unwrap()["hash"], "abc");
        let e = patch_write_result("set changed since you read it").unwrap_err();
        assert!(e.contains("patch_source") && e.contains("patch_write"), "{e}");
        assert!(patch_write_result("refused: 1 error(s): x").unwrap_err().starts_with("refused"));
    }

    #[test]
    fn node_types_cover_what_an_agent_needs() {
        let v = crate::dj::patchdef::catalog_json();
        let nodes = v["nodes"].as_array().unwrap();
        for t in ["Clock", "MonoSeq", "GateSeq", "Saw", "ADSR", "Filter", "Kick", "Mix4", "AudioOut", "Knob"] {
            assert!(nodes.iter().any(|n| n["type"] == t), "{t}");
        }
        let seq = nodes.iter().find(|n| n["type"] == "MonoSeq").unwrap();
        assert!(seq["outputs"].as_array().unwrap().iter().any(|o| o == "freq") && seq["inputs"]["clock"].is_number());
        assert!(v["format"]["rules"].is_array());
    }

    #[tokio::test]
    async fn patch_live_tools_are_gated_by_class() {
        let s = Codj::new(false);
        assert!(s.patch_load(Parameters(PatchLoadArgs { slot: 0, name: "x".into() })).await.unwrap_err().contains("--live"));
        assert!(s.patch_unload(Parameters(PatchSlotArgs { slot: 0 })).await.unwrap_err().contains("--live"));
        assert!(s.patch_live(Parameters(PatchLiveArgs { slot: 0, live: true })).await.unwrap_err().contains("--live"));
        // argument errors come before anything else
        assert!(s.patch_load(Parameters(PatchLoadArgs { slot: 9, name: "x".into() })).await.unwrap_err().contains("0..3"));
        assert!(s.patch_action(Parameters(PatchActionArgs { slot: 0, action: json!({"op": "bogus"}) })).await.unwrap_err().contains("unknown op"));
        // the node catalog needs no daemon
        assert!(s.patch_node_types().await.unwrap().contains("AudioOut"));
    }

    #[test]
    fn set_tool_guardrail() {
        use Effect::*;
        assert!(needs_live(SetRuntime));
        assert!(refusal("set_arm").contains("--live") && refusal("set_arm").contains("set editing"));
    }

    #[test]
    fn author_is_sanitized() {
        assert_eq!(agent_author(Some("claude-code")), "agent:claude-code");
        assert_eq!(agent_author(Some("tester")), "agent:tester");
        assert_eq!(agent_author(Some("Claude Code v1.2")), "agent:Claude-Code-v1.2");
        assert_eq!(agent_author(Some("x\ny: z\u{7}")), "agent:x-y-z");
        assert_eq!(agent_author(Some("  ")), "agent:unknown");
        assert_eq!(agent_author(None), "agent:unknown");
        assert_eq!(agent_author(Some("../../etc")), "agent:..-..-etc");
        assert_eq!(agent_author(Some(&"a".repeat(100))).len(), "agent:".len() + 40);
    }

    #[test]
    fn set_replies() {
        assert_eq!(reply_detail("ok").unwrap()["ok"], true);
        assert_eq!(reply_detail("ok cue drop -> aft bar 33").unwrap()["detail"], "cue drop -> aft bar 33");
        assert!(reply_detail("no active set").is_err());
        assert!(reply_detail("okay").is_err());
        assert_eq!(write_result("ok abc123").unwrap()["hash"], "abc123");
        let e = write_result("set changed since you read it").unwrap_err();
        assert!(e.contains("set_source"), "{e}");
        assert_eq!(write_result("kaiku.codj:3:1: nope").unwrap_err(), "kaiku.codj:3:1: nope");
        assert!(check_name("drop").is_ok() && check_name("a b").is_err() && check_name("").is_err());
        assert!(SET_LANGUAGE_HELP.contains("swap") && SET_LANGUAGE_HELP.contains("mix aft -> smk"));
    }

    #[tokio::test]
    async fn set_tools_are_gated_by_class() {
        let s = Codj::new(false);
        // refused before any D-Bus call
        assert!(s.set_arm(Parameters(SetMixArgs { mix: 0 })).await.unwrap_err().contains("--live"));
        assert!(s.set_disarm().await.unwrap_err().contains("--live"));
        assert!(s.set_auto(Parameters(SetAutoArgs { on: true })).await.unwrap_err().contains("--live"));
        assert!(s.set_preview(Parameters(SetPreviewArgs { mix: 0, bars_before: None })).await.unwrap_err().contains("--live"));
        assert!(s.set_load(Parameters(SetTrackArgs { track: 0 })).await.unwrap_err().contains("--live"));
        // argument checks come first for the always-allowed ones
        assert!(s.set_cue(Parameters(SetCaptureArgs { deck: 2, name: "x".into() })).await.unwrap_err().contains("deck"));
        assert!(s.set_cue(Parameters(SetCaptureArgs { deck: 0, name: "a b".into() })).await.unwrap_err().contains("bad name"));
        let help = s.set_language_help().await.unwrap();
        assert!(help.contains("track"));
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
        // no codjd needed: the guardrail answers first
        let s = Codj::new(false);
        let err = s.toggle().await.unwrap_err();
        assert!(err.contains("--live"));
        let err = s.set_volume(Parameters(VolumeArgs { volume: 2.0 })).await.unwrap_err();
        assert!(err.contains("between 0 and 1"));
    }
}
