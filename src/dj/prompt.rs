//! The AI prompt box (`:ai <text>`, F11): runs the agent command from `[agent]` in the config
//! as a child process, streams its output into a pane and, when it exits, shows what it changed
//! in the set files as a diff with keep (`y`) / undo (`n`).
//!
//! The agent edits sets only through codj's own MCP server (`codj mcp`), attached through
//! `--mcp-config` (for `claude`) or the `CODJ_MCP_CONFIG` environment variable (any other
//! command). Nothing here blocks the UI: the child, its pipes and every D-Bus call live in
//! tasks and report back through a channel that [`Prompt::tick`] drains.
//!
//! Pure parts (command building, context preamble, change detection, undo plan, output
//! cleaning) are plain functions with tests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tui_input::{Input, InputRequest};
use zbus::Connection;

use super::editor::{diff_stat, diff_style, line_diff, unified_diff, AUTHOR};
use crate::dj_types::{DeckInfo, PatchSlot, SetState};
use crate::parser::AgentSettings;
use crate::ServerProxy;

/// the agent when `[agent] command` is not set
pub const DEFAULT_COMMAND: &str = "claude -p";
/// name of codj's MCP server in the config handed to the agent (tools are `mcp__codj__<tool>`)
pub const MCP_NAME: &str = "codj";
/// set tools that never change what is playing
pub const SET_TOOLS: [&str; 9] =
    ["sets_list", "set_source", "set_check", "set_write", "set_use", "set_cue", "set_loop_capture", "set_grid_capture", "set_record"];
/// set tools that arm or load things: refused by `codj mcp` without `--live`
pub const LIVE_TOOLS: [&str; 5] = ["set_arm", "set_disarm", "set_auto", "set_preview", "set_load"];
/// patch tools that never need `--live` (files, state, node catalog, edits of slots not on air)
pub const PATCH_TOOLS: [&str; 8] =
    ["patch_list", "patch_source", "patch_check", "patch_write", "patch_import", "patch_action", "patch_state", "patch_node_types"];
/// patch tools refused by `codj mcp` without `--live`
pub const PATCH_LIVE_TOOLS: [&str; 3] = ["patch_load", "patch_unload", "patch_live"];
/// note shown when an undo is written (the D-Bus call has no note field, the author is `human`)
pub const UNDO_NOTE: &str = "undo agent change";
const MAX_OUT_LINES: usize = 2000;
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------- building the command

/// Splits a command line on spaces; "double" and 'single' quotes group, backslash escapes
pub fn split_command(s: &str) -> Vec<String> {
    let (mut words, mut cur, mut quote, mut any) = (Vec::new(), String::new(), None::<char>, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', _) => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    any = true;
                }
            }
            (q, Some(open)) if q == open => quote = None,
            (q @ ('"' | '\''), None) => {
                quote = Some(q);
                any = true;
            }
            (c, None) if c.is_whitespace() => {
                if any {
                    words.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (c, _) => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(cur);
    }
    words
}

/// The config file handed to the agent: codj's MCP server, `--live` only when allowed
pub fn mcp_config_json(exe: &Path, live: bool) -> String {
    let mut args = vec![serde_json::Value::from("mcp")];
    if live {
        args.push("--live".into());
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": { MCP_NAME: { "command": exe.to_string_lossy(), "args": args } }
    }))
    .unwrap_or_default()
}

/// Full names (`mcp__codj__set_check`) of the MCP tools the agent may call: `[agent] mcp_tools`
/// when set, else every set and patch tool, plus the live ones when live
pub fn allowed_tools(custom: Option<&[String]>, live: bool) -> Vec<String> {
    let full = |t: &str| if t.starts_with("mcp__") { t.to_string() } else { format!("mcp__{MCP_NAME}__{t}") };
    match custom {
        Some(list) => list.iter().map(|t| full(t)).collect(),
        None => SET_TOOLS
            .iter()
            .chain(PATCH_TOOLS.iter())
            .chain(if live { LIVE_TOOLS.iter().chain(PATCH_LIVE_TOOLS.iter()) } else { [].iter().chain([].iter()) })
            .map(|t| full(t))
            .collect(),
    }
}

/// How the prompt reaches the command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptVia {
    Stdin,
    Arg,
}

impl PromptVia {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.map(str::trim) {
            None | Some("") | Some("stdin") => Ok(PromptVia::Stdin),
            Some("arg") => Ok(PromptVia::Arg),
            Some(other) => Err(format!("[agent] prompt_via = \"{other}\": use \"stdin\" or \"arg\"")),
        }
    }
}

/// A command ready to spawn
#[derive(Debug, Clone, PartialEq)]
pub struct AgentCmd {
    pub program: String,
    pub args: Vec<String>,
    /// written to the child's stdin (then closed); None = stdin is /dev/null
    pub stdin: Option<String>,
    pub env: Vec<(String, String)>,
    /// contents of the MCP config file, to be written at `mcp_path`
    pub mcp_json: String,
    pub mcp_path: PathBuf,
    /// stdout is stream-json event lines, shown as text, tool calls and results
    pub stream_json: bool,
}

impl AgentCmd {
    /// the command line as one string, for the pane's first line (the prompt itself is cut)
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        for a in &self.args {
            parts.push(if a.len() > 60 || a.contains('\n') { format!("<{} bytes of prompt>", a.len()) } else if a.contains(' ') || a.is_empty() { format!("\"{a}\"") } else { a.clone() });
        }
        parts.join(" ")
    }
}

/// The conversation the prompt box keeps with `claude` across prompts (`:ai new` starts another)
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// a v4 UUID, as `claude --session-id` wants it
    pub id: String,
    /// prompts sent in it so far
    pub turns: u32,
}

impl Session {
    pub fn new() -> Session {
        Session { id: uuid_v4(), turns: 0 }
    }
}

/// A random version 4 UUID from the kernel's random source
fn uuid_v4() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // no /dev/urandom: time and pid are unique enough for a local session name
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        b[..16].copy_from_slice(&(t ^ ((std::process::id() as u128) << 64)).to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// True for `claude` (or a path ending in it): it gets the MCP flags
pub fn is_claude(program: &str) -> bool {
    Path::new(program).file_name().is_some_and(|n| n == "claude")
}

/// Builds the child process for `prompt` (the preamble is already part of it).
///
/// For `claude` the flags are `--mcp-config <file> --strict-mcp-config --tools "" --allowedTools
/// <mcp tools>`, so the only things it can do are codj's MCP tools. Any other command gets
/// `CODJ_MCP_CONFIG`, `CODJ_MCP_LIVE` and `CODJ_MCP_TOOLS` in its environment instead (and the
/// same file), which a wrapper script can pass on to its agent.
pub fn build_agent_cmd(cfg: &AgentSettings, exe: &Path, mcp_path: &Path, prompt: &str, session: Option<&Session>) -> Result<AgentCmd, String> {
    let line = cfg.command.as_deref().map(str::trim).filter(|c| !c.is_empty()).unwrap_or(DEFAULT_COMMAND);
    let mut words = split_command(line);
    if words.is_empty() {
        return Err("[agent] command is empty".into());
    }
    let via = PromptVia::parse(cfg.prompt_via.as_deref())?;
    let live = cfg.live.unwrap_or(false);
    let tools = allowed_tools(cfg.mcp_tools.as_deref(), live);
    let program = words.remove(0);
    let mut args = words;
    if is_claude(&program) {
        args.extend([
            "--mcp-config".to_string(),
            mcp_path.to_string_lossy().into_owned(),
            "--strict-mcp-config".into(),
            "--tools".into(),
            String::new(),
            "--allowedTools".into(),
            tools.join(","),
        ]);
        // the agent's work as events (text, each tool call and its result), not just its last words
        if cfg.output.as_deref().is_none_or(|o| o == "stream-json") {
            args.extend(["--output-format".to_string(), "stream-json".into(), "--verbose".into()]);
        }
        // one conversation across prompts: the first names the session, the rest resume it
        if let Some(sess) = session {
            args.extend([if sess.turns == 0 { "--session-id" } else { "--resume" }.to_string(), sess.id.clone()]);
        }
    }
    let stdin = match via {
        PromptVia::Stdin => Some(prompt.to_string()),
        PromptVia::Arg => {
            if is_claude(&program) {
                // the flags above take lists: `--` ends them so the prompt stays the prompt
                args.push("--".into());
            }
            args.push(prompt.to_string());
            None
        }
    };
    let env = vec![
        ("CODJ_MCP_CONFIG".to_string(), mcp_path.to_string_lossy().into_owned()),
        ("CODJ_MCP_LIVE".to_string(), if live { "1" } else { "0" }.to_string()),
        ("CODJ_MCP_TOOLS".to_string(), tools.join(",")),
    ];
    let stream_json = match cfg.output.as_deref() {
        Some("stream-json") => true,
        Some("text") => false,
        Some(other) => return Err(format!("[agent] output = \"{other}\": use \"stream-json\" or \"text\"")),
        None => is_claude(&program),
    };
    Ok(AgentCmd { program, args, stdin, env, mcp_json: mcp_config_json(exe, live), mcp_path: mcp_path.to_path_buf(), stream_json })
}

// ---------------------------------------------------------------- the preamble

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn deck_line(name: &str, d: &DeckInfo) -> String {
    if !d.loaded {
        return format!("Deck {name}: empty");
    }
    let bpm = if d.bpm > 0.0 { format!("{:.1} BPM", d.bpm) } else { "no grid".into() };
    let key = if d.key.is_empty() { "key unknown".to_string() } else { format!("key {}", d.key) };
    format!(
        "Deck {name}: \"{}\" by {}, {bpm}, {key}, {} / {}, {}",
        d.title,
        if d.artist.is_empty() { "unknown artist" } else { &d.artist },
        clock(d.position),
        clock(d.duration),
        if d.playing { "playing" } else { "paused" }
    )
}

/// One line per patch slot for the preamble
fn patch_line(i: usize, s: &PatchSlot) -> String {
    if !s.loaded {
        return format!("P{}: empty", i + 1);
    }
    let knobs: Vec<String> = s.knobs.iter().map(|k| format!("{} {:.2}", k.0, k.1)).collect();
    let seqs: Vec<String> = s
        .sequencers
        .iter()
        .map(|q| format!("{} pattern {}{}", q.0, super::patch::pattern_letter(q.1 as usize), if q.2 == super::patch::NONE_QUEUED { String::new() } else { format!(" (next: {})", super::patch::pattern_letter(q.2 as usize)) }))
        .collect();
    format!(
        "P{}: {}{}, {}{}{}{}{}",
        i + 1,
        s.name,
        if s.dirty { " (unsaved edits)" } else { "" },
        if s.live { "LIVE (the audience hears it)" } else { "cue-only (headphones)" },
        if s.muted_reason.is_empty() { String::new() } else { format!(", MUTED: {}", s.muted_reason) },
        if knobs.is_empty() { String::new() } else { format!("; knobs: {}", knobs.join(", ")) },
        if seqs.is_empty() { String::new() } else { format!("; sequencers: {}", seqs.join(", ")) },
        if s.cpu >= 0.1 { format!("; cpu {:.0}%", s.cpu * 100.0) } else { String::new() },
    )
}

/// What the agent is told about the set, the decks and the patch slots, plus the rules, then the request
pub fn build_prompt(request: &str, set: Option<&SetState>, decks: &[DeckInfo], patches: &[PatchSlot], live: bool) -> String {
    let mut t = String::new();
    t.push_str("You are helping a DJ who is performing right now with codj, a terminal DJ app that talks to the codjd audio daemon. The DJ edits \"sets\" (text files: tracks, cues and the transitions between them, see the set language) and asked you to change one.\n\n");
    match set {
        Some(s) => {
            t.push_str(&format!("Active set: {}{}\n", s.name, if s.file.is_empty() { String::new() } else { format!(" (file {})", s.file) }));
            if s.errors > 0 {
                t.push_str(&format!("It currently has {} error(s).\n", s.errors));
            }
            t.push_str("Tracks (id: title):\n");
            for (id, title, _) in &s.tracks {
                t.push_str(&format!("  {id}: {title}\n"));
            }
            t.push_str("Mixes (from -> to, start beat, length in beats):\n");
            for (from, to, start, len) in &s.mixes {
                t.push_str(&format!("  {from} -> {to}, beat {start}, {len} beats ({} bars)\n", len / 4.0));
            }
        }
        None => t.push_str("No set is active (sets_list shows the available ones).\n"),
    }
    t.push_str("\nOn the decks right now:\n");
    for (i, name) in ["A", "B"].iter().enumerate() {
        t.push_str(&format!("  {}\n", decks.get(i).map(|d| deck_line(name, d)).unwrap_or_else(|| format!("Deck {name}: empty"))));
    }
    t.push_str("\nPatch decks (synths and drum machines that play in time with the decks; four slots P1-P4, slot numbers 0-3 in the tools):\n");
    for i in 0..4 {
        t.push_str(&format!("  {}\n", patches.get(i).map(|s| patch_line(i, s)).unwrap_or_else(|| format!("P{}: empty", i + 1))));
    }
    t.push_str("\nRules:\n");
    t.push_str("- Edit sets only through the codj MCP set tools (sets_list, set_source, set_check, set_write). Never write a set file any other way.\n");
    t.push_str("- Always call set_check on the complete new text and fix every error before set_write; pass the base_hash that set_source returned. If set_write says the set changed since you read it, read it again and redo your edit on the new text.\n");
    t.push_str("- Prefer replacing one track or mix block over rewriting the whole file, so the diff stays small. Keep everything the request does not mention exactly as it is.\n");
    t.push_str("- Patches are JSON files edited only through the patch tools: patch_node_types (the node catalog) -> write the JSON -> patch_check until it is valid -> patch_write with the base_hash of patch_source (\"\" for a new patch). patch_action turns knobs and sequencer cells of a loaded slot (names in patch_state); a slot marked cue-only is safe to tweak, one marked LIVE is what the audience hears right now, so only change it if the request asks for that.\n");
    if live {
        t.push_str("- Live mode is on: the arm/preview/load tools and patch_load/patch_unload/patch_live work and change what the audience hears. Use them only if the request asks for it, say what you did, and prefer patch_load into a free slot and a check in cue-only before patch_live.\n");
    } else {
        t.push_str("- Never touch audio: do not arm, preview, load or change decks or the mixer, and do not load, unload or send patch slots live. The audio-affecting tools are refused in this session anyway.\n");
    }
    t.push_str("- Answer in a few short lines: what you changed and why.\n");
    t.push_str("\nRequest:\n");
    t.push_str(request.trim());
    t.push('\n');
    t
}

// ---------------------------------------------------------------- what the agent changed

/// set name -> (text, hash)
pub type Snapshot = BTreeMap<String, (String, String)>;

#[derive(Debug, Clone, PartialEq)]
pub struct SetChange {
    pub name: String,
    /// None: the agent created the set
    pub old: Option<(String, String)>,
    pub new: (String, String),
}

/// The sets whose hash differs between the two snapshots (or that did not exist before)
pub fn changed_sets(before: &Snapshot, after: &Snapshot) -> Vec<SetChange> {
    after
        .iter()
        .filter(|(name, new)| before.get(*name).map_or(true, |old| old.1 != new.1))
        .map(|(name, new)| SetChange { name: name.clone(), old: before.get(name).cloned(), new: new.clone() })
        .collect()
}

/// One `SetWrite` that puts a set back
#[derive(Debug, Clone, PartialEq)]
pub struct UndoOp {
    pub name: String,
    pub text: String,
    /// the hash the set had after the agent: if someone saved since, the write is refused
    pub base_hash: String,
}

/// What to write for a keep / undo decision: nothing for keep; for undo the old text of every
/// changed set that existed before (a set the agent created has no old text to put back)
pub fn undo_ops(changes: &[SetChange], keep: bool) -> Vec<UndoOp> {
    if keep {
        return Vec::new();
    }
    changes
        .iter()
        .filter_map(|c| c.old.as_ref().map(|(text, _)| UndoOp { name: c.name.clone(), text: text.clone(), base_hash: c.new.1.clone() }))
        .collect()
}

/// The review text: a header and a unified diff per changed set
pub fn review_lines(changes: &[SetChange]) -> Vec<String> {
    let mut out = Vec::new();
    for c in changes {
        let old = c.old.as_ref().map(|o| o.0.as_str()).unwrap_or("");
        let (a, d) = diff_stat(&line_diff(old, &c.new.0));
        out.push(format!("== {}{}  +{a} -{d}", c.name, if c.old.is_none() { " (new set)" } else { "" }));
        out.extend(unified_diff(old, &c.new.0, &c.name, 3));
    }
    out
}

/// Output of a child made safe for the terminal: escape sequences and control characters out
pub fn clean_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => match it.peek() {
                Some('[') => {
                    it.next();
                    for n in it.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    it.next();
                    while let Some(n) = it.next() {
                        if n == '\x07' || (n == '\x1b' && it.peek() == Some(&'\\')) {
                            if n == '\x1b' {
                                it.next();
                            }
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            },
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------- agent events

/// A tool name as the pane shows it: codj's own without the MCP prefix
fn tool_name(name: &str) -> &str {
    name.strip_prefix("mcp__codj__").unwrap_or(name)
}

/// One line of at most `n` characters
fn one_line(s: &str, n: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > n { format!("{}…", flat.chars().take(n.saturating_sub(1)).collect::<String>()) } else { flat }
}

/// The interesting arguments of a tool call, short: names and values, never whole texts
fn tool_args(input: &serde_json::Value) -> String {
    let Some(map) = input.as_object() else { return String::new() };
    let parts: Vec<String> = map
        .iter()
        .filter_map(|(k, v)| match v {
            serde_json::Value::String(s) if s.len() <= 40 && !s.contains('\n') => Some(format!("{k}={s}")),
            serde_json::Value::String(s) => Some(format!("{k}=<{} lines>", s.lines().count())),
            serde_json::Value::Number(n) => Some(format!("{k}={n}")),
            serde_json::Value::Bool(b) => Some(format!("{k}={b}")),
            _ => None,
        })
        .collect();
    one_line(&parts.join(" "), 70)
}

/// What one stream-json line (Claude Code's `--output-format stream-json`, which grok's headless
/// `streaming-messages-json` copies) shows in the pane: the agent's text, `→ tool args` for each
/// call, `✓`/`✗` and the first line of each result, and a closing summary. A line that is not
/// JSON is shown as it is.
pub fn stream_event_lines(line: &str) -> Vec<(String, bool)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        let l = clean_line(line);
        return if l.trim().is_empty() { vec![] } else { vec![(l, false)] };
    };
    let content = |v: &serde_json::Value| v["message"]["content"].as_array().cloned().unwrap_or_default();
    match v["type"].as_str() {
        Some("system") if v["subtype"] == "init" => {
            let model = v["model"].as_str().unwrap_or("?");
            let tools = v["tools"].as_array().map_or(0, |t| t.len());
            vec![(format!("-- agent ready: {model}, {tools} tools"), false)]
        }
        Some("assistant") => content(&v)
            .iter()
            .flat_map(|c| match c["type"].as_str() {
                Some("text") => c["text"].as_str().unwrap_or("").lines().map(|l| (clean_line(l), false)).collect::<Vec<_>>(),
                Some("tool_use") => {
                    let name = tool_name(c["name"].as_str().unwrap_or("?"));
                    vec![(format!("→ {name} {}", tool_args(&c["input"])).trim_end().to_string(), false)]
                }
                _ => vec![],
            })
            .collect(),
        Some("user") => content(&v)
            .iter()
            .filter(|c| c["type"] == "tool_result")
            .map(|c| {
                let text = match &c["content"] {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(" "),
                    _ => String::new(),
                };
                let err = c["is_error"].as_bool().unwrap_or(false);
                (format!("  {} {}", if err { "✗" } else { "✓" }, one_line(&clean_line(&text), 80)).trim_end().to_string(), err)
            })
            .collect(),
        Some("result") => {
            let turns = v["num_turns"].as_u64().unwrap_or(0);
            let secs = v["duration_ms"].as_f64().unwrap_or(0.) / 1000.;
            let cost = v["total_cost_usd"].as_f64().map(|c| format!(", ${c:.2}")).unwrap_or_default();
            let failed = v["is_error"].as_bool().unwrap_or(false);
            vec![(format!("-- {}: {turns} turns, {secs:.1} s{cost}", if failed { "failed" } else { "done" }), failed)]
        }
        _ => vec![],
    }
}

// ---------------------------------------------------------------- daemon + child tasks

#[derive(Debug)]
pub enum PromptEvent {
    Before(Result<Snapshot, String>),
    Line { text: String, err: bool },
    Exited(String),
    After(Result<Snapshot, String>),
    Undone(Vec<String>),
}

fn dbus_problem(what: &str, e: &zbus::Error) -> String {
    let t = e.to_string();
    if t.contains("UnknownMethod") || t.contains("Unknown method") || t.contains("No such method") {
        format!("codjd has no {what}: update codjd")
    } else {
        format!("{what}: {t}")
    }
}

/// Text and hash of every set (SetList, then SetSource each)
async fn snapshot(conn: &Connection) -> Result<Snapshot, String> {
    let proxy = ServerProxy::new(conn).await.map_err(|e| dbus_problem("SetList", &e))?;
    let sets = tokio::time::timeout(CALL_TIMEOUT, proxy.set_list())
        .await
        .map_err(|_| "SetList: codjd does not answer".to_string())?
        .map_err(|e| dbus_problem("SetList", &e))?;
    let mut snap = Snapshot::new();
    for (name, _, _) in sets {
        let r = tokio::time::timeout(CALL_TIMEOUT, proxy.set_source(&name))
            .await
            .map_err(|_| "SetSource: codjd does not answer".to_string())?
            .map_err(|e| dbus_problem("SetSource", &e))?;
        snap.insert(name, r);
    }
    Ok(snap)
}

async fn undo(conn: &Connection, ops: Vec<UndoOp>) -> Vec<String> {
    let Ok(proxy) = ServerProxy::new(conn).await else { return vec!["could not reach codjd".into()] };
    let mut out = Vec::new();
    for op in ops {
        let r = tokio::time::timeout(CALL_TIMEOUT, proxy.set_write(&op.name, &op.text, &op.base_hash, AUTHOR)).await;
        out.push(match r {
            Ok(Ok(reply)) if reply.starts_with("ok") => format!("{}: back to the version before the agent", op.name),
            Ok(Ok(reply)) => format!("{}: not undone: {reply}", op.name),
            Ok(Err(e)) => format!("{}: not undone: {}", op.name, dbus_problem("SetWrite", &e)),
            Err(_) => format!("{}: not undone: codjd does not answer", op.name),
        });
    }
    out
}

async fn run_child(cmd: AgentCmd, tx: mpsc::UnboundedSender<PromptEvent>, mut kill: oneshot::Receiver<()>) {
    let say = |text: String, err: bool| {
        let _ = tx.send(PromptEvent::Line { text, err });
    };
    if let Err(e) = std::fs::write(&cmd.mcp_path, &cmd.mcp_json) {
        say(format!("could not write the MCP config {}: {e}", cmd.mcp_path.display()), true);
        let _ = tx.send(PromptEvent::Exited("not started".into()));
        return;
    }
    let mut c = tokio::process::Command::new(&cmd.program);
    c.args(&cmd.args)
        .envs(cmd.env.iter().map(|(k, v)| (k, v)))
        .stdin(if cmd.stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            say(format!("could not start `{}`: {e}", cmd.program), true);
            let _ = std::fs::remove_file(&cmd.mcp_path);
            let _ = tx.send(PromptEvent::Exited("not started".into()));
            return;
        }
    };
    if let (Some(mut stdin), Some(text)) = (child.stdin.take(), cmd.stdin.clone()) {
        tokio::spawn(async move {
            let _ = stdin.write_all(text.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(tokio::spawn(async move {
            let stream = cmd.stream_json;
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if stream {
                    for (text, err) in stream_event_lines(&l) {
                        let _ = tx.send(PromptEvent::Line { text, err });
                    }
                } else {
                    let _ = tx.send(PromptEvent::Line { text: clean_line(&l), err: false });
                }
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send(PromptEvent::Line { text: clean_line(&l), err: true });
            }
        }));
    }
    let status = tokio::select! {
        st = child.wait() => st.map(|s| s.to_string()).unwrap_or_else(|e| format!("wait failed: {e}")),
        _ = &mut kill => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            "cancelled".to_string()
        }
    };
    // the pipes close with the child; a grandchild keeping them open must not hang us
    for r in readers {
        let _ = tokio::time::timeout(Duration::from_millis(500), r).await;
    }
    let _ = std::fs::remove_file(&cmd.mcp_path);
    let _ = tx.send(PromptEvent::Exited(status));
}

// ---------------------------------------------------------------- the pane

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// typing the request
    Input,
    /// reading the sets before the agent starts
    Starting,
    Running,
    /// the child ended, reading the sets again
    Collecting,
    /// a diff is waiting for keep / undo
    Review,
    Done,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PromptOutcome {
    Stay,
    /// the request was entered: the screen builds the preamble and calls [`Prompt::start`]
    Submit(String),
    Close,
}

pub struct Prompt {
    conn: Option<Connection>,
    agent: AgentSettings,
    /// shown (owns the keyboard); a running agent keeps going while hidden
    pub visible: bool,
    input: Input,
    pub phase: Phase,
    out: Vec<(String, bool)>,
    /// lines scrolled back from the end (0 = follow)
    back: usize,
    prompt: String,
    before: Snapshot,
    changes: Vec<SetChange>,
    review: Vec<String>,
    review_scroll: usize,
    kill: Option<oneshot::Sender<()>>,
    cancelled: bool,
    status: String,
    tx: mpsc::UnboundedSender<PromptEvent>,
    rx: mpsc::UnboundedReceiver<PromptEvent>,
    /// the conversation this run belongs to (the DJ screen keeps it across prompts)
    session: Option<Session>,
}

impl Prompt {
    /// The conversation the next run continues
    pub fn set_session(&mut self, session: Session) {
        self.session = Some(session);
    }

    pub fn new(conn: Option<Connection>, agent: AgentSettings) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Prompt {
            conn,
            agent,
            visible: true,
            input: Input::default(),
            phase: Phase::Input,
            out: Vec::new(),
            back: 0,
            prompt: String::new(),
            before: Snapshot::new(),
            changes: Vec::new(),
            review: Vec::new(),
            review_scroll: 0,
            kill: None,
            cancelled: false,
            status: String::new(),
            tx,
            rx,
            session: None,
        }
    }

    pub fn live(&self) -> bool {
        self.agent.live.unwrap_or(false)
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Starting | Phase::Running | Phase::Collecting)
    }

    /// What to tell the DJ while the pane is hidden
    pub fn hidden_note(&self) -> Option<String> {
        match self.phase {
            Phase::Starting | Phase::Running | Phase::Collecting => Some("AI agent running (F11 shows it)".into()),
            Phase::Review => Some(format!("AI agent changed {} set(s): F11 to keep or undo", self.changes.len())),
            _ => None,
        }
    }

    /// Starts the agent with the full prompt (request + preamble). The sets are read first.
    pub fn start(&mut self, full_prompt: String) {
        self.prompt = full_prompt;
        self.phase = Phase::Starting;
        self.out.clear();
        self.back = 0;
        self.cancelled = false;
        self.status = "reading the sets...".into();
        let Some(conn) = self.conn.clone() else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(PromptEvent::Before(snapshot(&conn).await));
        });
    }

    fn launch(&mut self) {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("codj"));
        let mcp_path = std::env::temp_dir().join(format!("codj-agent-{}-{}.json", std::process::id(), self.out.len() + self.before.len()));
        match build_agent_cmd(&self.agent, &exe, &mcp_path, &self.prompt, self.session.as_ref()) {
            Ok(cmd) => {
                self.out.push((format!("$ {}", cmd.display()), false));
                let (ktx, krx) = oneshot::channel();
                self.kill = Some(ktx);
                self.phase = Phase::Running;
                self.status = "running".into();
                tokio::spawn(run_child(cmd, self.tx.clone(), krx));
            }
            Err(e) => {
                self.out.push((e, true));
                self.phase = Phase::Done;
                self.status = "not started".into();
            }
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> PromptOutcome {
        if key.kind == KeyEventKind::Release {
            return PromptOutcome::Stay;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match self.phase {
            Phase::Input => match key.code {
                KeyCode::Esc | KeyCode::F(11) => return PromptOutcome::Close,
                KeyCode::Enter => {
                    let text = self.input.value().trim().to_string();
                    return if text.is_empty() { PromptOutcome::Close } else { PromptOutcome::Submit(text) };
                }
                KeyCode::Char(c) if !ctrl => {
                    self.input.handle(InputRequest::InsertChar(c));
                }
                KeyCode::Char('c') => return PromptOutcome::Close,
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
            },
            Phase::Starting | Phase::Running | Phase::Collecting => match key.code {
                KeyCode::Char('c') if ctrl => self.cancel(),
                KeyCode::Esc | KeyCode::F(11) => self.visible = false,
                KeyCode::Up | KeyCode::Char('k') => self.back = (self.back + 1).min(self.out.len()),
                KeyCode::Down | KeyCode::Char('j') => self.back = self.back.saturating_sub(1),
                KeyCode::PageUp => self.back = (self.back + 10).min(self.out.len()),
                KeyCode::PageDown => self.back = self.back.saturating_sub(10),
                _ => {}
            },
            Phase::Review => match key.code {
                KeyCode::Char('y') => self.decide(true),
                KeyCode::Char('n') => self.decide(false),
                KeyCode::Esc | KeyCode::F(11) => self.visible = false,
                KeyCode::Up | KeyCode::Char('k') => self.review_scroll = self.review_scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.review_scroll = (self.review_scroll + 1).min(self.review.len().saturating_sub(1)),
                KeyCode::PageUp => self.review_scroll = self.review_scroll.saturating_sub(10),
                KeyCode::PageDown => self.review_scroll = (self.review_scroll + 10).min(self.review.len().saturating_sub(1)),
                _ => {}
            },
            Phase::Done => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::F(11) | KeyCode::Char('q') => return PromptOutcome::Close,
                KeyCode::Up | KeyCode::Char('k') => self.back = (self.back + 1).min(self.out.len()),
                KeyCode::Down | KeyCode::Char('j') => self.back = self.back.saturating_sub(1),
                _ => {}
            },
        }
        PromptOutcome::Stay
    }

    /// Ctrl-C: kill the child (or stop it from starting)
    fn cancel(&mut self) {
        self.cancelled = true;
        match self.kill.take() {
            Some(k) => {
                let _ = k.send(());
                self.status = "cancelling...".into();
            }
            None if self.phase == Phase::Starting => {
                self.out.push(("cancelled before the agent started".into(), true));
                self.phase = Phase::Done;
                self.status = "cancelled".into();
            }
            None => {}
        }
    }

    fn decide(&mut self, keep: bool) {
        let ops = undo_ops(&self.changes, keep);
        if keep {
            self.out.push((format!("kept the agent's changes to {} set(s)", self.changes.len()), false));
            self.status = "kept".into();
            self.phase = Phase::Done;
            return;
        }
        let created: Vec<_> = self.changes.iter().filter(|c| c.old.is_none()).map(|c| c.name.clone()).collect();
        for n in created {
            self.out.push((format!("{n}: created by the agent, left in place (delete the file to remove it)"), true));
        }
        self.status = UNDO_NOTE.into();
        self.phase = Phase::Done;
        let Some(conn) = self.conn.clone() else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(PromptEvent::Undone(undo(&conn, ops).await));
        });
    }

    fn push_line(&mut self, text: String, err: bool) {
        self.out.push((text, err));
        if self.out.len() > MAX_OUT_LINES {
            self.out.drain(..self.out.len() - MAX_OUT_LINES);
        }
        if self.back > 0 {
            self.back = (self.back + 1).min(self.out.len());
        }
    }

    /// Drains what the tasks reported; call every loop turn
    pub fn tick(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                PromptEvent::Before(Ok(snap)) => {
                    self.before = snap;
                    if self.cancelled {
                        continue;
                    }
                    self.launch();
                }
                PromptEvent::Before(Err(e)) => {
                    self.push_line(format!("not started: {e}"), true);
                    self.phase = Phase::Done;
                    self.status = "not started".into();
                }
                PromptEvent::Line { text, err } => self.push_line(text, err),
                PromptEvent::Exited(status) => {
                    self.kill = None;
                    self.push_line(format!("-- agent exited: {status}"), false);
                    self.status = "looking for changes...".into();
                    self.phase = Phase::Collecting;
                    if let Some(conn) = self.conn.clone() {
                        let tx = self.tx.clone();
                        tokio::spawn(async move {
                            let _ = tx.send(PromptEvent::After(snapshot(&conn).await));
                        });
                    }
                }
                PromptEvent::After(Ok(after)) => {
                    self.changes = changed_sets(&self.before, &after);
                    if self.changes.is_empty() {
                        self.push_line("-- the agent changed no set".into(), false);
                        self.status = "no changes".into();
                        self.phase = Phase::Done;
                    } else {
                        self.review = review_lines(&self.changes);
                        // the run's summary (turns, time, cost) stays in sight above the diff
                        if let Some((summary, _)) = self.out.iter().rev().find(|(l, _)| l.starts_with("-- done:") || l.starts_with("-- failed:")) {
                            self.review.insert(0, summary.clone());
                        }
                        self.review_scroll = 0;
                        self.status = format!("{} set(s) changed", self.changes.len());
                        self.phase = Phase::Review;
                    }
                }
                PromptEvent::After(Err(e)) => {
                    self.push_line(format!("could not read the sets afterwards: {e}"), true);
                    self.phase = Phase::Done;
                }
                PromptEvent::Undone(results) => {
                    for r in results {
                        self.push_line(r, false);
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ drawing

    pub fn render(&self, buf: &mut Buffer, area: Rect) {
        if area.width < 20 || area.height < 4 {
            return;
        }
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].reset();
            }
        }
        let title = match self.phase {
            Phase::Input => match self.session.as_ref().filter(|s| s.turns > 0) {
                Some(s) => format!(" AI prompt · turn {} of this conversation · Enter runs · :ai new starts over · Esc closes ", s.turns + 1),
                None => " AI prompt · Enter runs · Esc closes ".to_string(),
            },
            Phase::Starting | Phase::Running | Phase::Collecting => format!(" AI agent · {} · Ctrl-C cancels · Esc hides ", self.status),
            Phase::Review => format!(" AI agent · {} · y keep · n undo ", self.status),
            Phase::Done => format!(" AI agent · {} · Esc closes ", self.status),
        };
        let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Magenta)).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        let w = inner.width.saturating_sub(1) as usize;
        match self.phase {
            Phase::Input => {
                let cmd = self.agent.command.as_deref().unwrap_or(DEFAULT_COMMAND);
                buf.set_stringn(inner.x, inner.y, format!("ask: {}█", self.input.value()), w, Style::default());
                if inner.height > 1 {
                    let note = format!("runs `{cmd}` with codj's MCP set tools{}", if self.live() { " (LIVE: it may arm and load)" } else { " (not live: audio is off limits)" });
                    buf.set_stringn(inner.x, inner.y + 1, note, w, Style::default().fg(Color::DarkGray));
                }
            }
            Phase::Review => {
                let rows = inner.height.saturating_sub(1) as usize;
                for (i, l) in self.review.iter().skip(self.review_scroll).take(rows).enumerate() {
                    let style = if l.starts_with("==") { Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow) } else { diff_style(l) };
                    buf.set_stringn(inner.x, inner.y + i as u16, l, w, style);
                }
                buf.set_stringn(
                    inner.x,
                    inner.bottom() - 1,
                    "y keep · n undo (writes the old text back as human, \"undo agent change\") · j/k scroll · Esc hide",
                    w,
                    Style::default().fg(Color::Yellow),
                );
            }
            _ => {
                let rows = inner.height as usize;
                let end = self.out.len().saturating_sub(self.back);
                let start = end.saturating_sub(rows);
                for (i, (l, err)) in self.out[start..end].iter().enumerate() {
                    let style = if *err { Style::default().fg(Color::Red) } else if l.starts_with("$ ") || l.starts_with("--") { Style::default().fg(Color::DarkGray) } else { Style::default() };
                    buf.set_stringn(inner.x, inner.y + i as u16, l, w, style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(command: Option<&str>, live: bool, via: Option<&str>, tools: Option<&[&str]>) -> AgentSettings {
        AgentSettings {
            command: command.map(str::to_string),
            live: Some(live),
            prompt_via: via.map(str::to_string),
            mcp_tools: tools.map(|t| t.iter().map(|s| s.to_string()).collect()),
            output: None,
        }
    }

    #[test]
    fn stream_json_events_become_readable_lines() {
        let l = |s: &str| stream_event_lines(s);
        assert_eq!(l(r#"{"type":"system","subtype":"init","model":"claude-opus-5-5","tools":["a","b","c"]}"#), [("-- agent ready: claude-opus-5-5, 3 tools".to_string(), false)]);
        let both = l(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Reading the set.\nThen fixing it."},{"type":"tool_use","name":"mcp__codj__set_source","input":{"name":"claude-trio"}}]}}"#);
        assert_eq!(both, [("Reading the set.".to_string(), false), ("Then fixing it.".into(), false), ("→ set_source name=claude-trio".into(), false)]);
        // a whole set text is never dumped into the pane
        let write = l(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__codj__set_write","input":{"name":"x","text":"set \"x\"\ntrack a \"f\"\n","base_hash":"abc"}}]}}"#);
        assert_eq!(write[0].0, "→ set_write name=x text=<2 lines> base_hash=abc");
        assert_eq!(l(r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok 3f2a","is_error":false}]}}"#), [("  ✓ ok 3f2a".to_string(), false)]);
        assert_eq!(l(r#"{"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"text","text":"set changed since you read it"}],"is_error":true}]}}"#), [("  ✗ set changed since you read it".to_string(), true)]);
        assert_eq!(l(r#"{"type":"result","subtype":"success","num_turns":4,"duration_ms":12345,"total_cost_usd":0.0712,"is_error":false}"#), [("-- done: 4 turns, 12.3 s, $0.07".to_string(), false)]);
        // not JSON: shown as it is; unknown event types: nothing
        assert_eq!(l("plain words"), [("plain words".to_string(), false)]);
        assert!(l(r#"{"type":"stream_event"}"#).is_empty());
    }

    #[test]
    fn claude_streams_events_and_other_commands_print_text() {
        let c = build_agent_cmd(&cfg(None, false, None, None), Path::new("/c"), Path::new("/m"), "p", None).unwrap();
        assert!(c.stream_json && c.args.windows(2).any(|w| w == ["--output-format", "stream-json"]) && c.args.contains(&"--verbose".to_string()));
        let o = build_agent_cmd(&cfg(Some("my-agent"), false, None, None), Path::new("/c"), Path::new("/m"), "p", None).unwrap();
        assert!(!o.stream_json && !o.args.contains(&"--output-format".to_string()));
        let mut forced = cfg(Some("my-agent"), false, None, None);
        forced.output = Some("stream-json".into());
        assert!(build_agent_cmd(&forced, Path::new("/c"), Path::new("/m"), "p", None).unwrap().stream_json);
        let mut text = cfg(None, false, None, None);
        text.output = Some("text".into());
        let t = build_agent_cmd(&text, Path::new("/c"), Path::new("/m"), "p", None).unwrap();
        assert!(!t.stream_json && !t.args.contains(&"stream-json".to_string()));
        let mut bad = cfg(None, false, None, None);
        bad.output = Some("xml".into());
        assert!(build_agent_cmd(&bad, Path::new("/c"), Path::new("/m"), "p", None).is_err());
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn command_lines_split_like_a_shell_would_for_simple_cases() {
        assert_eq!(split_command("claude -p"), ["claude", "-p"]);
        assert_eq!(split_command("  my-agent  --model \"big one\" 'x y' a\\ b "), ["my-agent", "--model", "big one", "x y", "a b"]);
        assert_eq!(split_command("a \"\" b"), ["a", "", "b"]);
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn claude_gets_the_mcp_flags_and_the_prompt_on_stdin() {
        let c = build_agent_cmd(&cfg(None, false, None, None), Path::new("/opt/codj"), Path::new("/tmp/m.json"), "PROMPT", None).unwrap();
        assert_eq!(c.program, "claude");
        assert_eq!(c.args[0], "-p");
        let at = |f: &str| c.args.iter().position(|a| a == f).unwrap();
        assert_eq!(c.args[at("--mcp-config") + 1], "/tmp/m.json");
        assert!(c.args.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(c.args[at("--tools") + 1], "", "no built-in tools");
        let allowed = &c.args[at("--allowedTools") + 1];
        assert!(allowed.contains("mcp__codj__set_check") && allowed.contains("mcp__codj__set_write"));
        assert!(!allowed.contains("set_arm"), "live tools stay out when not live");
        assert_eq!(c.stdin.as_deref(), Some("PROMPT"));
        assert!(!c.args.contains(&"PROMPT".to_string()));
        assert!(c.env.contains(&("CODJ_MCP_LIVE".into(), "0".into())));
    }

    #[test]
    fn live_adds_the_flag_and_the_live_tools() {
        let c = build_agent_cmd(&cfg(Some("claude -p"), true, None, None), Path::new("/opt/codj"), Path::new("/tmp/m.json"), "P", None).unwrap();
        let allowed = c.args.iter().find(|a| a.contains("mcp__codj__")).unwrap();
        assert!(allowed.contains("mcp__codj__set_arm"));
        let json: serde_json::Value = serde_json::from_str(&c.mcp_json).unwrap();
        assert_eq!(json["mcpServers"]["codj"]["command"], "/opt/codj");
        assert_eq!(json["mcpServers"]["codj"]["args"], serde_json::json!(["mcp", "--live"]));
        let safe: serde_json::Value = serde_json::from_str(&mcp_config_json(Path::new("/opt/codj"), false)).unwrap();
        assert_eq!(safe["mcpServers"]["codj"]["args"], serde_json::json!(["mcp"]));
    }

    #[test]
    fn prompt_as_the_last_argument() {
        let c = build_agent_cmd(&cfg(Some("claude -p"), false, Some("arg"), None), Path::new("/c"), Path::new("/m"), "do it", None).unwrap();
        assert_eq!(c.stdin, None);
        assert_eq!(&c.args[c.args.len() - 2..], ["--", "do it"], "-- keeps the list flags from eating it");
        // another command: no flags, prompt last, config through the environment
        let o = build_agent_cmd(&cfg(Some("my-agent --fast"), false, Some("arg"), None), Path::new("/c"), Path::new("/tmp/m.json"), "do it", None).unwrap();
        assert_eq!(o.program, "my-agent");
        assert_eq!(o.args, ["--fast", "do it"]);
        assert!(o.env.contains(&("CODJ_MCP_CONFIG".into(), "/tmp/m.json".into())));
        let o = build_agent_cmd(&cfg(Some("my-agent"), false, Some("stdin"), None), Path::new("/c"), Path::new("/m"), "p", None).unwrap();
        assert_eq!(o.stdin.as_deref(), Some("p"));
        assert!(o.args.is_empty());
    }

    #[test]
    fn custom_tool_list_and_bad_settings() {
        let c = build_agent_cmd(&cfg(None, false, None, Some(&["set_check", "mcp__other__x"])), Path::new("/c"), Path::new("/m"), "p", None).unwrap();
        let allowed = c.args.iter().find(|a| a.contains("mcp__")).unwrap();
        assert_eq!(allowed, "mcp__codj__set_check,mcp__other__x");
        assert!(build_agent_cmd(&cfg(None, false, Some("pipe"), None), Path::new("/c"), Path::new("/m"), "p", None).is_err());
        assert!(PromptVia::parse(Some("arg")).is_ok());
        let wrapped = build_agent_cmd(&cfg(Some("/usr/bin/claude -p"), false, None, None), Path::new("/c"), Path::new("/m"), "p", None).unwrap();
        assert!(wrapped.args.contains(&"--mcp-config".to_string()), "a path to claude counts as claude");
        // a conversation: the first prompt names the session, later ones resume it
        let first = Session { id: "4f2c9a1e-0b7d-4c3e-9f10-2a3b4c5d6e7f".into(), turns: 0 };
        let c = build_agent_cmd(&cfg(None, false, None, None), Path::new("/c"), Path::new("/m"), "p", Some(&first)).unwrap();
        let at = |c: &AgentCmd, f: &str| c.args.iter().position(|a| a == f);
        assert_eq!(c.args[at(&c, "--session-id").unwrap() + 1], first.id);
        assert!(at(&c, "--resume").is_none());
        let later = Session { turns: 2, ..first.clone() };
        let c = build_agent_cmd(&cfg(None, false, None, None), Path::new("/c"), Path::new("/m"), "p", Some(&later)).unwrap();
        assert_eq!(c.args[at(&c, "--resume").unwrap() + 1], first.id);
        // another agent command gets no claude flags
        let o = build_agent_cmd(&cfg(Some("my-agent"), false, None, None), Path::new("/c"), Path::new("/m"), "p", Some(&later)).unwrap();
        assert!(at(&o, "--resume").is_none());
        // a fresh id is a v4 UUID
        let id = Session::new().id;
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }

    fn set() -> SetState {
        SetState {
            name: "demo".into(),
            file: "/sets/demo.set".into(),
            errors: 0,
            tracks: vec![("warm".into(), "Aftertekno".into(), "/a.flac".into()), ("mid".into(), "Smokin".into(), "/b.flac".into())],
            mixes: vec![("warm".into(), "mid".into(), 8.0, 64.0)],
            ..Default::default()
        }
    }

    #[test]
    fn the_preamble_says_what_is_going_on_and_the_rules() {
        let a = DeckInfo { loaded: true, title: "Aftertekno".into(), artist: "Foo".into(), bpm: 124.04, key: "8A".into(), position: 83.0, duration: 300.0, playing: true, ..Default::default() };
        let t = build_prompt("make the mix longer", Some(&set()), &[a, DeckInfo::default()], &[], false);
        assert!(t.contains("Active set: demo (file /sets/demo.set)"));
        assert!(t.contains("warm: Aftertekno") && t.contains("warm -> mid, beat 8, 64 beats (16 bars)"));
        assert!(t.contains("Deck A: \"Aftertekno\" by Foo, 124.0 BPM, key 8A, 1:23 / 5:00, playing"));
        assert!(t.contains("Deck B: empty"));
        assert!(t.contains("only through the codj MCP set tools"));
        assert!(t.contains("set_check") && t.contains("before set_write"));
        assert!(t.contains("Never touch audio"));
        assert!(t.ends_with("Request:\nmake the mix longer\n"));
        let live = build_prompt("x", None, &[], &[], true);
        assert!(live.contains("Live mode is on") && !live.contains("Never touch audio"));
        assert!(live.contains("No set is active") && live.contains("Deck A: empty"));
    }

    #[test]
    fn the_preamble_tells_about_patches_and_the_slots() {
        let slots = vec![
            PatchSlot {
                loaded: true,
                name: "d1".into(),
                dirty: true,
                live: true,
                cpu: 0.3,
                muted_reason: "cpu overrun".into(),
                knobs: vec![("cutoff".into(), 0.3, 0.0, 1.0)],
                sequencers: vec![("drums".into(), 1, 2, 0), ("bass".into(), 0, u32::MAX, 3)],
                ..Default::default()
            },
            PatchSlot { loaded: true, name: "pad".into(), ..Default::default() },
        ];
        let t = build_prompt("make a bassline", None, &[], &slots, false);
        assert!(t.contains("P1: d1 (unsaved edits), LIVE (the audience hears it), MUTED: cpu overrun; knobs: cutoff 0.30; sequencers: drums pattern B (next: C), bass pattern A; cpu 30%"), "{t}");
        assert!(t.contains("P2: pad, cue-only (headphones)"), "{t}");
        assert!(t.contains("P3: empty") && t.contains("P4: empty"));
        assert!(t.contains("patch_node_types") && t.contains("patch_check") && t.contains("patch_write") && t.contains("patch_action"));
        assert!(t.contains("do not load, unload or send patch slots live"));
        let live = build_prompt("x", None, &[], &slots, true);
        assert!(live.contains("patch_load/patch_unload/patch_live work"));
        // without the slots polled all four read as empty
        assert!(build_prompt("x", None, &[], &[], false).contains("P1: empty"));
    }

    #[test]
    fn the_agent_may_use_the_patch_tools_by_default_and_the_live_ones_when_live() {
        let safe = allowed_tools(None, false);
        for t in PATCH_TOOLS {
            assert!(safe.contains(&format!("mcp__codj__{t}")), "{t}");
        }
        for t in PATCH_LIVE_TOOLS {
            assert!(!safe.contains(&format!("mcp__codj__{t}")), "{t}");
        }
        let live = allowed_tools(None, true);
        for t in PATCH_TOOLS.iter().chain(&PATCH_LIVE_TOOLS).chain(&LIVE_TOOLS).chain(&SET_TOOLS) {
            assert!(live.contains(&format!("mcp__codj__{t}")), "{t}");
        }
        // the lists name real MCP tools: the live ones are exactly the gated ones
        assert_eq!(PATCH_LIVE_TOOLS, ["patch_load", "patch_unload", "patch_live"]);
    }

    fn snap(items: &[(&str, &str, &str)]) -> Snapshot {
        items.iter().map(|(n, t, h)| (n.to_string(), (t.to_string(), h.to_string()))).collect()
    }

    #[test]
    fn detects_changed_and_new_sets() {
        let before = snap(&[("a", "one\n", "h1"), ("b", "same\n", "h2")]);
        let after = snap(&[("a", "one\ntwo\n", "h3"), ("b", "same\n", "h2"), ("c", "new\n", "h4")]);
        let ch = changed_sets(&before, &after);
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0].name, "a");
        assert_eq!(ch[0].old, Some(("one\n".into(), "h1".into())));
        assert_eq!(ch[1].name, "c");
        assert_eq!(ch[1].old, None);
        assert!(changed_sets(&before, &before).is_empty());
        let r = review_lines(&ch);
        assert_eq!(r[0], "== a  +1 -0");
        assert!(r.contains(&"+two".to_string()));
        assert!(r.contains(&"== c (new set)  +1 -0".to_string()));
    }

    #[test]
    fn undo_writes_the_old_text_on_the_current_hash() {
        let before = snap(&[("a", "one\n", "h1")]);
        let after = snap(&[("a", "one\ntwo\n", "h3"), ("c", "new\n", "h4")]);
        let ch = changed_sets(&before, &after);
        assert!(undo_ops(&ch, true).is_empty(), "keep writes nothing");
        let ops = undo_ops(&ch, false);
        assert_eq!(ops, vec![UndoOp { name: "a".into(), text: "one\n".into(), base_hash: "h3".into() }], "a new set has nothing to go back to");
    }

    #[test]
    fn child_output_is_made_safe() {
        assert_eq!(clean_line("plain"), "plain");
        assert_eq!(clean_line("\x1b[31mred\x1b[0m text"), "red text");
        assert_eq!(clean_line("a\x1b]0;title\x07b"), "ab");
        assert_eq!(clean_line("tab\there\r"), "tab    here");
        assert_eq!(clean_line("bell\x07 \x08ok"), "bell ok");
    }

    #[test]
    fn keys_in_the_prompt() {
        let mut p = Prompt::new(None, AgentSettings::default());
        assert_eq!(p.on_key(&key(KeyCode::Enter)), PromptOutcome::Close, "empty request");
        for c in "make it longer".chars() {
            p.on_key(&key(KeyCode::Char(c)));
        }
        p.on_key(&key(KeyCode::Backspace));
        assert_eq!(p.on_key(&key(KeyCode::Enter)), PromptOutcome::Submit("make it longe".into()));
        let mut p = Prompt::new(None, AgentSettings::default());
        assert_eq!(p.on_key(&key(KeyCode::Esc)), PromptOutcome::Close);
    }

    #[test]
    fn running_pane_hides_instead_of_closing_and_review_decides() {
        let mut p = Prompt::new(None, AgentSettings::default());
        p.phase = Phase::Running;
        assert_eq!(p.on_key(&key(KeyCode::Esc)), PromptOutcome::Stay);
        assert!(!p.visible);
        assert!(p.hidden_note().unwrap().contains("running"));
        // review: y keeps, nothing is written
        p.phase = Phase::Review;
        p.changes = changed_sets(&snap(&[("a", "1\n", "h1")]), &snap(&[("a", "2\n", "h2")]));
        assert!(p.hidden_note().unwrap().contains("keep or undo"));
        p.on_key(&key(KeyCode::Char('y')));
        assert_eq!(p.phase, Phase::Done);
        assert_eq!(p.on_key(&key(KeyCode::Esc)), PromptOutcome::Close);
    }

    #[test]
    fn output_lines_follow_the_tail_unless_scrolled() {
        let mut p = Prompt::new(None, AgentSettings::default());
        for i in 0..5 {
            p.push_line(format!("l{i}"), false);
        }
        p.back = 2;
        p.push_line("l5".into(), false);
        assert_eq!(p.back, 3, "scrolled back stays on the same line");
        for i in 0..MAX_OUT_LINES + 10 {
            p.push_line(format!("x{i}"), false);
        }
        assert_eq!(p.out.len(), MAX_OUT_LINES);
    }
}
