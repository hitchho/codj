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
pub mod editor;
pub mod glossary;
pub mod help;
pub mod hints;
pub mod keys;
pub mod mixnew;
pub mod patch;
pub mod patchdef;
pub mod patchview;
pub mod native;
pub mod prompt;
pub mod tutor;
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

use crate::dj_types::{DeckInfo, MixerInfo, PatchSlot, SetState};
use crate::types::Music;
use crate::ServerProxy;
use keys::{coalesce, DjCall, DjKeys, Nav};
use render::{Focus, View, WaveView};
use wave::Waveform;

const POLL_EVERY: Duration = Duration::from_millis(33);
const CALL_TIMEOUT: Duration = Duration::from_millis(1500);
/// `:mix new` chains five calls
const MIX_TIMEOUT: Duration = Duration::from_secs(8);
/// warnings are refetched this often (ticks, ~3 s) while a set is active, and when the set changes
const WARN_EVERY: u32 = 90;
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
    /// `RecordAudioState`: (recording, file, seconds, dropped frames, error)
    audio_rec: Option<(bool, String, f64, u64, String)>,
    /// codjd has no `RecordAudioState`
    audio_rec_missing: bool,
    /// the player's queue ("title — artist") and its coming transition, for the UP NEXT box
    queue: Vec<String>,
    up_next: Option<(bool, String, f64, String)>,
    /// feedback of a set command (list, "armed mix 2"), shown like a note
    /// (text, shown until, made at)
    info: Option<(String, Instant, Instant)>,
    /// the active set's errors, drawn over the decks until Esc
    overlay: Option<Vec<String>>,
    /// a text popup (`:help <term>`, the recipes), closed by any key
    popup: Option<(String, Vec<String>)>,
    /// the active set's warnings (`SetWarnings`) and what they were fetched for
    warnings: Vec<String>,
    warn_sig: String,
    /// codjd has no `SetWarnings`: polling stops
    warn_missing: bool,
    /// per deck: the beat grid of the loaded track (path it was fetched for), for bar counting
    grids: [Option<(String, tutor::Grid)>; 2],
    /// the tutor's search for a track pair
    finding: tutor::Finding,
    /// the four patch slots, None until the first good poll
    patches: Option<Vec<PatchSlot>>,
    /// codjd has no `PatchState`: polling stops
    patch_missing: bool,
    /// a patch call finished: poll again now
    patch_poll_now: bool,
    /// the patch pane is open: poll fast, for the moving step column
    patch_fast: bool,
}

/// Short text for a D-Bus failure; an old codjd without the deck API gets a clear hint
pub fn describe_error(what: &str, e: &zbus::Error) -> String {
    let text = e.to_string();
    if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
        format!("codjd has no deck API ({what}): update codjd")
    } else if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        "codjd is not running".to_string()
    } else {
        // the daemon's refusals a newcomer meets first, with the keys that get past them
        let next = if text.contains("other deck is not playing") {
            " · sync follows a playing deck: focus the other one (h/l) and Space it first"
        } else if text.contains("other deck has nothing loaded") {
            " · Tab, j/k to a track, a or b loads it"
        } else {
            ""
        };
        format!("{what}: {text}{next}")
    }
}

/// Like [`describe_error`] for the set runtime, which is newer than the deck API
pub fn describe_set_error(what: &str, e: &zbus::Error) -> String {
    let text = e.to_string();
    if (text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method")) && what.starts_with("Dj") {
        format!("this codjd is older than codj ({what}): `codjd -r` switches it to the installed build")
    } else if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
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
    /// `:set new <name>`: write a starter set, then use it
    New(String),
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
    /// `:rec audio`: the master to a file
    AudioRec(bool),
    /// `:mix new` without a recipe: list them
    Recipes,
    /// `:mix new <recipe> [bars]`
    MixNew(MixNewReq),
    /// `gt`: `DjAutoMix` from the first deck into the second
    AutoMix(u32, u32),
    /// `t{technique}`: `DjTransition` (from, to, recipe or "auto", bars or 0, at the outro, a
    /// track to load onto `to` first or "")
    Transition(u32, u32, &'static str, u32, bool, String),
    /// `:automix [on|off]`: transitions on the beat between tracks; no word toggles
    Automix(Option<bool>),
    /// `:autoplay [on|off]`: queue tracks that blend when the queue runs low; no word toggles
    Autoplay(Option<bool>),
    /// `:split [on|off]`: one pair of headphones, the master left and the cue right; off is the
    /// usual master and headphone outputs
    Split(Option<bool>),
}

/// A call to the patch decks. Like the set calls they run one after another in their own task.
#[derive(Debug, Clone, PartialEq)]
pub enum PatchCall {
    Load(u32, String),
    Unload(u32),
    Live(u32, bool),
    Undo(u32),
    Redo(u32),
    /// `PatchSave` (author `human`)
    Save(u32, String),
    /// `PatchAction` with one op as JSON (a cell, a pattern switch)
    Action(u32, String),
    List,
    /// `:patch new`: write the template, then load it into `slot` when there is a free one
    New { name: String, kind: String, slot: Option<u32> },
    Import { path: String, name: String },
}

impl PatchCall {
    fn name(&self) -> &'static str {
        match self {
            PatchCall::Load(..) => "PatchLoad",
            PatchCall::Unload(_) => "PatchUnload",
            PatchCall::Live(..) => "PatchLive",
            PatchCall::Undo(_) => "PatchUndo",
            PatchCall::Redo(_) => "PatchRedo",
            PatchCall::Save(..) => "PatchSave",
            PatchCall::Action(..) => "PatchAction",
            PatchCall::List => "PatchList",
            PatchCall::New { .. } => "PatchWrite",
            PatchCall::Import { .. } => "PatchImport",
        }
    }

    /// what to say when codjd answered "ok" or `ok <detail>` (slots are P1..P4 on screen)
    fn done(&self, detail: &str) -> String {
        let detail = detail.trim();
        match self {
            PatchCall::Load(s, n) => format!("{n} loaded into P{}: headphones only, :patch live {} on sends it to the room", s + 1, s + 1),
            PatchCall::Unload(s) => format!("P{} unloaded", s + 1),
            PatchCall::Live(s, true) => format!("P{} is LIVE: the room hears it", s + 1),
            PatchCall::Live(s, false) => format!("P{} back to headphones only", s + 1),
            PatchCall::Undo(s) => format!("P{} undone", s + 1),
            PatchCall::Redo(s) => format!("P{} redone", s + 1),
            PatchCall::Save(s, n) => format!("P{} saved as {n}", s + 1),
            PatchCall::Import { name, .. } => match detail.split_once('(') {
                Some((_, notes)) => format!("imported {name}: {}", notes.trim_end_matches(')')),
                None => format!("imported {name}"),
            },
            PatchCall::Action(..) | PatchCall::List | PatchCall::New { .. } => String::new(),
        }
    }
}

/// Short text for a D-Bus failure of the patch decks, which are newer than the deck API
pub fn describe_patch_error(what: &str, e: &zbus::Error) -> String {
    let text = e.to_string();
    if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
        "codjd has no patch decks: update codjd".to_string()
    } else {
        describe_error(what, e)
    }
}

/// `:patch list` output
pub fn format_patch_list(files: &[(String, String)]) -> String {
    if files.is_empty() {
        return "no patches yet: :patch new <name> [drums|acid|pad]".into();
    }
    let items: Vec<String> = files.iter().map(|(n, t)| if t.is_empty() || t == n { n.clone() } else { format!("{n} “{t}”") }).collect();
    format!("patches: {}  (:patch load <slot> <name>)", items.join(" · "))
}

/// The file `:set new` writes: the name and where to go next
pub fn new_set_text(name: &str) -> String {
    format!(
        "# {name}: made in codj with :set new. F10 edits it here, :edit in $EDITOR.\n\
         # :mix new writes a mix between the decks, F9 records what you play.\n\
         # The language: docs/set-language.md in codjd.\n\
         set \"{name}\"\n"
    )
}

fn is_ok(reply: &str) -> bool {
    reply == "ok" || reply.starts_with("ok ") || reply.is_empty()
}

async fn run_patch_call(proxy: &ServerProxy<'_>, call: &PatchCall) -> Result<String, String> {
    let name = call.name();
    let err = |e: zbus::Error| describe_patch_error(name, &e);
    let reply = match call {
        PatchCall::Load(s, n) => proxy.patch_load(*s, n).await.map_err(err)?,
        PatchCall::Unload(s) => proxy.patch_unload(*s).await.map_err(err)?,
        PatchCall::Live(s, on) => proxy.patch_live(*s, *on).await.map_err(err)?,
        PatchCall::Undo(s) => proxy.patch_undo(*s).await.map_err(err)?,
        PatchCall::Redo(s) => proxy.patch_redo(*s).await.map_err(err)?,
        PatchCall::Save(s, n) => proxy.patch_save(*s, n, editor::AUTHOR).await.map_err(err)?,
        PatchCall::Action(s, json) => proxy.patch_action(*s, json).await.map_err(err)?,
        PatchCall::List => return proxy.patch_list().await.map(|l| format_patch_list(&l)).map_err(err),
        PatchCall::Import { path, name } => proxy.patch_import(path, name, editor::AUTHOR).await.map_err(err)?,
        PatchCall::New { name, kind, slot } => {
            let text = patchdef::template(kind, name).ok_or_else(|| format!("no template {kind:?}: drums, acid or pad"))?;
            // an empty base hash creates the file; an existing patch of that name is refused
            let r = proxy.patch_write(name, &text, "", editor::AUTHOR).await.map_err(err)?;
            if !is_ok(&r) {
                // the daemon's reply speaks of its API (PatchSource, hashes); say what to type
                if r.contains("exists") {
                    let slot = slot.map_or("<slot>".to_string(), |s| (s + 1).to_string());
                    return Err(format!("{name} exists: :patch load {slot} {name} plays it, or pick another name"));
                }
                return Err(format!("PatchWrite: {r}"));
            }
            return match slot {
                Some(s) => {
                    let r = proxy.patch_load(*s, name).await.map_err(|e| describe_patch_error("PatchLoad", &e))?;
                    if is_ok(&r) {
                        Ok(format!("created {name} ({kind}) in P{}: headphones only, p sends it to the room", s + 1))
                    } else {
                        Err(format!("created {name}, but PatchLoad: {r}"))
                    }
                }
                None => Ok(format!("created {name} ({kind}); all four slots are in use: :patch load <slot> {name}")),
            };
        }
    };
    if is_ok(&reply) {
        Ok(call.done(reply.strip_prefix("ok").unwrap_or("")))
    } else {
        Err(format!("{name}: {reply}"))
    }
}

/// Everything `:mix new` needs, taken from the screen when the command ran
#[derive(Debug, Clone, PartialEq)]
pub struct MixNewReq {
    pub recipe: String,
    pub bars: Option<u32>,
    pub set: String,
    pub pair: mixnew::Pair,
}

impl SetCall {
    fn name(&self) -> &'static str {
        match self {
            SetCall::Use(_) => "SetUse",
            SetCall::New(_) => "SetWrite",
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
            SetCall::AudioRec(_) => "RecordAudio",
            SetCall::Recipes => "SetRecipes",
            SetCall::MixNew(_) => "SetRecipeText",
            SetCall::AutoMix(..) => "DjAutoMix",
            SetCall::Transition(..) => "DjTransition",
            SetCall::Automix(_) => "Automix",
            SetCall::Autoplay(_) => "Autoplay",
            SetCall::Split(_) => "DjMode",
        }
    }

    /// what to say when codjd answered "ok" (mixes and tracks are numbered from 1 on screen)
    fn done(&self) -> String {
        match self {
            SetCall::Use(n) if n.is_empty() => "no set active".into(),
            SetCall::Use(n) => format!("set {n} active"),
            SetCall::New(n) => format!("set {n} made and active: :mix new writes a mix, F9 records, F10 edits"),
            SetCall::Arm(m) => format!("armed mix {}", m + 1),
            SetCall::Disarm => "cancelled: a planned transition is dropped, a running one rolled back".into(),
            SetCall::Auto(on) => format!("auto {}", if *on { "on" } else { "off" }),
            SetCall::Preview(m, bars) => format!("previewing mix {} from {bars} bars before", m + 1),
            SetCall::Load(t) => format!("loaded track {}", t + 1),
            SetCall::List | SetCall::Errors(_) => String::new(),
            SetCall::Cue(d, n) => format!("cue {n} saved from deck {}", deck_letter(*d)),
            SetCall::LoopSave(d, n) => format!("loop {n} saved from deck {}", deck_letter(*d)),
            SetCall::GridSave(d) => format!("grid of deck {} saved", deck_letter(*d)),
            SetCall::Record(true) => "recording: play the mix, F9 or :rec off stops".into(),
            SetCall::Record(false) => "recording stopped".into(),
            SetCall::AudioRec(on) => if *on { "recording the room" } else { "recording saved" }.into(),
            SetCall::Recipes | SetCall::MixNew(_) => String::new(),
            SetCall::AutoMix(_, to) => format!("automix into deck {}", deck_letter(*to)),
            SetCall::Transition(_, to, ..) => format!("transition into deck {}", deck_letter(*to)),
            SetCall::Automix(_) | SetCall::Autoplay(_) | SetCall::Split(_) => String::new(),
        }
    }

    /// what to say when codjd answered `ok <detail>`
    fn done_with(&self, detail: &str) -> String {
        let text = capture::pretty_reply(detail);
        match self {
            SetCall::Record(false) => format!("recorded {text}"),
            SetCall::Record(true) => format!("recording: {text}"),
            SetCall::AudioRec(true) => format!("● recording the room to {} · :rec audio stops", home_short(detail.trim())),
            SetCall::AudioRec(false) => format!("saved {}", home_short(detail.trim())),
            // the first line says what it chose; the set text follows (for saving, not for the status line)
            SetCall::AutoMix(_, to) => {
                format!("automix into deck {}: {}", deck_letter(*to), detail.lines().next().unwrap_or("").trim())
            }
            SetCall::Transition(_, to, ..) => {
                format!("into deck {}: {} · F8 cancels", deck_letter(*to), detail.lines().next().unwrap_or("").trim())
            }
            SetCall::Automix(_) if detail == "on" => "automix on: tracks blend into each other on the beat".into(),
            SetCall::Automix(_) => "automix off: tracks play to their end".into(),
            SetCall::Autoplay(_) if detail == "on" => "autoplay on: when the queue runs low, tracks that blend are queued".into(),
            SetCall::Autoplay(_) => "autoplay off: the list plays in order".into(),
            SetCall::Split(_) if detail == "on" => "split: the master in your left ear, the cue (what you prepare) in the right".into(),
            SetCall::Split(_) => "split off: the master on the main output, the cue on the headphones".into(),
            _ => text,
        }
    }
}

/// Where `Enter` loads: the focused deck, else the other one when the focused one is playing;
/// None when both play (a load would cut what the room hears)
pub fn smart_deck(focus: u8, playing: [bool; 2]) -> Option<u8> {
    let other = 1 - focus.min(1);
    if !playing[focus.min(1) as usize] {
        Some(focus.min(1))
    } else if !playing[other as usize] {
        Some(other)
    } else {
        None
    }
}

/// `/home/me/Music/x.wav` as `~/Music/x.wav`
fn home_short(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && path.starts_with(&h) => format!("~{}", &path[h.len()..]),
        _ => path.to_string(),
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

/// The `:errors` overlay: the errors, then the warnings marked as such (drawn yellow)
pub fn errors_with_warnings(mut errors: Vec<String>, warnings: Vec<String>) -> Vec<String> {
    errors.extend(warnings.into_iter().map(|w| format!("warning: {w}")));
    errors
}

enum SetReply {
    Status(String),
    List(Vec<(String, String, u32)>),
    Errors(Vec<String>),
}

/// `:mix new`: the recipe's block, merged into the set's text, checked and written. Returns the
/// text to show, or the reason it did not happen.
async fn run_mix_new(proxy: &ServerProxy<'_>, req: &MixNewReq) -> Result<String, String> {
    let what = |e: zbus::Error| mixnew::describe_error("SetRecipes", &e);
    let recipes = proxy.set_recipes().await.map_err(what)?;
    let Some((name, _, default_bars)) = recipes.iter().find(|r| r.0.eq_ignore_ascii_case(&req.recipe)).cloned() else {
        let names: Vec<&str> = recipes.iter().map(|r| r.0.as_str()).collect();
        return Err(format!("no recipe {:?}; recipes: {} (:mix new lists them)", req.recipe, names.join(", ")));
    };
    let bars = req.bars.unwrap_or(default_bars);
    let pair = &req.pair;
    let block = proxy
        .set_recipe_text(&name, &pair.from_id, &pair.to_id, bars)
        .await
        .map_err(|e| mixnew::describe_error("SetRecipeText", &e))?;
    if !mixnew::looks_like_block(&block) {
        return Err(format!("SetRecipeText: {}", block.lines().next().unwrap_or("empty reply")));
    }
    let (source, hash) = proxy.set_source(&req.set).await.map_err(|e| describe_set_error("SetSource", &e))?;
    let mut merged = mixnew::merge_block(&source, &block, &pair.from_id, &pair.to_id, false);
    let mut errors = proxy.set_check(&merged.text).await.map_err(|e| describe_set_error("SetCheck", &e))?;
    if !errors.is_empty() && merged.existing.is_some() {
        // the older block of the same two tracks may be what the new one clashes with
        let replaced = mixnew::merge_block(&source, &block, &pair.from_id, &pair.to_id, true);
        let again = proxy.set_check(&replaced.text).await.map_err(|e| describe_set_error("SetCheck", &e))?;
        if again.is_empty() {
            merged = replaced;
            errors = again;
        }
    }
    if let Some(first) = errors.first() {
        return Err(format!("the {name} block has {} error{}: {first}", errors.len(), if errors.len() == 1 { "" } else { "s" }));
    }
    let reply = proxy.set_write(&req.set, &merged.text, &hash, "human").await.map_err(|e| describe_set_error("SetWrite", &e))?;
    if reply == "ok" || reply.starts_with("ok ") {
        Ok(mixnew::done_text(&name, pair, bars, &merged))
    } else {
        Err(format!("SetWrite: {reply}"))
    }
}

async fn run_set_call(proxy: &ServerProxy<'_>, call: &SetCall) -> zbus::Result<SetReply> {
    Ok(match call {
        SetCall::Use(n) => SetReply::Status(proxy.set_use(n).await?),
        SetCall::New(n) => {
            // an empty base hash creates the file; an existing set of that name is refused
            let r = proxy.set_write(n, &new_set_text(n), "", editor::AUTHOR).await?;
            if !is_ok(&r) {
                return Ok(SetReply::Status(if r.contains("exists") { format!("{n} exists: :set {n} uses it, or pick another name") } else { r }));
            }
            SetReply::Status(proxy.set_use(n).await?)
        }
        SetCall::Arm(m) => SetReply::Status(proxy.set_arm(*m).await?),
        SetCall::Disarm => SetReply::Status(proxy.set_disarm().await?),
        SetCall::Auto(on) => SetReply::Status(proxy.set_auto(*on).await?),
        SetCall::Preview(m, bars) => SetReply::Status(proxy.set_preview(*m, *bars).await?),
        SetCall::Load(t) => SetReply::Status(proxy.set_load(*t).await?),
        SetCall::List => SetReply::List(proxy.set_list().await?),
        SetCall::Errors(n) => {
            let errors = proxy.set_errors(n).await?;
            // an old codjd without warnings just has none
            SetReply::Errors(errors_with_warnings(errors, proxy.set_warnings(n).await.unwrap_or_default()))
        }
        SetCall::Recipes | SetCall::MixNew(_) => unreachable!("handled by the executor"),
        SetCall::Cue(d, n) => SetReply::Status(proxy.set_cue(*d, n).await?),
        SetCall::LoopSave(d, n) => SetReply::Status(proxy.set_loop_capture(*d, n).await?),
        SetCall::GridSave(d) => SetReply::Status(proxy.set_grid_capture(*d).await?),
        SetCall::Record(on) => SetReply::Status(proxy.set_record(*on).await?),
        SetCall::AudioRec(on) => SetReply::Status(proxy.record_audio(*on).await?),
        SetCall::AutoMix(from, to) => SetReply::Status(proxy.dj_auto_mix(*from, *to).await?),
        SetCall::Transition(from, to, recipe, bars, outro, path) => SetReply::Status(proxy.dj_transition(*from, *to, recipe, *bars, *outro, path).await?),
        // the reply says which way it went, so a toggle can show it
        SetCall::Automix(on) => {
            let on = match on {
                Some(on) => *on,
                None => !proxy.automix_state().await?.0,
            };
            SetReply::Status(with_switch(proxy.automix(on).await?, on))
        }
        SetCall::Autoplay(on) => {
            let on = match on {
                Some(on) => *on,
                None => !proxy.autoplay_state().await?.0,
            };
            SetReply::Status(with_switch(proxy.autoplay(on).await?, on))
        }
        SetCall::Split(on) => {
            let on = match on {
                Some(on) => *on,
                None => proxy.dj_mixer().await?.mode != "split",
            };
            SetReply::Status(with_switch(proxy.dj_mode(if on { "split" } else { "internal" }).await?, on))
        }
    })
}

/// `ok` becomes `ok on` / `ok off`; anything else is codjd's reason
fn with_switch(reply: String, on: bool) -> String {
    if reply == "ok" { format!("ok {}", if on { "on" } else { "off" }) } else { reply }
}

/// The connection to codjd for one DJ session: tasks end when this is dropped
pub struct Session {
    shared: Arc<Mutex<Shared>>,
    tx: mpsc::UnboundedSender<DjCall>,
    set_tx: mpsc::UnboundedSender<SetCall>,
    patch_tx: mpsc::UnboundedSender<PatchCall>,
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
        DjCall::Seek { deck, seconds } => proxy.dj_seek(*deck, *seconds).await,
        DjCall::Nudge { deck, amount } => proxy.dj_nudge(*deck, *amount).await,
        DjCall::BeatJump { deck, beats } => proxy.dj_beat_jump(*deck, *beats).await,
        DjCall::Loop { deck, beats } => proxy.dj_loop(*deck, *beats).await,
        DjCall::Hotcue { deck, slot, action } => proxy.dj_hotcue(*deck, *slot, action).await,
        DjCall::Set { control, deck, value } => proxy.dj_set(control, *deck, *value).await,
        DjCall::PatchKnob { slot, name, value } => {
            proxy.patch_action(*slot, &serde_json::json!({"op": "knob", "name": name, "value": value}).to_string()).await
        }
        DjCall::VolumeStep(delta) => {
            let now = proxy.get_volume().await? as f64;
            let v = ((now + delta).clamp(0., 1.) * 100.).round() / 100.;
            proxy.volume(v as f32).await.map(|_| "ok".to_string())
        }
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

/// Reads `SetWarnings` of the active set when it changed (name, file, errors, track and mix
/// counts) and every [`WARN_EVERY`] ticks; an old codjd without the method ends the polling
async fn poll_warnings(proxy: &ServerProxy<'_>, shared: &Arc<Mutex<Shared>>, tick: u32) {
    let name = {
        let mut s = shared.lock().unwrap();
        let Some(set) = s.set.clone().filter(|x| x.is_active()) else {
            s.warnings.clear();
            s.warn_sig.clear();
            return;
        };
        let sig = format!("{}|{}|{}|{}|{}", set.name, set.file, set.errors, set.tracks.len(), set.mixes.len());
        if s.warn_missing || (sig == s.warn_sig && tick % WARN_EVERY != 0) {
            return;
        }
        s.warn_sig = sig;
        set.name
    };
    let reply = tokio::time::timeout(CALL_TIMEOUT, proxy.set_warnings(&name)).await;
    let mut s = shared.lock().unwrap();
    match reply {
        Ok(Ok(w)) => s.warnings = w,
        Ok(Err(e)) => {
            let text = e.to_string();
            if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
                s.warn_missing = true;
                s.warnings.clear();
            }
        }
        Err(_) => {}
    }
}

/// the patch slots are polled every other tick (~15 Hz) while the pane is open, else once a second
const PATCH_POLL_FAST: u32 = 2;
const PATCH_POLL_IDLE: u32 = 30;

/// Reads `PatchState` when it is due; an old codjd without the method ends the polling
async fn poll_patches(proxy: &ServerProxy<'_>, shared: &Arc<Mutex<Shared>>, tick: u32) {
    let due = {
        let mut s = shared.lock().unwrap();
        let every = if s.patch_fast { PATCH_POLL_FAST } else { PATCH_POLL_IDLE };
        let forced = std::mem::take(&mut s.patch_poll_now);
        !s.patch_missing && (forced || tick % every == 0)
    };
    if !due {
        return;
    }
    let reply = tokio::time::timeout(CALL_TIMEOUT, proxy.patch_state()).await;
    let mut s = shared.lock().unwrap();
    match reply {
        Ok(Ok(slots)) => s.patches = Some(slots),
        Ok(Err(e)) => {
            let text = e.to_string();
            if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
                s.patch_missing = true;
                s.patches = None;
            }
        }
        Err(_) => {}
    }
}

/// Reads the player's queue and its coming automix transition, once a second
async fn poll_queue(proxy: &ServerProxy<'_>, shared: &Arc<Mutex<Shared>>, tick: u32) {
    if tick % REC_POLL_IDLE != 0 {
        return;
    }
    let queue = tokio::time::timeout(CALL_TIMEOUT, proxy.get_queue()).await.ok().and_then(|r| r.ok());
    let next = tokio::time::timeout(CALL_TIMEOUT, proxy.automix_state()).await.ok().and_then(|r| r.ok());
    let mut s = shared.lock().unwrap();
    if let Some(q) = queue {
        s.queue = q.iter().map(|m| if m.artist.is_empty() { m.title.clone() } else { format!("{} — {}", m.title, m.artist) }).collect();
    }
    s.up_next = next;
}

fn is_unknown_method(text: &str) -> bool {
    text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method")
}

/// Reads `SetRecording` and `RecordAudioState` when it is due: ~4 Hz while recording, once a second otherwise
async fn poll_recording(proxy: &ServerProxy<'_>, shared: &Arc<Mutex<Shared>>, tick: u32) {
    let due = {
        let mut s = shared.lock().unwrap();
        let recording = s.rec.is_some_and(|r| r.0) || s.audio_rec.as_ref().is_some_and(|r| r.0);
        let every = if recording { REC_POLL_ACTIVE } else { REC_POLL_IDLE };
        let forced = std::mem::take(&mut s.rec_poll_now);
        !(s.rec_missing && s.audio_rec_missing) && (forced || tick % every == 0)
    };
    if !due {
        return;
    }
    let audio = if shared.lock().unwrap().audio_rec_missing {
        None
    } else {
        Some(tokio::time::timeout(CALL_TIMEOUT, proxy.record_audio_state()).await)
    };
    let reply = tokio::time::timeout(CALL_TIMEOUT, proxy.set_recording()).await;
    let mut s = shared.lock().unwrap();
    match audio {
        Some(Ok(Ok(state))) => s.audio_rec = Some(state),
        Some(Ok(Err(e))) if is_unknown_method(&e.to_string()) => {
            s.audio_rec_missing = true;
            s.audio_rec = None;
        }
        _ => {}
    }
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
            audio_rec: None,
            audio_rec_missing: false,
            queue: Vec::new(),
            up_next: None,
            rec_poll_now: false,
            info: None,
            overlay: None,
            popup: None,
            warnings: Vec::new(),
            warn_sig: String::new(),
            warn_missing: false,
            grids: [None, None],
            finding: tutor::Finding::Searching,
            patches: None,
            patch_missing: false,
            patch_poll_now: false,
            patch_fast: false,
        }));
        let (tx, mut rx) = mpsc::unbounded_channel::<DjCall>();
        let (set_tx, mut set_rx) = mpsc::unbounded_channel::<SetCall>();
        let (patch_tx, mut patch_rx) = mpsc::unbounded_channel::<PatchCall>();
        let conn_for_patch = conn.clone();

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
                    poll_warnings(&proxy, &shared, tick).await;
                    poll_recording(&proxy, &shared, tick).await;
                    poll_queue(&proxy, &shared, tick).await;
                    poll_patches(&proxy, &shared, tick).await;
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
                    // the recipe commands chain several calls and have their own replies
                    if matches!(call, SetCall::Recipes | SetCall::MixNew(_)) {
                        let result = tokio::time::timeout(MIX_TIMEOUT, async {
                            match &call {
                                SetCall::MixNew(req) => run_mix_new(&proxy, req).await.map(|text| (text, None)),
                                _ => proxy
                                    .set_recipes()
                                    .await
                                    .map(|r| mixnew::recipes_popup(&r))
                                    .map(|p| (String::new(), Some(p)))
                                    .map_err(|e| mixnew::describe_error("SetRecipes", &e)),
                            }
                        })
                        .await;
                        let mut s = shared.lock().unwrap();
                        let now = Instant::now();
                        match result {
                            Ok(Ok((text, popup))) => {
                                s.call_error = None;
                                if let Some(p) = popup {
                                    s.popup = Some(p);
                                } else {
                                    s.info = Some((text, now + NOTE_FOR * 2, now));
                                }
                            }
                            Ok(Err(e)) => s.call_error = Some((e, now)),
                            Err(_) => s.call_error = Some((format!("{}: codjd does not answer", call.name()), now)),
                        }
                        s.set_poll_now = true;
                        continue;
                    }
                    let reply = tokio::time::timeout(CALL_TIMEOUT, run_set_call(&proxy, &call)).await;
                    let mut s = shared.lock().unwrap();
                    let now = Instant::now();
                    // a success must not hide behind the error of an earlier call
                    if matches!(&reply, Ok(Ok(SetReply::Status(r))) if r.is_empty() || r == "ok" || r.starts_with("ok ")) {
                        s.call_error = None;
                    }
                    match reply {
                        Ok(Ok(SetReply::Status(r))) if r == "ok" || r.is_empty() => {
                            s.info = Some((call.done(), now + NOTE_FOR, now));
                        }
                        Ok(Ok(SetReply::Status(r))) if r.starts_with("ok ") => {
                            s.info = Some((call.done_with(&r[3..]), now + NOTE_FOR, now));
                        }
                        Ok(Ok(SetReply::Status(r))) => s.call_error = Some((format!("{}: {r}", call.name()), now)),
                        Ok(Ok(SetReply::List(sets))) => s.info = Some((format_set_list(&sets), now + NOTE_FOR * 2, now)),
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

        let patch_executor = {
            let shared = shared.clone();
            tokio::spawn(async move {
                let Ok(proxy) = ServerProxy::new(&conn_for_patch).await else { return };
                while let Some(call) = patch_rx.recv().await {
                    let limit = if matches!(call, PatchCall::New { .. } | PatchCall::Import { .. }) { MIX_TIMEOUT } else { CALL_TIMEOUT };
                    let reply = tokio::time::timeout(limit, run_patch_call(&proxy, &call)).await;
                    let mut s = shared.lock().unwrap();
                    let now = Instant::now();
                    match reply {
                        Ok(Ok(text)) => {
                            s.call_error = None;
                            if !text.is_empty() {
                                s.info = Some((text, now + NOTE_FOR * 2, now));
                            }
                        }
                        Ok(Err(e)) => s.call_error = Some((e, now)),
                        Err(_) => s.call_error = Some((format!("{}: codjd does not answer", call.name()), now)),
                    }
                    s.patch_poll_now = true;
                }
            })
        };

        Session { shared, tx, set_tx, patch_tx, conn, tasks: vec![poller, executor, set_executor, patch_executor] }
    }

    pub fn patch_call(&self, call: PatchCall) {
        let _ = self.patch_tx.send(call);
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

    /// The tutor's search for two tracks with close tempos: `GetGrid` on a few tracks and
    /// `SimilarTo` of the first, in a blocking task (see `tutor::find_pair` for the caps)
    fn find_pair(&self, library: Vec<(String, String)>, playlist_index: Vec<usize>) {
        self.shared.lock().unwrap().finding = tutor::Finding::Searching;
        let (shared, conn) = (self.shared.clone(), self.conn.clone());
        tokio::task::spawn_blocking(move || {
            let h = tokio::runtime::Handle::current();
            let found = h.block_on(ServerProxy::new(&conn)).ok().and_then(|proxy| {
                tutor::find_pair(
                    &library,
                    |p| {
                        // SimilarTo answers playlist indices: turn them into positions in `library`
                        h.block_on(async { tokio::time::timeout(CALL_TIMEOUT, proxy.similar_to(p, 8)).await.ok().and_then(|r| r.ok()).unwrap_or_default() })
                            .into_iter()
                            .filter_map(|i| playlist_index.iter().position(|&x| x == i as usize))
                            .collect()
                    },
                    |p| {
                        h.block_on(async {
                            let g = tokio::time::timeout(CALL_TIMEOUT, proxy.get_grid(p)).await.ok()?.ok()?;
                            // analysed, steady and not in doubt about half or double time
                            (g.7 && !g.6 && !g.5).then_some(g.0)
                        })
                    },
                )
            });
            shared.lock().unwrap().finding = found.map_or(tutor::Finding::Nothing, tutor::Finding::Found);
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
                            s.doubt[i] = Some((p.clone(), g.6));
                            s.grids[i] = Some((p, tutor::Grid { first_beat_s: g.1, downbeat_offset: g.2, beats_per_bar: g.3 }));
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
    /// `g` pressed once; a second `g` goes to the top
    pending_g: bool,
}

/// What a key does while browsing the library
#[derive(Debug, PartialEq, Eq)]
pub enum Browse {
    Nothing,
    Load(u8),
    LoadFree,
    /// `q`: add the highlighted track to the player's queue
    Queue,
    Help,
    /// a key the library has no use for: back on the decks, the deck keys take it
    Pass,
}

/// What a `:` command asks for
#[derive(Debug, PartialEq)]
pub enum Command {
    /// back to the normal view
    Exit,
    /// `:automix [on|off]`
    Automix(Option<bool>),
    /// `:autoplay [on|off]`
    Autoplay(Option<bool>),
    /// `:split [on|off]`
    Split(Option<bool>),
    Load(u8),
    /// correct a deck's detected tempo by a factor; 0 resets
    Grid(u8, f64),
    /// `:set` alone: list the sets
    SetList,
    /// `:set <name>`, "" = none
    SetUse(String),
    /// `:set new <name>`
    SetNew(String),
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
    /// `:rec audio [on|off]`: the master to a WAV file; None toggles
    AudioRec(Option<bool>),
    /// `:e`: the in-TUI editor pane on the active set
    Editor,
    /// `:ai <text>`: run the agent on a request ("" opens the prompt box)
    /// `:ai new`: forget the conversation, the next prompt starts a new one
    AiNew,
    Ai(String),
    /// `:help [term]`: the glossary ("" lists the terms)
    Help(String),
    /// `:mix new [recipe [bars]]`: no recipe lists them
    MixNew { recipe: Option<String>, bars: Option<u32> },
    /// `:tutor [reset]`
    Tutor { reset: bool },
    /// `:tutor next`: skip the current step
    TutorNext,
    /// `:patch ...`
    Patch(PatchCmd),
    /// `:fader 0.5`, `:low kill`, `:filter -0.3`, `:xfade a`: set a control (deck None = the focused one)
    Mix { ctl: MixCtl, deck: Option<u8>, value: f64 },
    /// `:delm a` / `:delm!`: clear marks (hot cue slots 0..7) of the focused deck
    DelMarks(Vec<u8>),
    /// `:quantize [on|off]`
    Quantize(Option<bool>),
    /// `:hp [a|b] [on|off]`: headphone cue (deck None = the focused one)
    Headphones(Option<u8>, Option<bool>),
    /// `:focus a|b`
    Focus(u8),
    /// `:w [name]` with the patch pane open: save the selected slot's patch
    PatchSave(Option<String>),
    Nothing,
    Unknown(String),
}

/// The controls `:fader`, `:low` ... set
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixCtl {
    Fader,
    Low,
    Mid,
    High,
    Filter,
    Xfade,
}

impl MixCtl {
    fn name(self) -> &'static str {
        match self {
            MixCtl::Fader => "fader",
            MixCtl::Low => "low",
            MixCtl::Mid => "mid",
            MixCtl::High => "high",
            MixCtl::Filter => "filter",
            MixCtl::Xfade => "xfade",
        }
    }

    /// (lowest, neutral, highest)
    fn range(self) -> (f64, f64, f64) {
        match self {
            MixCtl::Fader => (0.0, 1.0, 1.0),
            _ => (-1.0, 0.0, 1.0),
        }
    }

    fn usage(self) -> String {
        let (lo, _, _) = self.range();
        match self {
            MixCtl::Fader => "usage: fader [a|b] <0..1 | off | full>".to_string(),
            MixCtl::Xfade => "usage: xfade <-1..1 | a | b | center> (-1 is all deck A, 1 all deck B)".to_string(),
            c => format!("usage: {} [a|b] <{lo}..1 | kill | flat>", c.name()),
        }
    }
}

/// `:fader 0.5`, `:low kill`, `:filter -0.3`, `:xfade a`, with an optional `a` / `b` first
fn parse_mix_ctl(ctl: MixCtl, words: &[&str]) -> Command {
    let (deck, rest) = match (ctl, words.first().copied()) {
        (MixCtl::Xfade, _) => (None, words),
        (_, Some("a" | "A")) => (Some(0), &words[1..]),
        (_, Some("b" | "B")) => (Some(1), &words[1..]),
        _ => (None, words),
    };
    let (lo, neutral, hi) = ctl.range();
    let value = match rest {
        [w] => match w.to_ascii_lowercase().as_str() {
            "kill" | "off" | "min" | "mute" => Some(lo),
            "flat" | "reset" | "center" | "centre" | "unity" | "neutral" => Some(neutral),
            "full" | "max" | "on" => Some(hi),
            "a" if ctl == MixCtl::Xfade => Some(-1.0),
            "b" if ctl == MixCtl::Xfade => Some(1.0),
            n => n.parse::<f64>().ok().filter(|v| v.is_finite() && (lo..=hi).contains(v)),
        },
        _ => None,
    };
    match value {
        Some(value) => Command::Mix { ctl, deck, value },
        None => Command::Unknown(ctl.usage()),
    }
}

/// `:delm a`, `:delm a c`, `:delm a-c`, `:delm abc`, `:delm!` (every mark)
fn parse_delmarks(all: bool, words: &[&str]) -> Command {
    const USAGE: &str = "usage: delm <marks a-h> (a, a c, a-c) or delm! for all of them";
    if all {
        return if words.is_empty() { Command::DelMarks((0..keys::MARKS).collect()) } else { Command::Unknown(USAGE.into()) };
    }
    let mut slots = Vec::new();
    let slot = |c: char| ('a'..='h').contains(&c).then(|| c as u8 - b'a');
    for w in words {
        let cs: Vec<char> = w.chars().collect();
        if let [a, '-', b] = cs.as_slice() {
            match (slot(*a), slot(*b)) {
                (Some(a), Some(b)) if a <= b => slots.extend(a..=b),
                _ => return Command::Unknown(USAGE.into()),
            }
            continue;
        }
        for c in cs {
            match slot(c) {
                Some(s) => slots.push(s),
                None => return Command::Unknown(USAGE.into()),
            }
        }
    }
    if slots.is_empty() {
        return Command::Unknown(USAGE.into());
    }
    slots.sort_unstable();
    slots.dedup();
    Command::DelMarks(slots)
}

fn on_off(w: &str) -> Option<bool> {
    match w {
        "on" | "1" | "true" => Some(true),
        "off" | "0" | "false" => Some(false),
        _ => None,
    }
}

/// What `:patch` asks for; slots are 0-based here (P1 = 0)
#[derive(Debug, PartialEq)]
pub enum PatchCmd {
    /// the pane
    Open,
    List,
    /// slot (None = the first free one), patch name
    Load(Option<u32>, String),
    Unload(u32),
    /// slot, on / off (None toggles)
    Live(u32, Option<bool>),
    New { name: String, kind: String },
    Import { path: String, name: String },
    /// the patch's JSON in the editor pane
    Edit(String),
}

const PATCH_USAGE: &str = "usage: patch [list | load [slot] <name> | unload <slot> | live <slot> [on|off] | new <name> [drums|acid|pad] | import <file.ncft> [name] | edit <name>]";

/// `:patch` words (after `patch`) as a command
pub fn parse_patch(words: &[&str]) -> Command {
    let usage = |m: &str| Command::Unknown(m.to_string());
    let slot_of = |w: Option<&&str>| w.and_then(|w| one_based(Some(w))).filter(|s| (*s as usize) < patchdef::SLOTS);
    let name_of = |w: Option<&&str>| w.map(|w| w.to_string()).filter(|w| patchdef::valid_name(w));
    let bad_name = "patch names use letters, digits, - and _";
    match words.first().copied() {
        None => Command::Patch(PatchCmd::Open),
        Some("list" | "ls") => Command::Patch(PatchCmd::List),
        Some("load") => match (slot_of(words.get(1)), words.len()) {
            (Some(s), 3) => name_of(words.get(2)).map(|n| Command::Patch(PatchCmd::Load(Some(s), n))).unwrap_or_else(|| usage(bad_name)),
            (None, 2) => name_of(words.get(1)).map(|n| Command::Patch(PatchCmd::Load(None, n))).unwrap_or_else(|| usage(bad_name)),
            _ => usage("usage: patch load <slot 1-4> <name> (or just a name for the first free slot)"),
        },
        Some("unload" | "eject") => slot_of(words.get(1)).filter(|_| words.len() == 2).map(|s| Command::Patch(PatchCmd::Unload(s))).unwrap_or_else(|| usage("usage: patch unload <slot 1-4>")),
        Some("live") => {
            let on = match words.get(2).copied() {
                None => Some(None),
                Some("on" | "1" | "true") => Some(Some(true)),
                Some("off" | "0" | "false" | "cue") => Some(Some(false)),
                Some(_) => None,
            };
            match (slot_of(words.get(1)), on) {
                (Some(s), Some(on)) if words.len() <= 3 => Command::Patch(PatchCmd::Live(s, on)),
                _ => usage("usage: patch live <slot 1-4> [on|off]"),
            }
        }
        Some("new") => {
            let kind = words.get(2).copied().unwrap_or("drums");
            match (name_of(words.get(1)), patchdef::TEMPLATE_KINDS.contains(&kind), words.len() <= 3) {
                (Some(name), true, true) => Command::Patch(PatchCmd::New { name, kind: kind.to_string() }),
                (None, ..) => usage(if words.len() < 2 { "usage: patch new <name> [drums|acid|pad]" } else { bad_name }),
                _ => usage("usage: patch new <name> [drums|acid|pad]"),
            }
        }
        Some("import") => {
            let Some(path) = words.get(1) else { return usage("usage: patch import <file.ncft> [name]") };
            let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().replace(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'), "_")).unwrap_or_default();
            let name = words.get(2).map(|w| w.to_string()).unwrap_or(stem);
            if !patchdef::valid_name(&name) || words.len() > 3 {
                return usage("usage: patch import <file.ncft> [name] (names use letters, digits, - and _)");
            }
            Command::Patch(PatchCmd::Import { path: path.to_string(), name })
        }
        Some("edit") => name_of(words.get(1)).filter(|_| words.len() == 2).map(|n| Command::Patch(PatchCmd::Edit(n))).unwrap_or_else(|| usage("usage: patch edit <name>")),
        Some(_) => usage(PATCH_USAGE),
    }
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
    if let Some(rest) = cmd.trim().strip_prefix("ai") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            if rest.trim() == "new" {
                return Command::AiNew;
            }
            return Command::Ai(rest.trim().to_string());
        }
    }
    let mut words = cmd.split_whitespace();
    match words.next() {
        Some("help" | "h" | "?") => Command::Help(words.collect::<Vec<_>>().join(" ")),
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
        Some(w @ ("automix" | "autoplay" | "split")) => {
            let on = match words.next() {
                None => None,
                Some("on") => Some(true),
                Some("off") => Some(false),
                Some(_) => return Command::Unknown(format!("usage: {w} [on|off]")),
            };
            match w {
                "automix" => Command::Automix(on),
                "autoplay" => Command::Autoplay(on),
                _ => Command::Split(on),
            }
        }
        Some("set") => {
            let name = words.collect::<Vec<_>>().join(" ");
            match name.as_str() {
                "" => Command::SetList,
                "none" | "off" | "-" => Command::SetUse(String::new()),
                n if n.starts_with("new ") || n == "new" => match n.strip_prefix("new").map(str::trim) {
                    Some(name) if patchdef::valid_name(name) => Command::SetNew(name.to_string()),
                    _ => Command::Unknown("usage: set new <name> (letters, digits, - and _)".into()),
                },
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
            let rest: Vec<&str> = words.collect();
            match rest.as_slice() {
                ["audio"] => Command::AudioRec(None),
                ["audio", "on"] => Command::AudioRec(Some(true)),
                ["audio", "off"] => Command::AudioRec(Some(false)),
                ["audio", ..] => Command::Unknown("usage: rec audio [on|off]".into()),
                _ => capture::parse(w, &rest).unwrap_or(Command::Nothing),
            }
        }
        Some("mix") => mixnew::parse(&words.collect::<Vec<_>>()),
        Some("patch" | "patches") => parse_patch(&words.collect::<Vec<_>>()),
        Some("w" | "write") => {
            let rest: Vec<&str> = words.collect();
            match rest.as_slice() {
                [] => Command::PatchSave(None),
                [n] if patchdef::valid_name(n) => Command::PatchSave(Some(n.to_string())),
                _ => Command::Unknown("usage: w [name] (letters, digits, - and _); saves the patch of the selected slot".into()),
            }
        }
        Some("tutor") => match words.next() {
            None => Command::Tutor { reset: false },
            Some("reset") if words.next().is_none() => Command::Tutor { reset: true },
            Some("next" | "skip") if words.next().is_none() => Command::TutorNext,
            Some(_) => Command::Unknown("usage: tutor [reset]".into()),
        },
        Some("e") => Command::Editor,
        Some("fader" | "vol") => parse_mix_ctl(MixCtl::Fader, &words.collect::<Vec<_>>()),
        Some("low" | "bass") => parse_mix_ctl(MixCtl::Low, &words.collect::<Vec<_>>()),
        Some("mid") => parse_mix_ctl(MixCtl::Mid, &words.collect::<Vec<_>>()),
        Some("high" | "treble") => parse_mix_ctl(MixCtl::High, &words.collect::<Vec<_>>()),
        Some("filter" | "flt") => parse_mix_ctl(MixCtl::Filter, &words.collect::<Vec<_>>()),
        Some("xfade" | "crossfader") => parse_mix_ctl(MixCtl::Xfade, &words.collect::<Vec<_>>()),
        Some("delm" | "delmarks") => parse_delmarks(false, &words.collect::<Vec<_>>()),
        Some("delm!" | "delmarks!") => parse_delmarks(true, &words.collect::<Vec<_>>()),
        Some("quantize" | "quant") => match words.next() {
            None => Command::Quantize(None),
            Some(w) => on_off(w).map(|on| Command::Quantize(Some(on))).unwrap_or_else(|| Command::Unknown("usage: quantize [on|off]".into())),
        },
        Some("hp" | "headphones") => {
            let mut deck = None;
            let mut on = None;
            for w in words {
                match w {
                    "a" | "A" => deck = Some(0),
                    "b" | "B" => deck = Some(1),
                    w => match on_off(w) {
                        Some(v) => on = Some(v),
                        None => return Command::Unknown("usage: hp [a|b] [on|off]".into()),
                    },
                }
            }
            Command::Headphones(deck, on)
        }
        Some("focus") => match words.next() {
            Some("a" | "A") => Command::Focus(0),
            Some("b" | "B") => Command::Focus(1),
            _ => Command::Unknown("usage: focus a|b (or h / l)".into()),
        },
        None => Command::Nothing,
        Some(other) => Command::Unknown(format!(
            "unknown DJ command: {other} (q: leave DJ mode, fader|low|mid|high|filter [a|b] <value>, xfade <value>, delm <a-h>, delm!, quantize, hp [a|b], focus a|b, patch [new|load|live|edit|list], load a|b|<track>, grid a|b x2|/2|.., set [name], arm <n|next>, disarm, auto on|off, preview <n> [bars], errors, edit, e, ai <request>, help [term], tutor, mix new [recipe], cue <name> [a|b], loopsave <name> [a|b], gridsave [a|b], rec [on|off])"
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
            pending_g: false,
        }
    }

    pub fn typing(&self) -> bool {
        matches!(self.focus, Focus::Search | Focus::Command)
    }

    pub fn browsing(&self) -> bool {
        self.focus == Focus::Library
    }

    pub fn start_browse(&mut self) {
        self.pending_g = false;
        self.focus = Focus::Library;
    }

    /// A key while browsing the library, vim style. The deck keys are off meanwhile.
    pub fn browse_key(&mut self, key: &KeyEvent) -> Browse {
        if key.kind == KeyEventKind::Release {
            return Browse::Nothing;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let g = std::mem::take(&mut self.pending_g);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-1),
            KeyCode::Char('d') if ctrl => self.scroll(10),
            KeyCode::Char('u') if ctrl => self.scroll(-10),
            KeyCode::PageDown => self.scroll(10),
            KeyCode::PageUp => self.scroll(-10),
            KeyCode::Char('g') if g => self.sel = 0,
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') | KeyCode::End => self.sel = self.lib.len().saturating_sub(1),
            KeyCode::Home => self.sel = 0,
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char(':') => self.start_command(),
            KeyCode::Char('a') | KeyCode::Char('<') => return Browse::Load(0),
            KeyCode::Char('q') => return Browse::Queue,
            KeyCode::Char('b') | KeyCode::Char('>') => return Browse::Load(1),
            KeyCode::Enter => return Browse::LoadFree,
            KeyCode::Char('?') => return Browse::Help,
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.focus = Focus::Decks,
            KeyCode::Char(_) | KeyCode::Left | KeyCode::Right | KeyCode::F(_) => {
                self.focus = Focus::Decks;
                return Browse::Pass;
            }
            _ => {}
        }
        Browse::Nothing
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

    /// Moves the highlight to the track at `path` (the search filter is cleared to find it)
    pub fn select_path(&mut self, path: &str) -> bool {
        let at = |lib: &[Music]| lib.iter().position(|m| <&std::path::Path>::from(&m.path).to_string_lossy() == path);
        if at(&self.lib).is_none() {
            self.search.clear();
            self.refilter();
        }
        match at(&self.lib) {
            Some(i) => {
                self.sel = i;
                true
            }
            None => false,
        }
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
            // a search returns to browsing the (filtered) list, like / in vim
            KeyCode::Esc => {
                if searching {
                    self.search.clear();
                    self.refilter();
                    self.focus = Focus::Library;
                } else {
                    self.focus = Focus::Decks;
                }
            }
            KeyCode::Tab if searching => self.focus = Focus::Library,
            KeyCode::Enter => {
                if searching {
                    self.focus = Focus::Library;
                } else {
                    submitted = Some(self.input.value().trim().to_string());
                    self.focus = Focus::Decks;
                }
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
        return Err("no active set: :set new <name> makes one, :set <name> uses one".into());
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
    /// (text, shown until, made at)
    note: Option<(String, Instant, Instant)>,
    /// `:edit` asked for the editor on this file; main suspends the terminal and runs it
    edit_request: Option<std::path::PathBuf>,
    /// the in-TUI set editor, while open
    editor: Option<editor::Editor>,
    /// the editor set aside by a jump to another pane (F11, F12), its unsaved text kept
    parked_editor: Option<editor::Editor>,
    /// the AI prompt box and the agent run behind it
    prompt: Option<prompt::Prompt>,
    /// the conversation the prompt box continues (one per DJ session, `:ai new` resets it)
    ai_session: Option<prompt::Session>,
    /// `[agent]` from the config, for the prompt box
    agent: crate::parser::AgentSettings,
    /// the hardware knob while DJ mode is open (dropping it releases the device)
    knob: Option<crate::knob::Knob>,
    /// a knob press switched the knob from volume to the jog wheel
    knob_jog: bool,
    /// the `?` key map is up
    help: bool,
    /// how far the `?` key map is scrolled
    help_scroll: u16,
    /// the guided first mix, while open or paused
    tutor: Option<tutor::Tutor>,
    /// the tutor has moved the library highlight to its pair's first / second track
    tutor_sel: (bool, bool),
    /// where the tutor saves its progress (None: not at all)
    tutor_path: Option<std::path::PathBuf>,
    /// the patch pane (F12, `:patch`)
    patch: patch::PatchPane,
}

impl DjScreen {
    pub fn new(conn: Connection, library: Vec<Music>, release_events: bool) -> Self {
        let note = (!release_events).then(|| {
            (
                "No key-release events in this terminal: cue toggles, nudge is a short bend, holds step per key repeat. Use kitty/foot/WezTerm/Ghostty for hold."
                    .to_string(),
                Instant::now() + Duration::from_secs(12),
                Instant::now(),
            )
        });
        DjScreen {
            session: Session::start(conn),
            keys: DjKeys::new(release_events),
            pane: Pane::new(library),
            note,
            edit_request: None,
            editor: None,
            parked_editor: None,
            prompt: None,
            ai_session: None,
            agent: Default::default(),
            knob: None,
            knob_jog: false,
            help: false,
            help_scroll: 0,
            tutor: None,
            tutor_sel: (false, false),
            tutor_path: tutor::state_path(),
            patch: patch::PatchPane::default(),
        }
    }

    /// the search box or command line has the keyboard (text is typed, not performed)
    pub fn typing(&self) -> bool {
        self.pane.typing() || self.editor.is_some() || self.prompt.as_ref().is_some_and(|p| p.visible)
    }

    pub fn set_agent(&mut self, agent: crate::parser::AgentSettings) {
        self.agent = agent;
    }

    /// Takes over the hardware knob from the normal view, which gets it back with [`take_knob`]
    pub fn set_knob(&mut self, knob: Option<crate::knob::Knob>) {
        self.knob = knob;
    }

    /// `[knob] jog`: seconds per scrub detent
    pub fn set_jog_seconds(&mut self, seconds: Option<f64>) {
        self.keys.set_jog_seconds(seconds);
    }

    /// Hands the knob back when DJ mode closes
    pub fn take_knob(&mut self) -> Option<crate::knob::Knob> {
        self.knob.take()
    }

    /// What the knob did: turning edits the picked control, scrolls the library while browsing,
    /// else turns the volume, or with the jog wheel switched on (a press) is the focused deck's
    /// jog wheel (scrub when paused, bend when playing). Pressing resets the picked control,
    /// loads the highlighted track while browsing, else switches between volume and jog wheel.
    /// Notes show in the status line. Nothing acts while a text box or pane has the keyboard.
    fn poll_knob(&mut self) {
        use crate::knob::{KnobAction, Msg};
        let Some(knob) = self.knob.as_ref() else { return };
        for msg in knob.drain() {
            match msg {
                Msg::Note(n) => self.set_note(n),
                Msg::Action(..) if self.typing() => {}
                Msg::Action(KnobAction::Up, t) => self.knob_turn(1, t),
                Msg::Action(KnobAction::Down, t) => self.knob_turn(-1, t),
                Msg::Action(KnobAction::Press, t) => {
                    if !self.keys.press(t) {
                        if self.pane.browsing() {
                            self.load_smart();
                        } else {
                            self.knob_jog = !self.knob_jog;
                            self.set_note(if self.knob_jog {
                                "knob: jog wheel (scrubs a paused deck, bends a playing one) · press again for volume"
                            } else {
                                "knob: volume · press for the jog wheel"
                            });
                        }
                    }
                }
            }
        }
    }

    fn knob_turn(&mut self, dir: i32, now: Instant) {
        if self.keys.wheel(dir, now) {
            return;
        }
        if self.pane.browsing() {
            self.pane.scroll(-dir as isize);
        } else if self.knob_jog {
            self.keys.jog(dir, now);
        } else {
            // the knob you reach for without thinking: volume, as in the normal view
            self.session.send(vec![DjCall::VolumeStep(dir as f64 * 0.02)]);
        }
    }

    /// A transition is planned or running: a set's armed mix, or the player's or the DJ's own
    fn transition_active(&self) -> bool {
        let s = self.session.shared.lock().unwrap();
        s.set.as_ref().is_some_and(|set| set.armed >= 0) || s.up_next.as_ref().is_some_and(|u| !u.1.is_empty() && u.2 >= 0.)
    }

    /// `q` in the library: the highlighted track goes to the end of the player's queue
    fn queue_selected(&mut self) {
        let Some(m) = self.pane.selected().cloned() else {
            self.set_note("nothing selected in the library");
            return;
        };
        let conn = self.session.conn.clone();
        let index = m.index as u32;
        tokio::spawn(async move {
            if let Ok(p) = ServerProxy::new(&conn).await {
                let _ = p.queue_add(index, false).await;
            }
        });
        self.session.shared.lock().unwrap().queue.push(m.title.clone());
        self.set_note(format!("queued {} · automix mixes into it when its turn comes", m.title));
    }

    /// load the highlighted track onto the focused deck (`h` / `l`), unless that one is playing:
    /// then onto the other, so a load never cuts the track the room hears
    /// `gt`: mix from the other deck (on air) into the focused one (loaded, paused), the way the
    /// player's automix would
    /// `t{technique}` / `T{technique}`: the DJ's transition from the playing deck into the other
    /// one, bringing in the highlighted library track (loaded there first unless that deck holds
    /// it already, cued by hand or not)
    fn transition(&mut self, recipe: Option<&'static str>, bars: Option<u32>, outro: bool) {
        let decks = self.session.shared.lock().unwrap().decks.clone();
        let playing = |i: usize| decks.get(i).is_some_and(|d| d.playing);
        let (from, to) = match (playing(0), playing(1)) {
            (true, false) => (0u8, 1u8),
            (false, true) => (1, 0),
            (false, false) => return self.set_note("nothing is playing: Space starts the focused deck, then t brings in the next"),
            (true, true) => return self.set_note("both decks are playing: a transition needs one of them free"),
        };
        let held = decks.get(to as usize).map(|d| d.path.clone()).unwrap_or_default();
        let highlighted = self.pane.selected_path().unwrap_or_default();
        // what the free deck holds is the target when the highlight is on it (or there is none)
        let path = if highlighted.is_empty() || highlighted == held { String::new() } else { highlighted };
        if path.is_empty() && held.is_empty() {
            return self.set_note("highlight a track first (j/k or /), then t brings it in");
        }
        self.keys.set_focus(to);
        self.session.set_call(SetCall::Transition(from as u32, to as u32, recipe.unwrap_or("auto"), bars.unwrap_or(0), outro, path));
    }

    fn auto_mix(&mut self) {
        let to = self.keys.focus();
        let from = 1 - to;
        let decks = self.session.shared.lock().unwrap().decks.clone();
        let letter = |d: u8| if d == 0 { 'A' } else { 'B' };
        let (Some(f), Some(t)) = (decks.get(from as usize), decks.get(to as usize)) else { return };
        if !t.loaded {
            self.set_note(format!("gt mixes into the focused deck {}: load a track there first (Enter)", letter(to)));
        } else if t.playing {
            self.set_note(format!("deck {} is playing: gt mixes into a paused deck (focus it with h/l)", letter(to)));
        } else if !f.playing {
            self.set_note(format!("deck {} is not playing: gt mixes from the playing deck into the focused one", letter(from)));
        } else {
            self.session.set_call(SetCall::AutoMix(from as u32, to as u32));
        }
    }

    fn load_smart(&mut self) {
        let playing = |i: usize| self.session.shared.lock().unwrap().decks[i].playing;
        let focus = self.keys.focus();
        match smart_deck(focus, [playing(0), playing(1)]) {
            Some(deck) => {
                self.load(deck);
                if deck != focus {
                    let f = if focus == 0 { 'A' } else { 'B' };
                    self.set_note(format!("deck {f} is playing, so the track went onto the other deck"));
                }
            }
            None => self.set_note("both decks are playing: a or b loads over one of them"),
        }
    }

    /// `:e` / F10: the editor pane on the active set
    /// F10 / F11 / F12 jump straight between the editor, the AI box and the patch decks, from
    /// any of them (the one left is set aside: the editor keeps its unsaved text, a running agent
    /// keeps running); the key of the open pane still closes it, by that pane's own rules. F9
    /// records from anywhere. True when the key was taken here.
    fn pane_jump(&mut self, key: &KeyEvent, now: Instant) -> bool {
        if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
            return false;
        }
        let prompt_open = self.prompt.as_ref().is_some_and(|p| p.visible);
        let editor_open = self.editor.is_some();
        let in_pane = editor_open || prompt_open || self.patch.open;
        let target = match key.code {
            // recording is a deck thing, but it must not need closing a pane first
            KeyCode::F(9) if editor_open || prompt_open => {
                self.record(None);
                return true;
            }
            KeyCode::F(10) if !editor_open && in_pane => 10,
            KeyCode::F(11) if !prompt_open && in_pane => 11,
            KeyCode::F(12) if !self.patch.open && in_pane => 12,
            _ => return false,
        };
        // set the open pane aside
        if let Some(ed) = self.editor.take() {
            self.parked_editor = Some(ed);
        }
        if let Some(p) = self.prompt.as_mut() {
            p.visible = false;
        }
        if self.patch.open {
            self.patch.open = false;
        }
        match target {
            10 => self.open_editor(now),
            11 => self.open_prompt(None, now),
            _ => self.open_patch(now),
        }
        true
    }

    fn open_editor(&mut self, now: Instant) {
        if self.editor.is_some() {
            return;
        }
        // back to the editor set aside by a jump, with its text as it was
        if let Some(ed) = self.parked_editor.take() {
            self.settle(now);
            self.editor = Some(ed);
            return;
        }
        let Some((set, _)) = self.active_set() else { return };
        // the pane takes every key and no release will reach the deck keys: end holds first
        self.settle(now);
        self.editor = Some(editor::Editor::new(Some(self.session.conn.clone()), &set.name, &set.file));
    }

    /// F11 / `:ai [text]`: the prompt box, or back to a running or finished run
    fn open_prompt(&mut self, text: Option<String>, now: Instant) {
        self.settle(now);
        if let Some(p) = self.prompt.as_mut() {
            if p.busy() || p.phase == prompt::Phase::Review {
                p.visible = true;
                if text.is_some() {
                    self.set_note("an agent run is still going: Ctrl-C cancels it");
                }
                return;
            }
        }
        let mut p = prompt::Prompt::new(Some(self.session.conn.clone()), self.agent.clone());
        if let Some(s) = &self.ai_session {
            p.set_session(s.clone());
        }
        self.prompt = Some(p);
        if let Some(t) = text.filter(|t| !t.is_empty()) {
            self.submit_prompt(t);
        }
    }

    /// Builds the preamble from what the screen knows and starts the agent
    fn submit_prompt(&mut self, request: String) {
        let (set, decks, _) = self.set_snapshot();
        let (patches, _) = self.patch_slots();
        let live = self.agent.live.unwrap_or(false);
        let full = prompt::build_prompt(&request, set.as_ref().filter(|s| s.is_active()), &decks, &patches, live);
        // the prompts of one DJ session are one conversation with the agent (`:ai new` resets)
        let session = self.ai_session.get_or_insert_with(prompt::Session::new);
        if let Some(p) = self.prompt.as_mut() {
            p.set_session(session.clone());
            p.start(full, request.clone());
            session.turns += 1;
        }
    }

    /// `:help [term]`: the glossary entry in a popup, the list of terms, or the key map
    fn help_command(&mut self, query: &str) {
        let q = query.trim().to_lowercase();
        if matches!(q.as_str(), "keys" | "key map" | "keymap" | "map" | "?") {
            self.settle(Instant::now());
            self.help = true;
            return;
        }
        if q.is_empty() {
            self.session.shared.lock().unwrap().popup = Some(glossary::index_popup());
        } else if let Some(term) = glossary::lookup(&q) {
            self.session.shared.lock().unwrap().popup = Some(glossary::help_popup(term));
        } else {
            let near = glossary::suggest(&q);
            self.set_note(if near.is_empty() {
                format!("no help for {q:?}: :help lists the terms")
            } else {
                format!("no help for {q:?}; did you mean: {}", near.join(", "))
            });
        }
    }

    /// `:tutor` opens or resumes the guided mix, `:tutor reset` starts it over
    fn tutor_command(&mut self, reset: bool) {
        let now = Instant::now();
        let mut search = false;
        if reset {
            match self.tutor.as_mut() {
                Some(t) => t.restart(now),
                None => {
                    if let Some(p) = &self.tutor_path {
                        tutor::forget(p);
                    }
                    self.tutor = Some(tutor::Tutor::open(self.tutor_path.clone(), now));
                }
            }
            search = true;
            self.set_note("tutor restarted from the first step");
        } else {
            match self.tutor.as_mut() {
                Some(t) if t.visible => self.set_note("the tutor is open: :tutor next skips a step, Esc pauses it"),
                Some(t) => t.resume(now),
                None => {
                    self.tutor = Some(tutor::Tutor::open(self.tutor_path.clone(), now));
                    search = true;
                }
            }
        }
        self.tutor_sel = (false, false);
        let at_pick = self.tutor.as_ref().is_some_and(|t| t.step == tutor::Step::Pick);
        if search && at_pick {
            let library = self.pane.all.iter().map(|m| (<&std::path::Path>::from(&m.path).to_string_lossy().into_owned(), m.title.clone())).collect();
            // the library hides duplicate copies, so its positions aren't playlist indices
            let playlist_index = self.pane.all.iter().map(|m| m.index).collect();
            self.session.find_pair(library, playlist_index);
        }
    }

    /// The patch slots feed the hint machinery; with the pane open their sources are read
    fn tick_patch(&mut self) {
        let (slots, _) = self.patch_slots();
        self.session.shared.lock().unwrap().patch_fast = self.patch.open;
        if self.patch.open {
            let conn = self.session.conn.clone();
            self.patch.sync(&slots, |slot, name, tx| {
                let conn = conn.clone();
                tokio::spawn(async move {
                    let text = async {
                        let proxy = ServerProxy::new(&conn).await.map_err(|e| describe_patch_error("PatchSource", &e))?;
                        match tokio::time::timeout(CALL_TIMEOUT, proxy.patch_source(&name)).await {
                            Ok(Ok((text, _))) => Ok(text),
                            Ok(Err(e)) => Err(describe_patch_error("PatchSource", &e)),
                            Err(_) => Err("PatchSource: codjd does not answer".to_string()),
                        }
                    }
                    .await;
                    let _ = tx.send(patch::Fetched::Source { slot, name, text });
                });
            });
        }
        let mut defaults: [Vec<(String, f64)>; 4] = Default::default();
        for (i, d) in defaults.iter_mut().enumerate() {
            if let Some(m) = self.patch.model(i) {
                *d = m.knob_defaults.clone();
            }
        }
        self.keys.set_patches(&slots, defaults);
    }

    /// The tutor looks at the decks; at step 1 it moves the library highlight to the proposal
    fn tick_tutor(&mut self, decks: &[DeckInfo], mixer: &MixerInfo, now: Instant) {
        let finding = self.session.shared.lock().unwrap().finding.clone();
        let Some(t) = self.tutor.as_mut().filter(|t| t.visible) else { return };
        t.finding = finding;
        if t.update(decks, mixer, now).is_some() {
            self.tutor_sel = (false, false);
        }
        if t.step != tutor::Step::Pick || self.pane.typing() {
            return;
        }
        if let tutor::Finding::Found(p) = &t.finding {
            let a_loaded = decks.first().is_some_and(|d| d.loaded && d.path == p.a.path);
            if !self.tutor_sel.0 {
                self.tutor_sel.0 = true;
                self.pane.select_path(&p.a.path);
            } else if a_loaded && !self.tutor_sel.1 {
                self.tutor_sel.1 = true;
                self.pane.select_path(&p.b.path);
            }
        }
    }

    /// `:mix new [recipe [bars]]`: the tracks come from the decks, the rest runs in the set task
    fn mix_new(&mut self, recipe: Option<String>, bars: Option<u32>) {
        let Some(recipe) = recipe else {
            self.session.set_call(SetCall::Recipes);
            return;
        };
        let Some((set, _)) = self.active_set() else { return };
        let (decks, mixer) = {
            let s = self.session.shared.lock().unwrap();
            (s.decks.clone(), s.mixer.clone())
        };
        match mixnew::pick_pair(&decks, &mixer, &set) {
            Ok(pair) => self.session.set_call(SetCall::MixNew(MixNewReq { recipe, bars, set: set.name, pair })),
            Err(e) => self.set_note(e),
        }
    }

    /// the patch slots as last polled (four empty ones before the first answer)
    fn patch_slots(&self) -> (Vec<PatchSlot>, bool) {
        let s = self.session.shared.lock().unwrap();
        let mut slots = s.patches.clone().unwrap_or_default();
        slots.resize_with(patchdef::SLOTS, PatchSlot::default);
        (slots, s.patch_missing)
    }

    /// F12 / `:patch`: the patch pane takes the keyboard (the deck keys are off meanwhile)
    fn open_patch(&mut self, now: Instant) {
        if self.patch.open {
            return;
        }
        self.settle(now);
        let (slots, _) = self.patch_slots();
        if !slots.get(self.patch.sel).is_some_and(|s| s.loaded) {
            self.patch.sel = slots.iter().position(|s| s.loaded).unwrap_or(0);
        }
        self.patch.open();
    }

    /// What a key in the patch pane asked for
    fn patch_effects(&mut self, effects: Vec<patch::PEffect>) {
        use patch::PEffect;
        for e in effects {
            match e {
                PEffect::Action { slot, json } => self.session.patch_call(PatchCall::Action(slot, json)),
                PEffect::Undo(s) => self.session.patch_call(PatchCall::Undo(s)),
                PEffect::Redo(s) => self.session.patch_call(PatchCall::Redo(s)),
                PEffect::Live(s, on) => self.session.patch_call(PatchCall::Live(s, on)),
                PEffect::Note(n) => self.set_note(n),
                PEffect::Help => self.session.shared.lock().unwrap().popup = Some(glossary::patch_keys_popup()),
                PEffect::Close => self.patch.close(),
            }
        }
    }

    /// `:w [name]`: PatchSave of the selected slot, under its own name unless one is given
    fn patch_save(&mut self, slot: u32, name: Option<String>) {
        let (slots, _) = self.patch_slots();
        match slots.get(slot as usize).filter(|s| s.loaded) {
            Some(s) => {
                let name = name.unwrap_or_else(|| s.name.clone());
                self.session.patch_call(PatchCall::Save(slot, name));
            }
            None => self.set_note(format!("P{} is empty: nothing to save", slot + 1)),
        }
    }

    /// A key while the patch pane is open. None: the key belongs to the deck-key machinery (the
    /// hints `f ] [ \ ;`, the `:` line, Esc over a selection), which the pane keeps working.
    fn patch_key(&mut self, key: &KeyEvent, now: Instant) -> Option<Outcome> {
        if self.keys.hints().active {
            return None;
        }
        if key.kind == KeyEventKind::Release {
            return Some(Outcome::Stay);
        }
        let nk = keys::normalize(key);
        let plain = !nk.alt && !nk.ctrl;
        if nk.code == KeyCode::F(12) && !nk.alt {
            self.patch.close();
            return Some(Outcome::Stay);
        }
        let hint_key = plain
            && match nk.code {
                KeyCode::Char('f') => !nk.shift,
                // the pane has no use for these, and with nothing picked they would nudge a deck
                KeyCode::Char(']' | '[' | '\\') => !nk.shift && self.keys.hints().selected.is_some(),
                KeyCode::Char(';') => true,
                KeyCode::Esc => self.keys.hints().selected.is_some(),
                _ => false,
            };
        if hint_key {
            return None;
        }
        let (slots, _) = self.patch_slots();
        let effects = self.patch.on_key(&nk, &slots);
        self.patch_effects(effects);
        let _ = now;
        Some(Outcome::Stay)
    }

    /// `:patch ...`
    fn patch_command(&mut self, cmd: PatchCmd) {
        let (slots, missing) = self.patch_slots();
        if missing && !matches!(cmd, PatchCmd::Open | PatchCmd::Edit(_)) {
            self.set_note("codjd has no patch decks: update codjd");
            return;
        }
        let free = || slots.iter().position(|s| !s.loaded).map(|i| i as u32);
        match cmd {
            PatchCmd::Open => self.open_patch(Instant::now()),
            PatchCmd::List => self.session.patch_call(PatchCall::List),
            PatchCmd::Load(slot, name) => match slot.or_else(free) {
                Some(s) => self.session.patch_call(PatchCall::Load(s, name)),
                None => self.set_note("all four slots are in use: :patch load <slot> <name> replaces one"),
            },
            PatchCmd::Unload(s) => self.session.patch_call(PatchCall::Unload(s)),
            PatchCmd::Live(s, on) => {
                let now_live = slots.get(s as usize).is_some_and(|x| x.live);
                self.session.patch_call(PatchCall::Live(s, on.unwrap_or(!now_live)));
            }
            PatchCmd::New { name, kind } => self.session.patch_call(PatchCall::New { name, kind, slot: free() }),
            PatchCmd::Import { path, name } => self.session.patch_call(PatchCall::Import { path, name }),
            PatchCmd::Edit(name) => {
                self.settle(Instant::now());
                self.editor = Some(editor::Editor::new_patch(Some(self.session.conn.clone()), &name));
            }
        }
    }

    /// A note from the normal view (what the knob reader said before DJ mode opened)
    pub fn note_from_outside(&mut self, text: String) {
        self.set_note(text);
    }

    fn set_note(&mut self, text: impl Into<String>) {
        let now = Instant::now();
        self.note = Some((text.into(), now + NOTE_FOR, now));
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
                    "no active set: :set new <name> makes one, :set lists them"
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

    /// the deck to capture from: the one named, else the focused one (it needs a track)
    fn capture_deck(&mut self, named: Option<u8>) -> Option<u32> {
        let deck = named.unwrap_or(self.keys.focus());
        let loaded = self.session.shared.lock().unwrap().decks.get(deck as usize).is_some_and(|d| d.loaded);
        if loaded {
            Some(deck as u32)
        } else {
            self.set_note(format!("no track on deck {}: load one, or name the other deck (a or b)", deck_letter(deck as u32)));
            None
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
                // the deck you just loaded is the one you work on next, like vim's :e
                self.keys.set_focus(deck);
                self.set_note(format!(
                    "loading {name} onto deck {} · focused: Space plays, c cues, s syncs",
                    if deck == 0 { "A" } else { "B" }
                ));
            }
            _ => self.set_note("nothing selected in the library"),
        }
    }

    /// Handles a key; performance keys are queued for the next [`tick`]
    pub fn handle_key(&mut self, key: &KeyEvent, now: Instant) -> Outcome {
        if self.pane_jump(key, now) {
            return Outcome::Stay;
        }
        match editor::key_focus(self.editor.is_some(), self.prompt.as_ref().is_some_and(|p| p.visible)) {
            editor::KeyFocus::Editor => {
                if let Some(ed) = self.editor.as_mut() {
                    if ed.on_key(key, now) == editor::EditorOutcome::Close {
                        self.editor = None;
                    }
                }
                return Outcome::Stay;
            }
            editor::KeyFocus::Prompt => {
                let outcome = self.prompt.as_mut().map(|p| p.on_key(key));
                match outcome {
                    Some(prompt::PromptOutcome::Close) => self.prompt = None,
                    Some(prompt::PromptOutcome::Submit(text)) => self.submit_prompt(text),
                    _ => {}
                }
                return Outcome::Stay;
            }
            editor::KeyFocus::Deck => {}
        }
        // the key map scrolls with vim's keys when it does not fit; other keys close it
        if self.help && key.kind != KeyEventKind::Release {
            let max = help::lines(self.keys.focus()) as u16;
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let s = &mut self.help_scroll;
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => *s = (*s + 1).min(max),
                KeyCode::Char('k') | KeyCode::Up => *s = s.saturating_sub(1),
                KeyCode::Char('d') if ctrl => *s = (*s + 10).min(max),
                KeyCode::Char('u') if ctrl => *s = s.saturating_sub(10),
                KeyCode::PageDown => *s = (*s + 10).min(max),
                KeyCode::PageUp => *s = s.saturating_sub(10),
                KeyCode::Char('g') | KeyCode::Home => *s = 0,
                KeyCode::Char('G') | KeyCode::End => *s = max,
                _ => {
                    self.help = false;
                    self.help_scroll = 0;
                }
            }
            return Outcome::Stay;
        }
        // popups close on any key, which does nothing else, except `:`: it also opens the
        // command line, so the command a popup suggests can be typed straight away
        if key.kind != KeyEventKind::Release {
            let popup = self.session.shared.lock().unwrap().popup.take();
            if popup.is_some() && key.code != KeyCode::Char(':') {
                return Outcome::Stay;
            }
        }
        // Esc closes the error overlay first
        if key.code == KeyCode::Esc && key.kind != KeyEventKind::Release {
            if self.session.shared.lock().unwrap().overlay.take().is_some() {
                return Outcome::Stay;
            }
        }
        // while the tutor is open Esc pauses it (after a pending count or pick is dropped); Tab
        // browses the library as everywhere else, `:tutor next` skips a step
        if !self.pane.typing() && !self.pane.browsing() && key.kind == KeyEventKind::Press && !self.keys.hints().active {
            let selected = self.keys.hints().selected.is_some() || self.keys.has_pending();
            if let Some(t) = self.tutor.as_mut().filter(|t| t.visible) {
                match key.code {
                    KeyCode::Esc if !selected => {
                        t.close();
                        self.set_note("tutor paused: :tutor resumes where you left off");
                        return Outcome::Stay;
                    }
                    _ => {}
                }
            }
        }
        if self.patch.open && !self.pane.typing() {
            if let Some(outcome) = self.patch_key(key, now) {
                return outcome;
            }
        }
        // a half-typed `t` in the library goes on with the deck keys: its menu is up
        if self.pane.browsing() && !self.keys.has_pending() {
            let browsed = self.pane.browse_key(key);
            let pass = browsed == Browse::Pass;
            match browsed {
                Browse::Load(d) => self.load(d),
                Browse::LoadFree => self.load_smart(),
                Browse::Queue => self.queue_selected(),
                Browse::Help => self.help = true,
                // h l f t Space...: the decks get the key, with the library left behind
                Browse::Pass => {}
                Browse::Nothing => {}
            }
            if !pass {
                return Outcome::Stay;
            }
        }
        if self.pane.typing() {
            return match self.pane.handle_typing(key) {
                Some(cmd) => self.run_command(&cmd),
                None => Outcome::Stay,
            };
        }
        // `u` with a transition planned or running undoes the transition (like F8): the old
        // track plays on alone; with none it undoes the last control change
        // Ctrl-z is the same, for the hand that reaches for it in a panic
        let ctrl_z = key.code == KeyCode::Char('z') && key.modifiers == KeyModifiers::CONTROL;
        let plain_u = key.code == KeyCode::Char('u') && key.modifiers.is_empty();
        if (plain_u || ctrl_z) && key.kind == KeyEventKind::Press && !self.keys.has_pending() && self.keys.hints().selected.is_none() && self.transition_active() {
            self.session.set_call(SetCall::Disarm);
            return Outcome::Stay;
        }
        let as_u;
        let key = if ctrl_z {
            as_u = KeyEvent { code: KeyCode::Char('u'), modifiers: KeyModifiers::NONE, ..*key };
            &as_u
        } else {
            key
        };
        let Some(nav) = self.keys.on_key(key, now) else {
            return Outcome::Stay;
        };
        match nav {
            // typing switches key-release reporting off: end holds now, their release would
            // never arrive
            Nav::Command => {
                self.settle(now);
                self.pane.start_command()
            }
            Nav::Search => {
                self.settle(now);
                self.pane.start_browse()
            }
            // `/` in vim: straight into the search
            Nav::SearchNow => {
                self.settle(now);
                self.pane.start_search()
            }
            Nav::Note(text) => self.set_note(text),
            Nav::LibUp => self.pane.scroll(-1),
            Nav::LibDown => self.pane.scroll(1),
            Nav::LibPageUp => self.pane.scroll(-5),
            Nav::LibPageDown => self.pane.scroll(5),
            Nav::Editor => self.open_editor(now),
            Nav::Patch => self.open_patch(now),
            Nav::Prompt => self.open_prompt(None, now),
            Nav::SetArmNext => self.set_arm(ArmTarget::Next),
            Nav::SetAuto => self.set_auto(None),
            // F8: a set's armed mix, and a planned transition (t, gt) that has not started
            Nav::SetDisarm => self.session.set_call(SetCall::Disarm),
            Nav::RecordToggle => self.record(None),
            Nav::AutoMix => self.auto_mix(),
            Nav::Queue => self.queue_selected(),
            Nav::Transition { recipe, bars, outro } => self.transition(recipe, bars, outro),
            Nav::NoSelection => self.set_note("no control picked: f shows the hints, type a label to pick one"),
            Nav::Help => {
                self.settle(now);
                self.help = true;
            }
            // handled inside DjKeys, which owns the selection
            Nav::Hints | Nav::Toggle | Nav::Reset => {}
            Nav::LoadSmart => self.load_smart(),
            Nav::Load(d) => self.load(d),
        }
        Outcome::Stay
    }

    /// A wheel detent (or a knob turn, `up` = clockwise): edits the selected control, else
    /// scrolls the library. Ignored while the editor or the prompt box has the screen.
    pub fn wheel(&mut self, up: bool, now: Instant) {
        if self.editor.is_some() || self.prompt.as_ref().is_some_and(|p| p.visible) {
            return;
        }
        let dir = if up { 1 } else { -1 };
        if !self.keys.wheel(dir, now) {
            self.pane.scroll(-dir as isize);
        }
    }

    /// Ends all holds and sends whatever is pending, at once
    pub fn settle(&mut self, now: Instant) {
        self.keys.cancel_hints();
        self.keys.release_all();
        let calls = self.keys.tick(now);
        self.session.send(calls);
    }

    pub fn run_command(&mut self, cmd: &str) -> Outcome {
        match parse_command(cmd) {
            Command::Exit => return Outcome::Exit,
            Command::Automix(on) => self.session.set_call(SetCall::Automix(on)),
            Command::Autoplay(on) => self.session.set_call(SetCall::Autoplay(on)),
            Command::Split(on) => self.session.set_call(SetCall::Split(on)),
            Command::Load(d) => self.load(d),
            Command::Grid(d, f) => self.session.grid_scale(d, f),
            Command::SetList => self.session.set_call(SetCall::List),
            Command::SetUse(name) => self.session.set_call(SetCall::Use(name)),
            Command::SetNew(name) => self.session.set_call(SetCall::New(name)),
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
            Command::AudioRec(on) => {
                let (missing, recording) = {
                    let s = self.session.shared.lock().unwrap();
                    (s.audio_rec_missing, s.audio_rec.as_ref().is_some_and(|r| r.0))
                };
                if missing {
                    self.set_note("this codjd cannot record audio: update codjd");
                } else {
                    self.session.set_call(SetCall::AudioRec(on.unwrap_or(!recording)));
                }
            }
            Command::Editor => self.open_editor(Instant::now()),
            Command::Ai(text) => self.open_prompt(Some(text), Instant::now()),
            Command::AiNew => {
                if self.prompt.as_ref().is_some_and(|p| p.busy()) {
                    self.set_note("an agent run is still going: Ctrl-C cancels it first");
                } else {
                    self.ai_session = None;
                    self.prompt = None;
                    self.set_note("new AI conversation: :ai <request> or F11");
                }
            }
            Command::Help(query) => self.help_command(&query),
            Command::MixNew { recipe, bars } => self.mix_new(recipe, bars),
            Command::Tutor { reset } => self.tutor_command(reset),
            Command::TutorNext => match self.tutor.as_mut().filter(|t| t.visible) {
                Some(t) => {
                    t.skip(Instant::now());
                    self.tutor_sel = (false, false);
                }
                None => self.set_note("the tutor is not open: :tutor starts or resumes it"),
            },
            Command::Patch(cmd) => self.patch_command(cmd),
            Command::PatchSave(name) => {
                if self.patch.open {
                    self.patch_save(self.patch.sel as u32, name);
                } else {
                    self.set_note("no patch pane: F12 or :patch opens it (:w saves the selected slot's patch)");
                }
            }
            Command::Mix { ctl, deck, value } => {
                let d = deck.unwrap_or(self.keys.focus());
                let slot = match ctl {
                    MixCtl::Fader => keys::Slot::Ctl(keys::Ctl::Fader(d)),
                    MixCtl::Low => keys::Slot::Ctl(keys::Ctl::Eq(d, 0)),
                    MixCtl::Mid => keys::Slot::Ctl(keys::Ctl::Eq(d, 1)),
                    MixCtl::High => keys::Slot::Ctl(keys::Ctl::Eq(d, 2)),
                    MixCtl::Filter => keys::Slot::Ctl(keys::Ctl::Filter(d)),
                    MixCtl::Xfade => keys::Slot::Ctl(keys::Ctl::Crossfader),
                };
                self.keys.set_control(slot, value, Instant::now());
            }
            Command::DelMarks(slots) => {
                let deck = self.keys.focus() as u32;
                let calls = slots.into_iter().map(|slot| DjCall::Hotcue { deck, slot: slot as u32, action: "clear" }).collect();
                self.session.send(calls);
            }
            Command::Quantize(on) => {
                let now_on = self.session.shared.lock().unwrap().mixer.quantize;
                let value = if on.unwrap_or(!now_on) { 1.0 } else { 0.0 };
                self.session.send(vec![DjCall::Set { control: "quantize", deck: 0, value }]);
            }
            Command::Headphones(deck, on) => {
                let d = deck.unwrap_or(self.keys.focus());
                let now_on = self.session.shared.lock().unwrap().decks.get(d as usize).is_some_and(|i| i.headphones);
                let value = if on.unwrap_or(!now_on) { 1.0 } else { 0.0 };
                self.session.send(vec![DjCall::Set { control: "cue", deck: d as u32, value }]);
            }
            Command::Focus(d) => self.keys.set_focus(d),
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
        if let Some(ed) = self.editor.as_mut() {
            if ed.tick(now) == editor::EditorOutcome::Close {
                self.editor = None;
            }
        }
        if let Some(p) = self.prompt.as_mut() {
            p.tick();
        }
        self.poll_knob();
        self.tick_tutor(&decks, &mixer, now);
        self.keys.set_snapshot(&decks, &mixer);
        let starts = {
            let s = self.session.shared.lock().unwrap();
            [0, 1].map(|i| {
                let d = decks.get(i)?;
                let (path, g) = s.grids[i].as_ref()?;
                (*path == d.path && d.track_bpm > 0.0).then(|| g.first_beat_s + g.downbeat_offset as f64 * 60.0 / d.track_bpm)
            })
        };
        self.keys.set_starts(starts);
        self.tick_patch();
        let calls = self.keys.tick(now);
        for c in &calls {
            if let DjCall::PatchKnob { slot, .. } = c {
                self.patch.note_other_edit(*slot as usize);
            }
        }
        self.session.send(calls);
        self.session.ensure_waves();
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area: Rect = frame.area();
        // the scrolled key map shows the screen minus header, status and its border: past the
        // last full screen, j would scroll nothing and k would seem stuck
        let room = area.height.saturating_sub(4);
        self.help_scroll = self.help_scroll.min((help::lines(self.keys.focus()) as u16).saturating_sub(room));
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
        // the newer of the screen's note and a set command's feedback
        let now = Instant::now();
        let newest = [self.note.as_ref(), s.info.as_ref()]
            .into_iter()
            .flatten()
            .filter(|(_, until, _)| now < *until)
            .max_by_key(|(_, _, at)| *at);
        let note = newest.map(|(n, _, _)| n.as_str());
        // a failed call shows until a newer note replaces it (a typo's usage line after an
        // error must not stay hidden); a broken link always shows
        let noted_at = newest.map(|(_, _, at)| *at);
        let error = s
            .poll_error
            .clone()
            .or_else(|| s.set_poll_error.clone())
            .or_else(|| {
                s.call_error
                    .as_ref()
                    .filter(|(_, t)| t.elapsed() < NOTE_FOR && noted_at.is_none_or(|n| n < *t))
                    .map(|(e, _)| e.clone())
            });
        let labels = hints::table();
        let showcmd = self.keys.showcmd();
        let typed = self.keys.hints().typed.clone();
        // `t` waits for a technique: show what the letters are
        let t_menu = match self.keys.pending() {
            Some((keys::Pending::T { outro }, bars)) => Some(keys::technique_menu(outro, bars)),
            _ => None,
        };
        let chip = self
            .keys
            .hints()
            .selected
            .map(|t| format!("◆ {} {}", self.keys.target_name(t), t.format(self.keys.value_of(t, now))));
        // one line about the picked control, or about what the typed hint letters lead to
        let explain: Option<String> = if self.keys.hints().active {
            glossary::hover(&typed, &labels, self.keys.visible()).or_else(|| self.keys.hints().selected.map(|t| glossary::explain(t).to_string()))
        } else {
            self.keys.hints().selected.map(|t| glossary::explain(t).to_string()).or_else(|| {
                // a paused deck is one being cued: say how to move through it
                let f = self.keys.focus() as usize;
                s.decks.get(f).filter(|d| d.loaded && !d.playing).map(|_| scrub_help(f, self.keys.jog_seconds(), self.knob_jog))
            })
        };
        // the tutor's pane and its proposal marks in the library
        let grids = [0, 1].map(|i| s.grids[i].as_ref().filter(|(p, _)| *p == s.decks[i].path).map(|(_, g)| *g));
        let tutor_view = self.tutor.as_ref().filter(|t| t.visible).map(|t| t.view(&s.decks, &s.mixer, grids));
        let marks: Vec<(String, char)> = match self.tutor.as_ref() {
            Some(t) if t.visible && t.step == tutor::Step::Pick => match &t.finding {
                tutor::Finding::Found(p) => vec![(p.a.path.clone(), 'A'), (p.b.path.clone(), 'B')],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let prompt_visible = self.prompt.as_ref().is_some_and(|p| p.visible);
        let hidden_note = self.prompt.as_ref().filter(|p| !p.visible).and_then(|p| p.hidden_note());
        let note = note.or(hidden_note.as_deref());
        let panel = if self.editor.is_some() {
            Some(render::Panel::Editor)
        } else if prompt_visible {
            Some(render::Panel::Prompt)
        } else if self.patch.open {
            Some(render::Panel::Patch)
        } else {
            None
        };
        let mut patch_slots = s.patches.clone().unwrap_or_default();
        patch_slots.resize_with(patchdef::SLOTS, PatchSlot::default);
        let knob_vals: Vec<Vec<f64>> = patch_slots
            .iter()
            .enumerate()
            .map(|(i, p)| (0..p.knobs.len()).map(|k| self.keys.value_of(hints::Target::Knob(i as u8, k as u8), now)).collect())
            .collect();
        let mut mix_vals = [[0.0; 3]; patchdef::SLOTS];
        for (i, m) in mix_vals.iter_mut().enumerate() {
            let d = hints::slot_deck(i as u8);
            let get = |c: keys::Ctl| self.keys.value_of(hints::Target::Slot(keys::Slot::Ctl(c)), now);
            *m = [get(keys::Ctl::Fader(d)), get(keys::Ctl::Filter(d)), get(keys::Ctl::Eq(d, 0))];
        }
        let grid_view = self.patch.grid.as_ref().and_then(|g| {
            let seq = self.patch.seq()?;
            let slot = patch_slots.get(self.patch.sel)?;
            let live = slot.sequencers.iter().find(|q| q.0 == seq.name);
            Some(patchview::GridView {
                seq,
                seq_index: g.seq,
                seq_count: self.patch.model(self.patch.sel).map_or(1, |m| m.seqs.len()),
                pattern: g.pattern,
                cx: g.cx,
                cy: g.cy,
                playing: live.map(|q| q.1),
                queued: live.map(|q| q.2).filter(|q| *q != patch::NONE_QUEUED),
                step: live.map(|q| q.3),
            })
        });
        let patch_view = patchview::PatchView {
            slots: &patch_slots,
            missing: s.patch_missing,
            sel: self.patch.sel,
            knob_vals: &knob_vals,
            mix_vals,
            grid: grid_view,
        };
        let queue = s.queue.clone();
        let up_next = s.up_next.as_ref().and_then(crate::ui::automix_label);
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
            focus_deck: self.keys.focus(),
            showcmd: &showcmd,
            set: s.set.as_ref().filter(|x| x.is_active()),
            rec: s.rec.filter(|r| r.0).map(|r| (r.1, r.2)),
            queue: &queue,
            up_next: up_next.as_deref(),
            audio_rec: s.audio_rec.as_ref().filter(|r| r.0).map(|r| r.2),
            overlay: s.overlay.as_deref(),
            panel,
            hints: render::HintView {
                active: self.keys.hints().active,
                typed: &typed,
                labels: &labels,
                selected: self.keys.hints().selected,
                chip: chip.as_deref(),
                loop_len: self.keys.loop_len(),
                gain: [self.keys.gain_shown(0, now), self.keys.gain_shown(1, now)],
                sync_range: self.keys.sync_range(),
                jog: self.keys.jog_seconds(),
            },
            patch: Some(&patch_view),
            learn: render::LearnView {
                help: self.help,
                help_scroll: self.help_scroll,
                popup: s.popup.as_ref().map(|(t, l)| (t.as_str(), l.as_slice())).or(t_menu.as_ref().map(|(t, l)| (t.as_str(), l.as_slice()))),
                marks: &marks,
                explain: explain.as_deref(),
                warnings: s.warnings.len(),
                tutor: tutor_view.as_ref(),
            },
        };
        let spots = render::draw(frame.buffer_mut(), area, &view);
        self.keys.set_visible(spots.iter().map(|s| s.target).collect());
        if let Some(r) = render::panel_rect(area, &view) {
            if let Some(ed) = self.editor.as_mut() {
                ed.render(frame.buffer_mut(), r);
            } else if let Some(p) = self.prompt.as_ref().filter(|p| p.visible) {
                p.render(frame.buffer_mut(), r);
            }
        }
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
    fn enter_loads_onto_the_focused_deck_unless_it_plays() {
        assert_eq!(smart_deck(1, [false, false]), Some(1), "B focused, both idle: B");
        assert_eq!(smart_deck(0, [false, false]), Some(0));
        assert_eq!(smart_deck(1, [false, true]), Some(0), "B is on air: A");
        assert_eq!(smart_deck(0, [true, false]), Some(1));
        assert_eq!(smart_deck(0, [true, true]), None, "never cut what plays");
    }

    #[test]
    fn refusals_say_what_to_press() {
        let e = zbus::Error::Failure("other deck is not playing".into());
        assert!(describe_error("DjSync", &e).contains("Space it first"), "{}", describe_error("DjSync", &e));
        let e = zbus::Error::Failure("other deck has nothing loaded".into());
        assert!(describe_error("DjSync", &e).contains("a or b loads it"));
        // waiting for a quantized start: the daemon's own advice, no Space (it would pause)
        let e = zbus::Error::Failure("other deck is waiting for its beat (quantize): sync again in a moment".into());
        assert!(!describe_error("DjSync", &e).contains("Space"));
        let e = zbus::Error::Failure("bad amount".into());
        assert_eq!(describe_error("DjNudge", &e), format!("DjNudge: {e}"));
    }

    #[test]
    fn library_browses_with_vim_keys() {
        let lib: Vec<Music> = (0..30).map(|i| Music { title: format!("t{i}"), ..Default::default() }).collect();
        let mut p = Pane::new(lib);
        p.start_browse();
        let k = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        p.browse_key(&k(KeyCode::Char('j')));
        p.browse_key(&k(KeyCode::Char('j')));
        assert_eq!(p.sel, 2);
        p.browse_key(&k(KeyCode::Char('k')));
        assert_eq!(p.sel, 1);
        p.browse_key(&k(KeyCode::Char('G')));
        assert_eq!(p.sel, 29);
        p.browse_key(&k(KeyCode::Char('g')));
        p.browse_key(&k(KeyCode::Char('g')));
        assert_eq!(p.sel, 0);
        p.browse_key(&KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(p.sel, 10);
        assert_eq!(p.browse_key(&k(KeyCode::Char('a'))), Browse::Load(0));
        assert_eq!(p.browse_key(&k(KeyCode::Char('b'))), Browse::Load(1));
        assert_eq!(p.browse_key(&k(KeyCode::Enter)), Browse::LoadFree);
        p.browse_key(&k(KeyCode::Char('/')));
        assert_eq!(p.focus, Focus::Search);
        p.handle_typing(&k(KeyCode::Enter));
        assert_eq!(p.focus, Focus::Library, "a search returns to browsing");
        p.browse_key(&k(KeyCode::Esc));
        assert_eq!(p.focus, Focus::Decks);
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
        assert_eq!(parse_command("automix"), Command::Automix(None));
        assert_eq!(parse_command("autoplay on"), Command::Autoplay(Some(true)));
        assert_eq!(parse_command("automix off"), Command::Automix(Some(false)));
        assert!(matches!(parse_command("autoplay maybe"), Command::Unknown(_)));
        assert_eq!(parse_command("split"), Command::Split(None));
        assert_eq!(parse_command("split off"), Command::Split(Some(false)));
    }

    #[test]
    fn mixer_commands_set_controls_of_the_focused_deck() {
        let mix = |ctl, deck, value| Command::Mix { ctl, deck, value };
        assert_eq!(parse_command("fader 0.5"), mix(MixCtl::Fader, None, 0.5));
        assert_eq!(parse_command("fader b 0"), mix(MixCtl::Fader, Some(1), 0.0));
        assert_eq!(parse_command("fader off"), mix(MixCtl::Fader, None, 0.0));
        assert_eq!(parse_command("fader full"), mix(MixCtl::Fader, None, 1.0));
        assert_eq!(parse_command("low kill"), mix(MixCtl::Low, None, -1.0));
        assert_eq!(parse_command("low 0"), mix(MixCtl::Low, None, 0.0));
        assert_eq!(parse_command("low a flat"), mix(MixCtl::Low, Some(0), 0.0));
        assert_eq!(parse_command("mid -0.5"), mix(MixCtl::Mid, None, -0.5));
        assert_eq!(parse_command("high kill"), mix(MixCtl::High, None, -1.0));
        assert_eq!(parse_command("filter -0.3"), mix(MixCtl::Filter, None, -0.3));
        assert_eq!(parse_command("filter b reset"), mix(MixCtl::Filter, Some(1), 0.0));
        assert_eq!(parse_command("xfade -1"), mix(MixCtl::Xfade, None, -1.0));
        assert_eq!(parse_command("xfade a"), mix(MixCtl::Xfade, None, -1.0));
        assert_eq!(parse_command("xfade b"), mix(MixCtl::Xfade, None, 1.0));
        assert_eq!(parse_command("xfade center"), mix(MixCtl::Xfade, None, 0.0));
        assert_eq!(parse_command("xfade 0.25"), mix(MixCtl::Xfade, None, 0.25));
        // out of range, missing or too many values are usage notes, nothing is sent
        for bad in ["fader", "fader 2", "fader -0.1", "low 1.5", "filter", "filter x", "xfade 3", "xfade a b", "low kill kill", "fader a"] {
            assert!(matches!(parse_command(bad), Command::Unknown(_)), "{bad}");
        }
        assert!(matches!(parse_command("low"), Command::Unknown(m) if m.contains("usage: low")));
    }

    #[test]
    fn delm_clears_marks_like_vims_delmarks() {
        assert_eq!(parse_command("delm a"), Command::DelMarks(vec![0]));
        assert_eq!(parse_command("delm a c"), Command::DelMarks(vec![0, 2]));
        assert_eq!(parse_command("delm abc"), Command::DelMarks(vec![0, 1, 2]));
        assert_eq!(parse_command("delm b-d"), Command::DelMarks(vec![1, 2, 3]));
        assert_eq!(parse_command("delmarks h a a"), Command::DelMarks(vec![0, 7]));
        assert_eq!(parse_command("delm!"), Command::DelMarks((0..8).collect()));
        assert_eq!(parse_command("delmarks!"), Command::DelMarks((0..8).collect()));
        for bad in ["delm", "delm z", "delm i", "delm d-b", "delm! a", "delm 1"] {
            assert!(matches!(parse_command(bad), Command::Unknown(_)), "{bad}");
        }
    }

    #[test]
    fn small_commands_and_leaving_only_by_q() {
        assert_eq!(parse_command("quantize"), Command::Quantize(None));
        assert_eq!(parse_command("quant off"), Command::Quantize(Some(false)));
        assert_eq!(parse_command("hp"), Command::Headphones(None, None));
        assert_eq!(parse_command("hp b"), Command::Headphones(Some(1), None));
        assert_eq!(parse_command("hp a off"), Command::Headphones(Some(0), Some(false)));
        assert_eq!(parse_command("focus b"), Command::Focus(1));
        assert!(matches!(parse_command("focus"), Command::Unknown(_)));
        for q in ["q", "q!", "quit", "wq", "x"] {
            assert_eq!(parse_command(q), Command::Exit, "{q}");
        }
    }

    #[test]
    fn help_commands() {
        assert_eq!(parse_command("help"), Command::Help(String::new()));
        assert_eq!(parse_command("help bass swap"), Command::Help("bass swap".into()));
        assert_eq!(parse_command("h  Hot  Cue"), Command::Help("Hot Cue".into()));
        assert_eq!(parse_command("?"), Command::Help(String::new()));
        // every term of the glossary is reachable through the command
        for term in glossary::TERMS {
            let Command::Help(q) = parse_command(&format!("help {}", term.name)) else { panic!() };
            assert_eq!(glossary::lookup(&q).map(|t| t.name), Some(term.name));
        }
    }

    #[test]
    fn mix_new_command_and_error_overlay_lines() {
        assert_eq!(parse_command("mix new"), Command::MixNew { recipe: None, bars: None });
        assert_eq!(parse_command("mix new blend 32"), Command::MixNew { recipe: Some("blend".into()), bars: Some(32) });
        assert!(matches!(parse_command("mix"), Command::Unknown(_)));
        assert_eq!(
            errors_with_warnings(vec!["a.codj:1:1: bad".into()], vec!["a.codj:5:1: short".into()]),
            vec!["a.codj:1:1: bad".to_string(), "warning: a.codj:5:1: short".to_string()]
        );
        assert_eq!(errors_with_warnings(vec![], vec![]), Vec::<String>::new());
    }

    #[test]
    fn patch_commands() {
        use PatchCmd::*;
        let p = |c: &str| parse_command(c);
        assert_eq!(p("patch"), Command::Patch(Open));
        assert_eq!(p("patch list"), Command::Patch(List));
        // slots are numbered from 1 on screen, 0-based on the bus
        assert_eq!(p("patch load 2 d1"), Command::Patch(Load(Some(1), "d1".into())));
        assert_eq!(p("patch load d1"), Command::Patch(Load(None, "d1".into())));
        assert_eq!(p("patch unload 4"), Command::Patch(Unload(3)));
        assert_eq!(p("patch live 1 on"), Command::Patch(Live(0, Some(true))));
        assert_eq!(p("patch live 3 off"), Command::Patch(Live(2, Some(false))));
        assert_eq!(p("patch live 3"), Command::Patch(Live(2, None)));
        assert_eq!(p("patch new d1"), Command::Patch(New { name: "d1".into(), kind: "drums".into() }));
        assert_eq!(p("patch new bass acid"), Command::Patch(New { name: "bass".into(), kind: "acid".into() }));
        assert_eq!(p("patch new pad1 pad"), Command::Patch(New { name: "pad1".into(), kind: "pad".into() }));
        assert_eq!(p("patch import /m/x.ncft lead"), Command::Patch(Import { path: "/m/x.ncft".into(), name: "lead".into() }));
        assert_eq!(p("patch import /m/My Song.ncft"), Command::Unknown("usage: patch import <file.ncft> [name] (names use letters, digits, - and _)".into()), "extra words are not a name");
        assert_eq!(p("patch import /m/my-song.ncft"), Command::Patch(Import { path: "/m/my-song.ncft".into(), name: "my-song".into() }));
        assert_eq!(p("patch edit d1"), Command::Patch(Edit("d1".into())));
        assert_eq!(p("patches"), Command::Patch(Open));
        for bad in [
            "patch load", "patch load 5 d1", "patch load 0 d1", "patch load 1 ../x", "patch load 1 a b", "patch unload", "patch unload 9",
            "patch live", "patch live 1 maybe", "patch new", "patch new ../x", "patch new d1 polka", "patch new d1 acid extra",
            "patch import", "patch edit", "patch edit a/b", "patch frob",
        ] {
            assert!(matches!(p(bad), Command::Unknown(_)), "{bad}: {:?}", p(bad));
        }
        // :w saves the selected slot's patch
        assert_eq!(p("w"), Command::PatchSave(None));
        assert_eq!(p("write acid2"), Command::PatchSave(Some("acid2".into())));
        assert!(matches!(p("w ../x"), Command::Unknown(_)));
        // the unknown-command help names it
        let Command::Unknown(msg) = p("frobnicate") else { panic!() };
        assert!(msg.contains("patch"));
        // `:q` and friends are untouched
        assert_eq!(p("q"), Command::Exit);
        assert_eq!(p("wq"), Command::Exit);
    }

    #[test]
    fn patch_calls_say_what_happened() {
        assert_eq!(PatchCall::Load(0, "d1".into()).done(""), "d1 loaded into P1: headphones only, :patch live 1 on sends it to the room");
        assert_eq!(PatchCall::Live(1, true).done(""), "P2 is LIVE: the room hears it");
        assert_eq!(PatchCall::Live(1, false).done(""), "P2 back to headphones only");
        assert_eq!(PatchCall::Unload(3).done(""), "P4 unloaded");
        assert_eq!(PatchCall::Save(0, "x".into()).done(" abc"), "P1 saved as x");
        assert_eq!(PatchCall::Import { path: "p".into(), name: "n".into() }.done(" abc123 (dropped MidiIn, Scope)"), "imported n: dropped MidiIn, Scope");
        assert_eq!(PatchCall::Import { path: "p".into(), name: "n".into() }.done(" abc123"), "imported n");
        assert!(PatchCall::Action(0, "{}".into()).done("").is_empty(), "cell edits are silent");
        assert_eq!(PatchCall::New { name: "n".into(), kind: "drums".into(), slot: None }.name(), "PatchWrite");
        assert!(format_patch_list(&[]).contains(":patch new"));
        let text = format_patch_list(&[("acid".into(), "acid line".into()), ("pad".into(), "pad".into())]);
        assert!(text.contains("acid “acid line”") && text.contains(" pad ") || text.contains("· pad"), "{text}");
        let e = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.UnknownMethod").unwrap(),
            Some("Unknown method 'PatchState'".into()),
            zbus::message::Message::method_call("/", "x").unwrap().build(&()).unwrap(),
        );
        assert_eq!(describe_patch_error("PatchLoad", &e), "codjd has no patch decks: update codjd");
    }

    #[test]
    fn tutor_command_and_highlighting_a_track() {
        assert_eq!(parse_command("tutor"), Command::Tutor { reset: false });
        assert_eq!(parse_command("tutor reset"), Command::Tutor { reset: true });
        assert_eq!(parse_command("tutor next"), Command::TutorNext);
        assert_eq!(parse_command("ai new"), Command::AiNew);
        assert_eq!(parse_command("ai new tracks for a warm-up"), Command::Ai("new tracks for a warm-up".into()));
        assert_eq!(parse_command("set new friday"), Command::SetNew("friday".into()));
        assert!(matches!(parse_command("set new"), Command::Unknown(m) if m.contains("usage: set new")));
        assert!(matches!(parse_command("set new two words"), Command::Unknown(_)));
        assert_eq!(parse_command("set newer"), Command::SetUse("newer".into()));
        assert!(new_set_text("friday").contains("set \"friday\""));
        assert_eq!(parse_command("tutor skip"), Command::TutorNext);
        assert!(matches!(parse_command("tutor now"), Command::Unknown(_)));
        assert!(matches!(parse_command("tutor reset now"), Command::Unknown(_)));
        let mut p = pane();
        assert!(p.select_path("/m/Hot minute.flac"));
        assert_eq!(p.selected().unwrap().title, "Hot minute");
        // a search that hides the track is cleared to find it
        p.start_search();
        type_str(&mut p, "smokin");
        p.handle_typing(&key(KeyCode::Enter));
        assert_eq!(p.lib.len(), 1);
        assert!(p.select_path("/m/Aftertekno.flac"));
        assert_eq!(p.lib.len(), 3);
        assert_eq!(p.selected().unwrap().title, "Aftertekno");
        assert!(!p.select_path("/m/none.flac"));
    }

    #[test]
    fn capture_commands_and_replies() {
        assert_eq!(parse_command("cue drop"), Command::Cue("drop".into(), None));
        assert_eq!(parse_command("cue drop b"), Command::Cue("drop".into(), Some(1)));
        assert_eq!(parse_command("loopsave brk a"), Command::LoopSave("brk".into(), Some(0)));
        assert_eq!(parse_command("gridsave"), Command::GridSave(None));
        assert_eq!(parse_command("rec"), Command::Record(None));
        assert_eq!(parse_command("rec off"), Command::Record(Some(false)));
        assert_eq!(parse_command("rec audio"), Command::AudioRec(None));
        assert_eq!(parse_command("rec audio off"), Command::AudioRec(Some(false)));
        assert!(matches!(parse_command("rec audio loud"), Command::Unknown(_)));
        assert_eq!(home_short("/nowhere/x.wav"), "/nowhere/x.wav");
        assert!(matches!(parse_command("cue"), Command::Unknown(_)));
        assert_eq!(SetCall::Record(false).done_with("mix 2: aft -> smk, 16 bars, 9 items"), "recorded mix 3: aft → smk, 16 bars, 9 items");
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

/// The status line while the focused deck is paused: how to find a spot in it
fn scrub_help(deck: usize, step: f64, knob_jog: bool) -> String {
    let knob = if knob_jog { "knob turns scrub" } else { "knob press: scrub with it" };
    format!(
        "deck {} paused: [ ] scrub {step} s (hold: faster) · g[ g] step · W B a bar · {{ }} 16 bars · {knob} · Space plays",
        if deck == 0 { 'A' } else { 'B' }
    )
}
