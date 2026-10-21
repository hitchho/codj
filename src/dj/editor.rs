//! The in-TUI set editor (`:e`, F10): a vim-style pane on the active set's source.
//!
//! - text and motions come from `edtui` (normal/insert/visual, `u`/`Ctrl-r`, `/` search)
//! - the set is read with `SetSource` (text + hash), checked with `SetCheck` shortly after the
//!   last edit and saved with `SetWrite` (author `human`, with the hash it was read at)
//! - errors (`file:line:col: message`) become gutter markers, the message of the cursor line is
//!   shown under the text
//!
//! The pure parts (error parsing, debounce, diff, key routing) are separate functions so they
//! are tested without a terminal or a daemon.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use edtui::events::KeyEvent as EKey;
use edtui::{EditorEventHandler, EditorMode, EditorState, EditorTheme, EditorView, Lines};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use tokio::sync::mpsc;
use zbus::Connection;

use crate::ServerProxy;

/// Time after the last edit before the text is sent to `SetCheck`
pub const CHECK_DEBOUNCE: Duration = Duration::from_millis(400);
/// the reason `SetWrite` gives when the file moved on
pub const CONFLICT_REASON: &str = "set changed since you read it";
/// author of everything written from this pane
pub const AUTHOR: &str = "human";

// ---------------------------------------------------------------- errors -> markers

/// An error placed on a line of the text (0-based line, 1-based column as the daemon says it)
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

/// Splits `file:line:col: message` (or `file:line: message`) into markers; messages without a
/// position come back in the second list. Lines count from 1 in the message.
pub fn parse_markers(errors: &[String]) -> (Vec<Marker>, Vec<String>) {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<(Regex, Regex)> = OnceLock::new();
    let (with_col, line_only) = RE.get_or_init(|| {
        (
            Regex::new(r"^.*?:(\d+):(\d+):\s*(.*)$").unwrap(),
            Regex::new(r"^.*?:(\d+):\s*(.*)$").unwrap(),
        )
    });
    let (mut marks, mut loose) = (Vec::new(), Vec::new());
    for e in errors {
        let parsed = if let Some(c) = with_col.captures(e) {
            Some((c[1].parse::<usize>(), c[2].parse::<usize>().ok(), c[3].to_string()))
        } else {
            line_only.captures(e).map(|c| (c[1].parse::<usize>(), Some(1), c[2].to_string()))
        };
        match parsed {
            Some((Ok(line), col, msg)) if line >= 1 => {
                marks.push(Marker { line: line - 1, col: col.unwrap_or(1).max(1), msg })
            }
            _ => loose.push(e.clone()),
        }
    }
    (marks, loose)
}

/// The first marker on `line`, for the message under the text
pub fn marker_on(marks: &[Marker], line: usize) -> Option<&Marker> {
    marks.iter().find(|m| m.line == line)
}

// ---------------------------------------------------------------- debounce

/// "Check once the typing stops": `touch` on every edit, `take` from the tick loop
#[derive(Debug, Clone)]
pub struct Debounce {
    delay: Duration,
    last_edit: Option<Instant>,
}

impl Debounce {
    pub fn new(delay: Duration) -> Self {
        Debounce { delay, last_edit: None }
    }

    pub fn touch(&mut self, now: Instant) {
        self.last_edit = Some(now);
    }

    pub fn pending(&self) -> bool {
        self.last_edit.is_some()
    }

    /// True once, when the last edit is at least `delay` old
    pub fn take(&mut self, now: Instant) -> bool {
        match self.last_edit {
            Some(t) if now.saturating_duration_since(t) >= self.delay => {
                self.last_edit = None;
                true
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------- diff

#[derive(Debug, Clone, PartialEq)]
pub enum DiffOp {
    Same(String),
    Del(String),
    Add(String),
}

/// Line diff (longest common subsequence after trimming the common head and tail)
pub fn line_diff(old: &str, new: &str) -> Vec<DiffOp> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut ops: Vec<DiffOp> = a[..head].iter().map(|l| DiffOp::Same(l.to_string())).collect();
    if am.len().saturating_mul(bm.len()) > 4_000_000 {
        // too big for the table: show the middle as replaced
        ops.extend(am.iter().map(|l| DiffOp::Del(l.to_string())));
        ops.extend(bm.iter().map(|l| DiffOp::Add(l.to_string())));
    } else {
        let (n, m) = (am.len(), bm.len());
        let mut t = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i][j] = if am[i] == bm[j] { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                ops.push(DiffOp::Same(am[i].to_string()));
                i += 1;
                j += 1;
            } else if i < n && (j == m || t[i + 1][j] >= t[i][j + 1]) {
                ops.push(DiffOp::Del(am[i].to_string()));
                i += 1;
            } else {
                ops.push(DiffOp::Add(bm[j].to_string()));
                j += 1;
            }
        }
    }
    ops.extend(a[a.len() - tail..].iter().map(|l| DiffOp::Same(l.to_string())));
    ops
}

/// (added, removed) line counts
pub fn diff_stat(ops: &[DiffOp]) -> (usize, usize) {
    ops.iter().fold((0, 0), |(a, d), op| match op {
        DiffOp::Add(_) => (a + 1, d),
        DiffOp::Del(_) => (a, d + 1),
        DiffOp::Same(_) => (a, d),
    })
}

/// Unified diff of two texts with `context` lines around each change; empty when equal
pub fn unified_diff(old: &str, new: &str, name: &str, context: usize) -> Vec<String> {
    let ops = line_diff(old, new);
    if diff_stat(&ops) == (0, 0) {
        return Vec::new();
    }
    // (old line, new line) before each op, 1-based
    let mut pos = Vec::with_capacity(ops.len());
    let (mut o, mut n) = (1usize, 1usize);
    for op in &ops {
        pos.push((o, n));
        match op {
            DiffOp::Same(_) => {
                o += 1;
                n += 1;
            }
            DiffOp::Del(_) => o += 1,
            DiffOp::Add(_) => n += 1,
        }
    }
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, op)| !matches!(op, DiffOp::Same(_))).map(|(i, _)| i).collect();
    // hunks: ranges of ops, merged when their context touches
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &c in &changed {
        let (s, e) = (c.saturating_sub(context), (c + context + 1).min(ops.len()));
        match hunks.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => hunks.push((s, e)),
        }
    }
    let mut out = vec![format!("--- a/{name}"), format!("+++ b/{name}")];
    for (s, e) in hunks {
        let slice = &ops[s..e];
        let old_n = slice.iter().filter(|op| !matches!(op, DiffOp::Add(_))).count();
        let new_n = slice.iter().filter(|op| !matches!(op, DiffOp::Del(_))).count();
        let (os, ns) = pos[s];
        out.push(format!("@@ -{},{} +{},{} @@", if old_n == 0 { os - 1 } else { os }, old_n, if new_n == 0 { ns - 1 } else { ns }, new_n));
        for op in slice {
            out.push(match op {
                DiffOp::Same(l) => format!(" {l}"),
                DiffOp::Del(l) => format!("-{l}"),
                DiffOp::Add(l) => format!("+{l}"),
            });
        }
    }
    out
}

/// Colour of one unified-diff line
pub fn diff_style(line: &str) -> Style {
    if line.starts_with("+++") || line.starts_with("---") {
        Style::default().add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

// ---------------------------------------------------------------- keys

/// Who gets the keyboard on the DJ screen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFocus {
    /// the performance keys
    Deck,
    Editor,
    Prompt,
}

/// While a pane is open (visible) it owns every key; the performance keys are not active
pub fn key_focus(editor_open: bool, prompt_open: bool) -> KeyFocus {
    if editor_open {
        KeyFocus::Editor
    } else if prompt_open {
        KeyFocus::Prompt
    } else {
        KeyFocus::Deck
    }
}

/// A terminal key as edtui's own key type; None for keys it has no use for
pub fn to_edtui(key: &KeyEvent) -> Option<EKey> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    Some(match key.code {
        KeyCode::Char(c) if ctrl => EKey::Ctrl(c),
        KeyCode::Char(c) => EKey::Char(c),
        KeyCode::Enter => EKey::Enter,
        KeyCode::Esc => EKey::Esc,
        KeyCode::Backspace => EKey::Backspace,
        KeyCode::Up => EKey::Up,
        KeyCode::Down => EKey::Down,
        KeyCode::Left => EKey::Left,
        KeyCode::Right => EKey::Right,
        _ => return None,
    })
}

// ---------------------------------------------------------------- daemon calls

/// Results of the background calls, handled in [`Editor::tick`]
#[derive(Debug)]
pub enum EditorEvent {
    Loaded { text: String, hash: String },
    LoadFailed(String),
    Checked { seq: u64, errors: Vec<String> },
    CheckFailed(String),
    /// the reply of `SetWrite`, and the text that was sent
    Written { reply: String, text: String },
    /// the newer version, after a refused write
    Theirs { text: String, hash: String },
}

fn problem(what: &str, e: &zbus::Error) -> String {
    let t = e.to_string();
    if t.contains("UnknownMethod") || t.contains("Unknown method") || t.contains("No such method") {
        format!("codjd has no {what}: update codjd")
    } else {
        format!("{what}: {t}")
    }
}

const CALL_TIMEOUT: Duration = Duration::from_secs(5);

async fn source_of(conn: &Connection, name: &str) -> Result<(String, String), String> {
    let proxy = ServerProxy::new(conn).await.map_err(|e| problem("SetSource", &e))?;
    match tokio::time::timeout(CALL_TIMEOUT, proxy.set_source(name)).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(problem("SetSource", &e)),
        Err(_) => Err("SetSource: codjd does not answer".into()),
    }
}

// ---------------------------------------------------------------- the pane

/// What `Esc` etc. asks of the screen
#[derive(Debug, PartialEq, Eq)]
pub enum EditorOutcome {
    Stay,
    Close,
}

enum Modal {
    None,
    /// the write was refused because the file moved on
    Conflict { theirs: String, theirs_hash: String, diff: Vec<String>, show_diff: bool, scroll: usize },
}

pub struct Editor {
    conn: Option<Connection>,
    pub name: String,
    pub file: String,
    state: EditorState,
    handler: EditorEventHandler,
    loaded: bool,
    /// hash the text was read (or last written) at
    base_hash: String,
    /// the text as it is on disk as far as we know
    saved_text: String,
    /// the text as of the last key, to notice edits
    last_text: String,
    /// first visible line, mirrored from edtui's own scrolling (it does not expose it)
    top: usize,
    pub markers: Vec<Marker>,
    pub loose_errors: Vec<String>,
    check: Debounce,
    check_seq: u64,
    checked_seq: u64,
    /// `:` line being typed
    cmd: Option<String>,
    modal: Modal,
    status: Option<(String, Instant)>,
    /// Esc was pressed on unsaved text once already
    warned_discard: bool,
    close_after_write: bool,
    tx: mpsc::UnboundedSender<EditorEvent>,
    rx: mpsc::UnboundedReceiver<EditorEvent>,
}

fn lines_to_text(lines: &Lines) -> String {
    lines.to_vecs().iter().map(|l| l.iter().collect::<String>()).collect::<Vec<_>>().join("\n")
}

impl Editor {
    /// Opens the pane on set `name` and starts reading its source (no daemon when `conn` is None)
    pub fn new(conn: Option<Connection>, name: &str, file: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let ed = Editor {
            conn,
            name: name.to_string(),
            file: file.to_string(),
            state: EditorState::default(),
            handler: EditorEventHandler::default(),
            loaded: false,
            base_hash: String::new(),
            saved_text: String::new(),
            last_text: String::new(),
            top: 0,
            markers: Vec::new(),
            loose_errors: Vec::new(),
            check: Debounce::new(CHECK_DEBOUNCE),
            check_seq: 0,
            checked_seq: 0,
            cmd: None,
            modal: Modal::None,
            status: None,
            warned_discard: false,
            close_after_write: false,
            tx,
            rx,
        };
        ed.load();
        ed
    }

    fn load(&self) {
        let Some(conn) = self.conn.clone() else { return };
        let (tx, name) = (self.tx.clone(), self.name.clone());
        tokio::spawn(async move {
            let _ = tx.send(match source_of(&conn, &name).await {
                Ok((text, hash)) => EditorEvent::Loaded { text, hash },
                Err(e) => EditorEvent::LoadFailed(e),
            });
        });
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    /// the whole text
    pub fn text(&self) -> String {
        lines_to_text(&self.state.lines)
    }

    pub fn dirty(&self) -> bool {
        self.loaded && self.text() != self.saved_text
    }

    pub fn mode(&self) -> EditorMode {
        self.state.mode
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn hash(&self) -> &str {
        &self.base_hash
    }

    /// 0-based line of the cursor
    pub fn cursor_line(&self) -> usize {
        self.state.cursor.row
    }

    /// Replaces the text (as after a load or a reload): clean, cursor at the top
    fn set_text(&mut self, text: &str, hash: &str) {
        let mut state = EditorState::new(Lines::from(text));
        state.mode = EditorMode::Normal;
        self.state = state;
        self.handler = EditorEventHandler::default();
        self.top = 0;
        self.saved_text = text.to_string();
        self.last_text = text.to_string();
        self.base_hash = hash.to_string();
        self.loaded = true;
        self.markers.clear();
        self.loose_errors.clear();
        self.warned_discard = false;
        self.check.touch(Instant::now());
    }

    /// Handles a key; `Close` when the pane should go away
    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) -> EditorOutcome {
        if key.kind == KeyEventKind::Release {
            return EditorOutcome::Stay;
        }
        if !self.loaded {
            // still reading, or the read failed: only closing works
            if key.code == KeyCode::Esc || matches!(key.code, KeyCode::Char('q')) {
                return EditorOutcome::Close;
            }
            return EditorOutcome::Stay;
        }
        if !matches!(self.modal, Modal::None) {
            return self.on_modal_key(key);
        }
        if self.cmd.is_some() {
            return self.on_cmd_key(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('s') {
            self.write(false);
            return EditorOutcome::Stay;
        }
        let normal = self.state.mode == EditorMode::Normal;
        if normal && key.code == KeyCode::Esc {
            if self.dirty() && !self.warned_discard {
                self.warned_discard = true;
                self.set_status("unsaved changes: :w saves, Esc again discards them and closes");
                return EditorOutcome::Stay;
            }
            return EditorOutcome::Close;
        }
        if normal && key.code == KeyCode::Char(':') && !ctrl {
            self.cmd = Some(String::new());
            return EditorOutcome::Stay;
        }
        if let Some(k) = to_edtui(key) {
            self.handler.on_key_event(k, &mut self.state);
            self.after_edit(now);
        }
        EditorOutcome::Stay
    }

    fn after_edit(&mut self, now: Instant) {
        let text = self.text();
        if text != self.last_text {
            self.last_text = text;
            self.warned_discard = false;
            self.check.touch(now);
        }
    }

    fn on_cmd_key(&mut self, key: &KeyEvent) -> EditorOutcome {
        let Some(cmd) = self.cmd.as_mut() else { return EditorOutcome::Stay };
        match key.code {
            KeyCode::Esc => self.cmd = None,
            KeyCode::Backspace => {
                if cmd.pop().is_none() {
                    self.cmd = None;
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => cmd.push(c),
            KeyCode::Enter => {
                let line = self.cmd.take().unwrap_or_default();
                return self.run_ex(line.trim());
            }
            _ => {}
        }
        EditorOutcome::Stay
    }

    fn run_ex(&mut self, line: &str) -> EditorOutcome {
        match line {
            "" => {}
            "w" | "write" => self.write(false),
            "wq" | "x" => self.write(true),
            "q" | "quit" => {
                if self.dirty() {
                    self.set_status("unsaved changes: :w saves, :q! discards");
                } else {
                    return EditorOutcome::Close;
                }
            }
            "q!" => return EditorOutcome::Close,
            "e!" | "e" | "edit!" => {
                self.set_status("reloading...");
                self.loaded = false;
                self.load();
            }
            other => self.set_status(format!("unknown editor command: {other} (:w :q :q! :wq :e!)")),
        }
        EditorOutcome::Stay
    }

    /// `:w`: SetWrite with the hash the text was read at
    fn write(&mut self, then_close: bool) {
        self.close_after_write = then_close;
        let text = self.text();
        self.send_write(text, self.base_hash.clone());
        self.set_status("writing...");
    }

    fn send_write(&self, text: String, base: String) {
        let Some(conn) = self.conn.clone() else { return };
        let (tx, name) = (self.tx.clone(), self.name.clone());
        tokio::spawn(async move {
            let reply = async {
                let proxy = ServerProxy::new(&conn).await.map_err(|e| problem("SetWrite", &e))?;
                match tokio::time::timeout(CALL_TIMEOUT, proxy.set_write(&name, &text, &base, AUTHOR)).await {
                    Ok(Ok(r)) => Ok(r),
                    Ok(Err(e)) => Err(problem("SetWrite", &e)),
                    Err(_) => Err("SetWrite: codjd does not answer".to_string()),
                }
            }
            .await
            .unwrap_or_else(|e| e);
            let _ = tx.send(EditorEvent::Written { reply, text });
        });
    }

    /// overwrite: read the current hash, then write on top of it
    fn send_overwrite(&self, text: String) {
        let Some(conn) = self.conn.clone() else { return };
        let (tx, name) = (self.tx.clone(), self.name.clone());
        tokio::spawn(async move {
            let reply = async {
                let (_, hash) = source_of(&conn, &name).await?;
                let proxy = ServerProxy::new(&conn).await.map_err(|e| problem("SetWrite", &e))?;
                match tokio::time::timeout(CALL_TIMEOUT, proxy.set_write(&name, &text, &hash, AUTHOR)).await {
                    Ok(Ok(r)) => Ok(r),
                    Ok(Err(e)) => Err(problem("SetWrite", &e)),
                    Err(_) => Err("SetWrite: codjd does not answer".to_string()),
                }
            }
            .await
            .unwrap_or_else(|e| e);
            let _ = tx.send(EditorEvent::Written { reply, text });
        });
    }

    fn send_check(&mut self) {
        let Some(conn) = self.conn.clone() else { return };
        self.check_seq += 1;
        let (tx, seq, text) = (self.tx.clone(), self.check_seq, self.text());
        tokio::spawn(async move {
            let ev = async {
                let proxy = ServerProxy::new(&conn).await.map_err(|e| problem("SetCheck", &e))?;
                match tokio::time::timeout(CALL_TIMEOUT, proxy.set_check(&text)).await {
                    Ok(Ok(errors)) => Ok(errors),
                    Ok(Err(e)) => Err(problem("SetCheck", &e)),
                    Err(_) => Err("SetCheck: codjd does not answer".to_string()),
                }
            }
            .await;
            let _ = tx.send(match ev {
                Ok(errors) => EditorEvent::Checked { seq, errors },
                Err(e) => EditorEvent::CheckFailed(e),
            });
        });
    }

    fn fetch_theirs(&self) {
        let Some(conn) = self.conn.clone() else { return };
        let (tx, name) = (self.tx.clone(), self.name.clone());
        tokio::spawn(async move {
            let _ = tx.send(match source_of(&conn, &name).await {
                Ok((text, hash)) => EditorEvent::Theirs { text, hash },
                Err(e) => EditorEvent::LoadFailed(e),
            });
        });
    }

    fn on_modal_key(&mut self, key: &KeyEvent) -> EditorOutcome {
        let Modal::Conflict { theirs, theirs_hash, show_diff, scroll, diff } = &mut self.modal else {
            return EditorOutcome::Stay;
        };
        match key.code {
            KeyCode::Char('d') => *show_diff = !*show_diff,
            KeyCode::Char('j') | KeyCode::Down => *scroll = (*scroll + 1).min(diff.len().saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
            KeyCode::Char('r') => {
                let (t, h) = (theirs.clone(), theirs_hash.clone());
                self.modal = Modal::None;
                self.set_text(&t, &h);
                self.set_status("reloaded the newer version, your edits are discarded");
            }
            KeyCode::Char('o') => {
                self.modal = Modal::None;
                let text = self.text();
                self.send_overwrite(text);
                self.set_status("overwriting...");
            }
            KeyCode::Esc => {
                self.modal = Modal::None;
                self.set_status("kept your text; :w asks again");
            }
            _ => {}
        }
        EditorOutcome::Stay
    }

    /// Handles finished background calls and sends the debounced check; call every loop turn.
    /// Returns Close when `:wq` finished.
    pub fn tick(&mut self, now: Instant) -> EditorOutcome {
        let mut close = false;
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                EditorEvent::Loaded { text, hash } => {
                    self.set_text(&text, &hash);
                    self.status = None;
                }
                EditorEvent::LoadFailed(e) => {
                    self.set_status(e);
                }
                EditorEvent::Checked { seq, errors } => {
                    if seq >= self.checked_seq {
                        self.checked_seq = seq;
                        let (m, l) = parse_markers(&errors);
                        self.markers = m;
                        self.loose_errors = l;
                    }
                }
                EditorEvent::CheckFailed(e) => self.set_status(e),
                EditorEvent::Written { reply, text } => {
                    if let Some(hash) = reply.strip_prefix("ok") {
                        self.base_hash = hash.trim().to_string();
                        self.saved_text = text;
                        self.warned_discard = false;
                        self.set_status(format!("written {} (author {AUTHOR})", self.name));
                        close |= std::mem::take(&mut self.close_after_write);
                    } else if reply.contains(CONFLICT_REASON) {
                        self.close_after_write = false;
                        self.set_status("the set changed under you...");
                        self.fetch_theirs();
                    } else {
                        self.close_after_write = false;
                        self.set_status(reply);
                    }
                }
                EditorEvent::Theirs { text, hash } => {
                    let diff = unified_diff(&text, &self.text(), &self.name, 3);
                    self.modal = Modal::Conflict { theirs: text, theirs_hash: hash, diff, show_diff: false, scroll: 0 };
                    self.status = None;
                }
            }
        }
        if self.loaded && self.check.take(now) {
            self.send_check();
        }
        if close {
            EditorOutcome::Close
        } else {
            EditorOutcome::Stay
        }
    }

    // ------------------------------------------------------------ drawing

    /// Draws the pane into `area` (a bordered box)
    pub fn render(&mut self, buf: &mut Buffer, area: Rect) {
        if area.width < 20 || area.height < 4 {
            return;
        }
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].reset();
            }
        }
        let errs = self.markers.len() + self.loose_errors.len();
        let mode = match self.state.mode {
            EditorMode::Normal => "NORMAL",
            EditorMode::Insert => "INSERT",
            EditorMode::Visual => "VISUAL",
            EditorMode::Search => "SEARCH",
        };
        let mut title = format!(" {} · {}{} · {} ", self.name, mode, if self.dirty() { " [+]" } else { "" }, if errs == 0 { "✓".to_string() } else { format!("✗ {errs}") });
        if !self.loaded {
            title = format!(" {} · reading... ", self.name);
        }
        let border = if errs > 0 { Color::Red } else { Color::Magenta };
        let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(border)).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 2 {
            return;
        }
        let msg_row = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        let body = Rect::new(inner.x, inner.y, inner.width, inner.height - 1);

        if !self.loaded {
            let text = match &self.status {
                Some((s, _)) => format!("{s}   (Esc closes)"),
                None => "reading the set source...".into(),
            };
            buf.set_stringn(body.x + 1, body.y, text, body.width.saturating_sub(2) as usize, Style::default().fg(Color::Yellow));
            return;
        }
        if let Modal::Conflict { diff, show_diff, scroll, .. } = &self.modal {
            self.draw_conflict(buf, body, msg_row, diff, *show_diff, *scroll);
            return;
        }

        let gutter_w = 5u16.min(body.width / 2);
        let gutter = Rect::new(body.x, body.y, gutter_w, body.height);
        let text_area = Rect::new(body.x + gutter_w, body.y, body.width - gutter_w, body.height);

        // mirror edtui's vertical scrolling (same rule, same inputs) to align the gutter
        let h = text_area.height as usize;
        let row = self.state.cursor.row;
        if row < self.top {
            self.top = row;
        }
        let max = h.saturating_sub(1) + self.top;
        if row >= max {
            self.top += row - max;
        }
        let theme = EditorTheme::default().base(Style::default()).hide_status_line().selection_style(Style::default().bg(Color::Yellow).fg(Color::Black));
        EditorView::new(&mut self.state).theme(theme).wrap(false).render(text_area, buf);

        let total = self.state.lines.len();
        for i in 0..body.height as usize {
            let line = self.top + i;
            if line >= total {
                break;
            }
            let y = gutter.y + i as u16;
            let marked = marker_on(&self.markers, line).is_some();
            let (mark, style) = if marked {
                ("✗", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            } else {
                (" ", Style::default())
            };
            buf.set_string(gutter.x, y, mark, style);
            let num = format!("{:>w$} ", line + 1, w = gutter_w.saturating_sub(2) as usize);
            buf.set_stringn(gutter.x + 1, y, num, gutter_w.saturating_sub(1) as usize, Style::default().fg(if marked { Color::Red } else { Color::DarkGray }));
        }

        // message row: command line, else the cursor line's error, else status
        let w = msg_row.width as usize;
        if let Some(c) = &self.cmd {
            buf.set_stringn(msg_row.x, msg_row.y, format!(":{c}█"), w, Style::default().fg(Color::White));
        } else if let Some(m) = marker_on(&self.markers, self.state.cursor.row) {
            buf.set_stringn(msg_row.x, msg_row.y, format!("✗ line {} col {}: {}", m.line + 1, m.col, m.msg), w, Style::default().fg(Color::Red));
        } else if let Some((s, _)) = &self.status {
            buf.set_stringn(msg_row.x, msg_row.y, s, w, Style::default().fg(Color::Yellow));
        } else if let Some(l) = self.loose_errors.first() {
            buf.set_stringn(msg_row.x, msg_row.y, format!("✗ {l}"), w, Style::default().fg(Color::Red));
        } else if self.check.pending() {
            buf.set_stringn(msg_row.x, msg_row.y, "checking...", w, Style::default().fg(Color::DarkGray));
        } else {
            buf.set_stringn(msg_row.x, msg_row.y, ":w save · :q close · Esc (normal mode) close · i insert · / search · u undo", w, Style::default().fg(Color::DarkGray));
        }
    }

    fn draw_conflict(&self, buf: &mut Buffer, body: Rect, msg_row: Rect, diff: &[String], show_diff: bool, scroll: usize) {
        let w = body.width.saturating_sub(2) as usize;
        buf.set_stringn(body.x + 1, body.y, format!("{CONFLICT_REASON}: someone saved {} meanwhile.", self.name), w, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        buf.set_stringn(body.x + 1, body.y + 1, "r reload theirs (discard my edits) · d view diff · o overwrite theirs with mine · Esc keep editing", w, Style::default().fg(Color::Yellow));
        if show_diff && body.height > 3 {
            buf.set_stringn(body.x + 1, body.y + 2, "diff, theirs -> mine (what overwrite does); j/k scroll:", w, Style::default().fg(Color::DarkGray));
            for (i, l) in diff.iter().skip(scroll).take(body.height as usize - 3).enumerate() {
                buf.set_stringn(body.x + 1, body.y + 3 + i as u16, l, w, diff_style(l));
            }
            if diff.is_empty() {
                buf.set_stringn(body.x + 1, body.y + 3, "(the texts are equal)", w, Style::default().fg(Color::DarkGray));
            }
        }
        let _ = msg_row;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn loaded(text: &str) -> Editor {
        let mut e = Editor::new(None, "demo", "/x/demo.set");
        e.set_text(text, "h0");
        e
    }

    fn type_str(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.on_key(&key(KeyCode::Char(c)), Instant::now());
        }
    }

    #[test]
    fn markers_from_error_lines() {
        let errs = vec![
            "demo.set:3:7: unknown track `zzz`".to_string(),
            "demo.set:12: mix needs a length".to_string(),
            "/home/u/sets/a:b.set:5:1: colons: in the message: ok".to_string(),
            "no position at all".to_string(),
            "demo.set:0:1: line zero is not a line".to_string(),
        ];
        let (m, loose) = parse_markers(&errs);
        assert_eq!(m[0], Marker { line: 2, col: 7, msg: "unknown track `zzz`".into() });
        assert_eq!(m[1], Marker { line: 11, col: 1, msg: "mix needs a length".into() });
        assert_eq!(m[2].line, 4);
        assert_eq!(m[2].msg, "colons: in the message: ok");
        assert_eq!(loose, vec!["no position at all".to_string(), "demo.set:0:1: line zero is not a line".to_string()]);
        assert_eq!(marker_on(&m, 2).unwrap().col, 7);
        assert!(marker_on(&m, 0).is_none());
    }

    #[test]
    fn debounce_fires_once_after_the_quiet_time() {
        let t0 = Instant::now();
        let mut d = Debounce::new(CHECK_DEBOUNCE);
        assert!(!d.take(t0), "nothing edited");
        d.touch(t0);
        assert!(!d.take(t0 + Duration::from_millis(399)));
        // another edit restarts the wait
        d.touch(t0 + Duration::from_millis(300));
        assert!(!d.take(t0 + Duration::from_millis(500)));
        assert!(d.take(t0 + Duration::from_millis(700)));
        assert!(!d.take(t0 + Duration::from_millis(900)), "only once");
        assert!(!d.pending());
    }

    #[test]
    fn line_diff_and_unified() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nb\nc\nX\ne\nf\ng\nh\ni\nj\nk\n";
        let ops = line_diff(old, new);
        assert_eq!(diff_stat(&ops), (2, 1));
        let u = unified_diff(old, new, "demo.set", 1);
        assert_eq!(u[0], "--- a/demo.set");
        assert_eq!(u[1], "+++ b/demo.set");
        assert_eq!(u[2], "@@ -3,3 +3,3 @@");
        assert_eq!(&u[3..7], [" c", "-d", "+X", " e"]);
        assert_eq!(u[7], "@@ -10,1 +10,2 @@");
        assert_eq!(&u[8..], [" j", "+k"]);
        assert!(unified_diff(old, old, "x", 3).is_empty());
        // pure insertion into an empty text
        let u = unified_diff("", "one\ntwo\n", "n", 3);
        assert_eq!(u[2], "@@ -0,0 +1,2 @@");
        assert_eq!(&u[3..], ["+one", "+two"]);
        // a replaced middle
        assert_eq!(diff_stat(&line_diff("a\nb\nc", "a\nB\nc")), (1, 1));
    }

    #[test]
    fn text_round_trips_through_the_editor() {
        let t = "track a\n  cue 1\n\nmix a b\n";
        assert_eq!(loaded(t).text(), t);
        assert_eq!(loaded("").text(), "");
        assert_eq!(loaded("one line").text(), "one line");
    }

    #[test]
    fn insert_mode_edits_and_marks_dirty() {
        let mut e = loaded("abc\n");
        assert!(!e.dirty());
        type_str(&mut e, "ix");
        assert_eq!(e.mode(), EditorMode::Insert);
        assert!(e.dirty());
        assert_eq!(e.text(), "xabc\n");
        // Esc in insert mode only leaves insert mode
        assert_eq!(e.on_key(&key(KeyCode::Esc), Instant::now()), EditorOutcome::Stay);
        assert_eq!(e.mode(), EditorMode::Normal);
        // undo restores the saved text
        type_str(&mut e, "u");
        assert_eq!(e.text(), "abc\n");
        assert!(!e.dirty());
    }

    #[test]
    fn typed_symbols_arrive_as_text() {
        let mut e = loaded("");
        type_str(&mut e, "i");
        let shifted = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        e.on_key(&shifted, Instant::now());
        type_str(&mut e, ":;{}→");
        assert_eq!(e.text(), "A:;{}→");
    }

    #[test]
    fn esc_in_normal_mode_closes_but_warns_about_unsaved_text() {
        let mut e = loaded("abc\n");
        assert_eq!(e.on_key(&key(KeyCode::Esc), Instant::now()), EditorOutcome::Close);
        let mut e = loaded("abc\n");
        type_str(&mut e, "ix");
        e.on_key(&key(KeyCode::Esc), Instant::now()); // to normal
        assert_eq!(e.on_key(&key(KeyCode::Esc), Instant::now()), EditorOutcome::Stay, "warns first");
        assert_eq!(e.on_key(&key(KeyCode::Esc), Instant::now()), EditorOutcome::Close);
    }

    #[test]
    fn ex_commands() {
        let mut e = loaded("abc\n");
        let now = Instant::now();
        let ex = |e: &mut Editor, s: &str| {
            e.on_key(&key(KeyCode::Char(':')), now);
            for c in s.chars() {
                e.on_key(&key(KeyCode::Char(c)), now);
            }
            e.on_key(&key(KeyCode::Enter), now)
        };
        assert_eq!(ex(&mut e, "q"), EditorOutcome::Close);
        type_str(&mut e, "ix");
        e.on_key(&key(KeyCode::Esc), now);
        assert_eq!(ex(&mut e, "q"), EditorOutcome::Stay, ":q refuses with unsaved text");
        assert_eq!(ex(&mut e, "q!"), EditorOutcome::Close);
        assert_eq!(ex(&mut e, "nope"), EditorOutcome::Stay);
        // Esc leaves the command line without closing
        e.on_key(&key(KeyCode::Char(':')), now);
        assert_eq!(e.on_key(&key(KeyCode::Esc), now), EditorOutcome::Stay);
        assert!(e.cmd.is_none());
    }

    #[test]
    fn edits_arm_the_debounce() {
        let mut e = loaded("abc\n");
        let t0 = Instant::now() + Duration::from_secs(5);
        assert!(e.check.take(t0), "the initial check after loading");
        e.on_key(&key(KeyCode::Char('i')), t0);
        assert!(!e.check.pending(), "entering insert mode is not an edit");
        e.on_key(&key(KeyCode::Char('x')), t0);
        assert!(e.check.pending());
        assert!(!e.check.take(t0 + Duration::from_millis(100)));
        assert!(e.check.take(t0 + Duration::from_millis(450)));
    }

    #[test]
    fn who_gets_the_keys() {
        assert_eq!(key_focus(false, false), KeyFocus::Deck);
        assert_eq!(key_focus(true, false), KeyFocus::Editor);
        assert_eq!(key_focus(false, true), KeyFocus::Prompt);
        assert_eq!(key_focus(true, true), KeyFocus::Editor);
        // keys the editor does not know are dropped, never turned into DJ actions
        assert_eq!(to_edtui(&key(KeyCode::F(6))), None);
        assert_eq!(to_edtui(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)), Some(EKey::Ctrl('r')));
        assert_eq!(to_edtui(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)), None);
        let mut rel = key(KeyCode::Char('x'));
        rel.kind = KeyEventKind::Release;
        assert_eq!(to_edtui(&rel), None);
    }

    #[test]
    fn nothing_works_until_the_source_is_read() {
        let mut e = Editor::new(None, "demo", "f");
        let now = Instant::now();
        assert!(!e.is_loaded());
        assert_eq!(e.on_key(&key(KeyCode::Char('i')), now), EditorOutcome::Stay);
        assert_eq!(e.on_key(&key(KeyCode::Esc), now), EditorOutcome::Close);
    }

    #[test]
    fn written_replies_update_the_base_hash() {
        let mut e = loaded("abc\n");
        type_str(&mut e, "ix");
        let sent = e.text();
        e.tx.send(EditorEvent::Written { reply: "ok h1".into(), text: sent.clone() }).unwrap();
        e.tick(Instant::now());
        assert_eq!(e.hash(), "h1");
        assert!(!e.dirty());
        // a refusal changes nothing but the status
        type_str(&mut e, "y");
        e.tx.send(EditorEvent::Written { reply: "line 2: unknown track".into(), text: e.text() }).unwrap();
        e.tick(Instant::now());
        assert_eq!(e.hash(), "h1");
        assert!(e.dirty());
    }

    #[test]
    fn conflict_choices() {
        let mut e = loaded("mine\n");
        type_str(&mut e, "ix");
        e.tx.send(EditorEvent::Theirs { text: "theirs\n".into(), hash: "h9".into() }).unwrap();
        e.tick(Instant::now());
        assert!(matches!(e.modal, Modal::Conflict { .. }));
        // DJ/vim keys do nothing while the choice is open, reload takes theirs
        e.on_key(&key(KeyCode::Char('d')), Instant::now());
        e.on_key(&key(KeyCode::Char('r')), Instant::now());
        assert!(matches!(e.modal, Modal::None));
        assert_eq!(e.text(), "theirs\n");
        assert_eq!(e.hash(), "h9");
        assert!(!e.dirty());
    }

    #[test]
    fn stale_check_results_are_ignored() {
        let mut e = loaded("abc\n");
        e.checked_seq = 5;
        e.tx.send(EditorEvent::Checked { seq: 4, errors: vec!["f:1:1: old".into()] }).unwrap();
        e.tick(Instant::now());
        assert!(e.markers.is_empty());
        e.tx.send(EditorEvent::Checked { seq: 6, errors: vec!["f:1:1: new".into()] }).unwrap();
        e.tick(Instant::now());
        assert_eq!(e.markers.len(), 1);
    }
}
