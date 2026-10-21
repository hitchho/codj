//! The patch pane of DJ mode: the four slots of codjd's patch decks and the vim-key sequencer
//! grid. This file is pure: the grid model is parsed from the patch's JSON, keys come in as
//! normalized keys and go out as [`PEffect`]s (calls for the daemon), so all of it is tested
//! without a terminal or a daemon. Drawing is in `render`, the daemon calls in `mod`.
//!
//! The daemon reports a slot's sequencers as (name, pattern, queued, step) only, so the cells
//! and row names are read once with `PatchSource` when a slot loads (and again when its unsaved
//! flag clears), and codj mirrors its own cell edits and their undo locally.

use std::collections::BTreeMap;

use crossterm::event::KeyCode;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::keys::NKey;
use super::patchdef::SLOTS;
use crate::dj_types::PatchSlot;

/// Patterns per sequencer node, A..H
pub const PATTERNS: usize = 8;
/// "no queued pattern" in `PatchState`
pub const NONE_QUEUED: u32 = u32::MAX;

/// letter of pattern `p` (0 = A)
pub fn pattern_letter(p: usize) -> char {
    (b'A' + (p.min(PATTERNS - 1)) as u8) as char
}

/// What velocity digit `d` (1..=9) means: `d / 9`, so 9 is full and 1 is a ghost note
pub fn velocity_of_digit(d: u8) -> f64 {
    ((d.clamp(1, 9) as f64 / 9.0) * 100.0).round() / 100.0
}

// ---------------------------------------------------------------- the model

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqKind {
    /// one note per step; rows are scale degrees
    Mono,
    /// drum style: one gate per row, with velocities
    Gate,
}

/// cells of one pattern: (step, row) -> velocity (1.0 for notes)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pattern {
    pub cells: BTreeMap<(usize, usize), f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeqModel {
    pub name: String,
    pub kind: SeqKind,
    pub steps: usize,
    pub rows: usize,
    pub row_names: Vec<String>,
    pub scale: String,
    pub root: i32,
    /// always [`PATTERNS`] entries
    pub patterns: Vec<Pattern>,
}

/// One cell change with what it was before, for the local undo
#[derive(Debug, Clone, PartialEq)]
pub struct CellEdit {
    pub pattern: usize,
    pub step: usize,
    pub row: usize,
    pub before: Option<f64>,
    pub after: Option<f64>,
}

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// semitones of the scale degrees, by the scale's name as written in a patch ("A minor", "D dorian")
pub fn scale_intervals(scale: &str) -> Option<&'static [i32]> {
    let name = scale.split_whitespace().last()?.to_lowercase();
    Some(match name.as_str() {
        "major" | "ionian" => &[0, 2, 4, 5, 7, 9, 11],
        "minor" | "aeolian" => &[0, 2, 3, 5, 7, 8, 10],
        "dorian" => &[0, 2, 3, 5, 7, 9, 10],
        "phrygian" => &[0, 1, 3, 5, 7, 8, 10],
        "lydian" => &[0, 2, 4, 6, 7, 9, 11],
        "mixolydian" => &[0, 2, 4, 5, 7, 9, 10],
        "locrian" => &[0, 1, 3, 5, 6, 8, 10],
        "harmonic" => &[0, 2, 3, 5, 7, 8, 11],
        "pentatonic" => &[0, 2, 4, 7, 9],
        "blues" => &[0, 3, 5, 6, 7, 10],
        "chromatic" => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        _ => return None,
    })
}

/// MIDI note as "A2" (C4 = 60)
pub fn note_name(midi: i32) -> String {
    format!("{}{}", NOTE_NAMES[midi.rem_euclid(12) as usize], midi.div_euclid(12) - 1)
}

impl SeqModel {
    /// The label of a row: the drum name, or the scale degree with its note (`1 A2`, `5 E3`)
    pub fn row_label(&self, row: usize) -> String {
        match self.kind {
            SeqKind::Gate => self.row_names.get(row).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("row {}", row + 1)),
            SeqKind::Mono => match scale_intervals(&self.scale) {
                Some(iv) => {
                    let (octave, degree) = (row / iv.len(), row % iv.len());
                    format!("{} {}", row + 1, note_name(self.root + 12 * octave as i32 + iv[degree]))
                }
                None => format!("{}", row + 1),
            },
        }
    }

    pub fn cell(&self, pattern: usize, step: usize, row: usize) -> Option<f64> {
        self.patterns.get(pattern)?.cells.get(&(step, row)).copied()
    }

    /// Sets or clears a cell and returns what changed: a note step holds one note, so turning a
    /// cell on clears the rest of its column first (each change is its own edit)
    pub fn edit_cell(&mut self, pattern: usize, step: usize, row: usize, on: bool, velocity: f64) -> Vec<CellEdit> {
        let mut edits = Vec::new();
        if pattern >= PATTERNS || step >= self.steps || row >= self.rows {
            return edits;
        }
        let mut put = |m: &mut SeqModel, row: usize, after: Option<f64>| {
            let pat = &mut m.patterns[pattern];
            let before = pat.cells.get(&(step, row)).copied();
            if before == after {
                return;
            }
            match after {
                Some(v) => pat.cells.insert((step, row), v),
                None => pat.cells.remove(&(step, row)),
            };
            edits.push(CellEdit { pattern, step, row, before, after });
        };
        if on && self.kind == SeqKind::Mono {
            let others: Vec<usize> = (0..self.rows).filter(|r| *r != row && self.patterns[pattern].cells.contains_key(&(step, *r))).collect();
            for r in others {
                put(self, r, None);
            }
        }
        put(self, row, on.then(|| if self.kind == SeqKind::Mono { 1.0 } else { velocity.clamp(0.0, 1.0) }));
        edits
    }

    /// The `PatchAction` JSON of one cell edit
    pub fn action_json(&self, e: &CellEdit) -> String {
        let (on, velocity) = (e.after.is_some(), e.after.unwrap_or(0.0).max(if e.after.is_some() { 0.0 } else { 1.0 }));
        json!({"op":"cell","seq":self.name,"pattern":e.pattern,"step":e.step,"row":e.row,"on":on,"velocity":if on { velocity } else { 1.0 }}).to_string()
    }
}

/// What the grid needs of a patch file
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PatchModel {
    pub seqs: Vec<SeqModel>,
    /// (knob name, value in the file): what `\` goes back to
    pub knob_defaults: Vec<(String, f64)>,
}

/// Reads the sequencers and knobs out of a patch file's JSON
pub fn parse_model(text: &str) -> Result<PatchModel, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let nodes = v["nodes"].as_object().ok_or("the patch has no nodes")?;
    let mut model = PatchModel::default();
    for n in nodes.values() {
        let params = &n["params"];
        match n["type"].as_str() {
            Some("Knob") => {
                if let Some(name) = n["name"].as_str() {
                    model.knob_defaults.push((name.to_string(), params["value"].as_f64().unwrap_or(0.0)));
                }
            }
            Some(t @ ("MonoSeq" | "GateSeq")) => {
                let kind = if t == "MonoSeq" { SeqKind::Mono } else { SeqKind::Gate };
                let steps = params["steps"].as_u64().unwrap_or(16).clamp(1, 64) as usize;
                let rows = params["rows"].as_u64().unwrap_or(if kind == SeqKind::Gate { 8 } else { 12 }).clamp(1, if kind == SeqKind::Gate { 8 } else { 48 }) as usize;
                let mut patterns = vec![Pattern::default(); PATTERNS];
                for (i, p) in params["patterns"].as_array().into_iter().flatten().take(PATTERNS).enumerate() {
                    for c in p["cells"].as_array().into_iter().flatten() {
                        let (Some(s), Some(r)) = (c[0].as_u64(), c[1].as_u64()) else { continue };
                        if (s as usize) < steps && (r as usize) < rows {
                            let vel = if kind == SeqKind::Gate { c[2].as_f64().unwrap_or(1.0) } else { 1.0 };
                            patterns[i].cells.insert((s as usize, r as usize), vel);
                        }
                    }
                }
                model.seqs.push(SeqModel {
                    // sequencers are referenced by name; an unnamed one cannot be edited live
                    name: n["name"].as_str().unwrap_or("").to_string(),
                    kind,
                    steps,
                    rows,
                    row_names: params["row_names"].as_array().into_iter().flatten().map(|x| x.as_str().unwrap_or("").to_string()).collect(),
                    scale: params["scale"].as_str().unwrap_or("").to_string(),
                    root: params["root"].as_i64().unwrap_or(48) as i32,
                    patterns,
                });
            }
            _ => {}
        }
    }
    Ok(model)
}

// ---------------------------------------------------------------- the pane

/// What the pane asks of the screen
#[derive(Debug, Clone, PartialEq)]
pub enum PEffect {
    /// `PatchAction` on a slot
    Action { slot: u32, json: String },
    Undo(u32),
    Redo(u32),
    Live(u32, bool),
    /// `PatchSave` (name None = the slot's own)
    Save { slot: u32, name: Option<String> },
    Note(String),
    /// the `?` popup of the pane's keys
    Help,
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelState {
    None,
    Pending(String),
    Ready(String, PatchModel),
    Failed(String, String),
}

impl ModelState {
    fn name(&self) -> Option<&str> {
        match self {
            ModelState::None => None,
            ModelState::Pending(n) | ModelState::Ready(n, _) | ModelState::Failed(n, _) => Some(n),
        }
    }

    pub fn model(&self) -> Option<&PatchModel> {
        match self {
            ModelState::Ready(_, m) => Some(m),
            _ => None,
        }
    }
}

/// The edited sequencer: which one, which pattern, where the cursor is. `cy` counts rows as
/// drawn (drums from the top, notes with the highest at the top).
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub seq: usize,
    pub pattern: usize,
    pub cx: usize,
    pub cy: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Cells { seq: String, edits: Vec<CellEdit> },
    /// an edit codj does not mirror (a knob): undoing it does not change the grid
    Other,
}

#[derive(Debug, Default)]
struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

pub enum Fetched {
    Source { slot: usize, name: String, text: Result<String, String> },
}

pub struct PatchPane {
    pub open: bool,
    /// the slot the pane is on, 0..3
    pub sel: usize,
    pub grid: Option<Grid>,
    pub models: [ModelState; SLOTS],
    history: [History; SLOTS],
    /// velocity a newly toggled drum cell gets: the last digit pressed
    pub vel: f64,
    dirty_seen: [bool; SLOTS],
    tx: mpsc::UnboundedSender<Fetched>,
    rx: mpsc::UnboundedReceiver<Fetched>,
}

impl Default for PatchPane {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        PatchPane {
            open: false,
            sel: 0,
            grid: None,
            models: [ModelState::None, ModelState::None, ModelState::None, ModelState::None],
            history: Default::default(),
            vel: 1.0,
            dirty_seen: [false; SLOTS],
            tx,
            rx,
        }
    }
}

impl SeqModel {
    /// data row of a drawn row
    pub fn row_at(&self, cy: usize) -> usize {
        match self.kind {
            SeqKind::Gate => cy,
            SeqKind::Mono => self.rows.saturating_sub(1 + cy),
        }
    }
}

impl PatchPane {
    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.grid = None;
    }

    pub fn model(&self, slot: usize) -> Option<&PatchModel> {
        self.models.get(slot)?.model()
    }

    /// The edited sequencer's model, if its slot's patch has been read
    pub fn seq(&self) -> Option<&SeqModel> {
        let g = self.grid.as_ref()?;
        self.model(self.sel)?.seqs.get(g.seq)
    }

    /// Reads a patch's source for slots that need it; call every tick with the polled slots and
    /// a function that starts the fetch. A slot is read again when its name changes, and when its
    /// unsaved flag clears (saved, or undone back to the file).
    pub fn sync(&mut self, slots: &[PatchSlot], mut fetch: impl FnMut(usize, String, mpsc::UnboundedSender<Fetched>)) {
        for (i, s) in slots.iter().enumerate().take(SLOTS) {
            let cleaned = self.dirty_seen[i] && !s.dirty;
            self.dirty_seen[i] = s.dirty;
            if !s.loaded {
                self.models[i] = ModelState::None;
                self.history[i] = History::default();
                continue;
            }
            if self.models[i].name() != Some(s.name.as_str()) || (cleaned && !matches!(self.models[i], ModelState::Pending(_))) {
                self.models[i] = ModelState::Pending(s.name.clone());
                self.history[i] = History::default();
                fetch(i, s.name.clone(), self.tx.clone());
            }
        }
        while let Ok(Fetched::Source { slot, name, text }) = self.rx.try_recv() {
            if self.models[slot].name() != Some(name.as_str()) {
                continue;
            }
            self.models[slot] = match text.and_then(|t| parse_model(&t)) {
                Ok(m) => ModelState::Ready(name, m),
                Err(e) => ModelState::Failed(name, e),
            };
            // the cursor must stay inside what was read
            if slot == self.sel {
                self.clamp();
            }
        }
        // a grid on a slot that went away or lost its sequencer closes
        if self.grid.is_some() && self.seq().is_none() && !matches!(self.models[self.sel], ModelState::Pending(_)) {
            self.grid = None;
        }
    }

    fn clamp(&mut self) {
        let Some(seq) = self.seq().cloned() else { return };
        if let Some(g) = self.grid.as_mut() {
            g.cx = g.cx.min(seq.steps - 1);
            g.cy = g.cy.min(seq.rows - 1);
        }
    }

    /// Notes that an edit codj does not mirror went to the daemon's history (a knob moved)
    pub fn note_other_edit(&mut self, slot: usize) {
        if let Some(h) = self.history.get_mut(slot) {
            if h.undo.last() != Some(&Entry::Other) {
                h.undo.push(Entry::Other);
            }
            h.redo.clear();
        }
    }

    /// Opens the grid of sequencer `seq` of the selected slot, on the pattern that is playing
    fn open_grid(&mut self, slots: &[PatchSlot], seq: usize) -> Option<String> {
        let slot = slots.get(self.sel).filter(|s| s.loaded);
        let Some(slot) = slot else { return Some(format!("P{} is empty: :patch load {} <name>", self.sel + 1, self.sel + 1)) };
        match &self.models[self.sel] {
            ModelState::Pending(_) | ModelState::None => return Some("reading the patch...".into()),
            ModelState::Failed(_, e) => return Some(format!("cannot read the patch: {e}")),
            ModelState::Ready(_, m) if m.seqs.is_empty() => return Some(format!("{} has no sequencer: knobs only", slot.name)),
            ModelState::Ready(..) => {}
        }
        let n = self.model(self.sel).map_or(0, |m| m.seqs.len());
        let seq = seq % n;
        let name = &self.model(self.sel).unwrap().seqs[seq].name;
        let pattern = slot.sequencers.iter().find(|s| &s.0 == name).map_or(0, |s| s.1 as usize).min(PATTERNS - 1);
        self.grid = Some(Grid { seq, pattern, cx: 0, cy: 0 });
        None
    }

    /// Tab: the next sequencer, across the loaded slots
    fn next_seq(&mut self, slots: &[PatchSlot], back: bool) -> Option<String> {
        let mut all: Vec<(usize, usize)> = Vec::new();
        for s in 0..SLOTS {
            for q in 0..self.model(s).map_or(0, |m| m.seqs.len()) {
                all.push((s, q));
            }
        }
        if all.is_empty() {
            return Some("no sequencer in any slot".into());
        }
        let here = self.grid.as_ref().and_then(|g| all.iter().position(|x| *x == (self.sel, g.seq)));
        let at = match (here, back) {
            (Some(i), false) => (i + 1) % all.len(),
            (Some(i), true) => (i + all.len() - 1) % all.len(),
            (None, _) => all.iter().position(|x| x.0 >= self.sel).unwrap_or(0),
        };
        self.sel = all[at].0;
        self.open_grid(slots, all[at].1)
    }

    /// A key while the pane has the keyboard
    pub fn on_key(&mut self, k: &NKey, slots: &[PatchSlot]) -> Vec<PEffect> {
        let mut out = Vec::new();
        let note = |out: &mut Vec<PEffect>, n: Option<String>| {
            if let Some(n) = n {
                out.push(PEffect::Note(n));
            }
        };
        if k.alt {
            return out;
        }
        // keys of both levels
        match (k.code, k.ctrl, k.shift) {
            (KeyCode::Char('?'), ..) | (KeyCode::Char('/'), false, true) => {
                out.push(PEffect::Help);
                return out;
            }
            (KeyCode::Char('r'), true, _) => {
                out.extend(self.undo_redo(false));
                return out;
            }
            (_, true, _) => return out,
            (KeyCode::Char('u'), false, false) => {
                out.extend(self.undo_redo(true));
                return out;
            }
            (KeyCode::Char('p'), false, false) => {
                match slots.get(self.sel).filter(|s| s.loaded) {
                    Some(s) => out.push(PEffect::Live(self.sel as u32, !s.live)),
                    None => note(&mut out, Some(format!("P{} is empty", self.sel + 1))),
                }
                return out;
            }
            (KeyCode::Tab, _, shift) => {
                let n = self.next_seq(slots, shift);
                note(&mut out, n);
                return out;
            }
            _ => {}
        }
        if self.grid.is_none() {
            return self.overview_key(k, slots);
        }
        self.grid_key(k, slots)
    }

    fn overview_key(&mut self, k: &NKey, slots: &[PatchSlot]) -> Vec<PEffect> {
        let mut out = Vec::new();
        match k.code {
            KeyCode::Esc => out.push(PEffect::Close),
            KeyCode::Char(c @ '1'..='4') if !k.shift => {
                self.sel = (c as u8 - b'1') as usize;
                if let Some(n) = self.open_grid(slots, 0) {
                    out.push(PEffect::Note(n));
                }
            }
            KeyCode::Enter => {
                if let Some(n) = self.open_grid(slots, 0) {
                    out.push(PEffect::Note(n));
                }
            }
            KeyCode::Char('j') | KeyCode::Down => self.sel = (self.sel + 1) % SLOTS,
            KeyCode::Char('k') | KeyCode::Up => self.sel = (self.sel + SLOTS - 1) % SLOTS,
            _ => {}
        }
        out
    }

    fn grid_key(&mut self, k: &NKey, slots: &[PatchSlot]) -> Vec<PEffect> {
        let mut out = Vec::new();
        let Some(seq) = self.seq().cloned() else {
            self.grid = None;
            return out;
        };
        let slot_no = self.sel as u32;
        let g = self.grid.as_mut().unwrap();
        let (steps, rows) = (seq.steps, seq.rows);
        match k.code {
            KeyCode::Esc => self.grid = None,
            KeyCode::Char('h') | KeyCode::Left if !k.shift => g.cx = g.cx.saturating_sub(1),
            KeyCode::Char('l') | KeyCode::Right if !k.shift => g.cx = (g.cx + 1).min(steps - 1),
            KeyCode::Char('k') | KeyCode::Up if !k.shift => g.cy = g.cy.saturating_sub(1),
            KeyCode::Char('j') | KeyCode::Down if !k.shift => g.cy = (g.cy + 1).min(rows - 1),
            // w / b: a beat (4 steps) at a time; 0 and $: the ends
            KeyCode::Char('w') if !k.shift => g.cx = ((g.cx / 4 + 1) * 4).min(steps - 1),
            KeyCode::Char('b') if !k.shift => g.cx = if g.cx % 4 == 0 { g.cx.saturating_sub(4) } else { g.cx / 4 * 4 },
            KeyCode::Char('0') => g.cx = 0,
            KeyCode::Char('$') | KeyCode::Char('4') | KeyCode::End if k.shift || k.code == KeyCode::End => g.cx = steps - 1,
            KeyCode::Home => g.cx = 0,
            // A..H: the pattern to edit
            KeyCode::Char(c @ 'a'..='h') if k.shift => g.pattern = (c as u8 - b'a') as usize,
            KeyCode::Char('x') if !k.shift => {
                let (row, pattern, step) = (seq.row_at(g.cy), g.pattern, g.cx);
                let on = seq.cell(pattern, step, row).is_none();
                let vel = self.vel;
                out.extend(self.edit(slot_no, &seq.name, pattern, step, row, on, vel));
            }
            KeyCode::Char(c @ '1'..='9') if !k.shift => {
                if seq.kind == SeqKind::Mono {
                    out.push(PEffect::Note("velocity only applies to drum rows (GateSeq); x toggles a note".into()));
                } else {
                    let (row, pattern, step) = (seq.row_at(g.cy), g.pattern, g.cx);
                    let vel = velocity_of_digit(c as u8 - b'0');
                    self.vel = vel;
                    out.extend(self.edit(slot_no, &seq.name, pattern, step, row, true, vel));
                }
            }
            KeyCode::Enter => {
                let pattern = g.pattern;
                let playing = slots.get(self.sel).and_then(|s| s.sequencers.iter().find(|q| q.0 == seq.name)).map(|q| q.1 as usize);
                out.push(PEffect::Action {
                    slot: slot_no,
                    json: json!({"op":"pattern","seq":seq.name,"pattern":pattern,"now":false}).to_string(),
                });
                out.push(PEffect::Note(if playing == Some(pattern) {
                    format!("{} already plays pattern {}", seq.name, pattern_letter(pattern))
                } else {
                    format!("pattern {} queued: it plays at the next bar", pattern_letter(pattern))
                }));
            }
            _ => {}
        }
        out
    }

    /// Applies a cell edit to the model and returns the calls for the daemon
    #[allow(clippy::too_many_arguments)]
    fn edit(&mut self, slot: u32, seq_name: &str, pattern: usize, step: usize, row: usize, on: bool, vel: f64) -> Vec<PEffect> {
        let Some(ModelState::Ready(_, model)) = self.models.get_mut(slot as usize) else { return Vec::new() };
        let Some(seq) = model.seqs.iter_mut().find(|s| s.name == seq_name) else { return Vec::new() };
        let edits = seq.edit_cell(pattern, step, row, on, vel);
        if edits.is_empty() {
            return Vec::new();
        }
        let calls: Vec<PEffect> = edits.iter().map(|e| PEffect::Action { slot, json: seq.action_json(e) }).collect();
        let h = &mut self.history[slot as usize];
        h.undo.push(Entry::Cells { seq: seq_name.to_string(), edits });
        h.redo.clear();
        calls
    }

    /// `u` / `Ctrl-r`: the daemon steps its history; the model follows for the edits it knows
    fn undo_redo(&mut self, undo: bool) -> Vec<PEffect> {
        let slot = self.sel;
        let h = &mut self.history[slot];
        let entry = if undo { h.undo.pop() } else { h.redo.pop() };
        let mut out = Vec::new();
        if let Some(entry) = entry {
            if let Entry::Cells { seq, edits } = &entry {
                if let Some(ModelState::Ready(_, m)) = self.models.get_mut(slot) {
                    if let Some(s) = m.seqs.iter_mut().find(|s| &s.name == seq) {
                        let ordered: Vec<&CellEdit> = if undo { edits.iter().rev().collect() } else { edits.iter().collect() };
                        for e in ordered {
                            let to = if undo { e.before } else { e.after };
                            match to {
                                Some(v) => s.patterns[e.pattern].cells.insert((e.step, e.row), v),
                                None => s.patterns[e.pattern].cells.remove(&(e.step, e.row)),
                            };
                        }
                    }
                }
            }
            let h = &mut self.history[slot];
            if undo { h.redo.push(entry) } else { h.undo.push(entry) }
        }
        out.push(if undo { PEffect::Undo(slot as u32) } else { PEffect::Redo(slot as u32) });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    const TEXT: &str = r#"{"codj_patch":1,"title":"t","nodes":{
      "1":{"type":"Clock","name":"clock"},
      "2":{"type":"GateSeq","name":"drums","params":{"steps":16,"rows":4,"row_names":["kick","snare","hat","clap"],"pattern":0,
           "patterns":[{"steps":16,"cells":[[0,0,1.0],[4,1,0.5],[99,0,1],[2,9,1]]},{"steps":16,"cells":[]}]}},
      "3":{"type":"MonoSeq","name":"seq","params":{"scale":"A minor","root":45,"steps":8,"rows":9,"pattern":0,
           "patterns":[{"steps":8,"cells":[[0,3],[2,7]]}]}},
      "4":{"type":"Knob","name":"cutoff","params":{"min":0,"max":1,"value":0.3}},
      "5":{"type":"AudioOut"}}}"#;

    fn k(c: char) -> NKey {
        NKey { code: KeyCode::Char(c), alt: false, shift: false, ctrl: false }
    }
    fn shifted(c: char) -> NKey {
        NKey { shift: true, ..k(c) }
    }
    fn code(c: KeyCode) -> NKey {
        NKey { code: c, alt: false, shift: false, ctrl: false }
    }

    fn slot(name: &str, live: bool, seqs: &[(&str, u32, u32, u32)]) -> PatchSlot {
        PatchSlot {
            loaded: true,
            name: name.into(),
            live,
            sequencers: seqs.iter().map(|(n, a, b, c)| (n.to_string(), *a, *b, *c)).collect(),
            ..Default::default()
        }
    }

    /// a pane on P1 with the test patch read
    fn pane() -> (PatchPane, Vec<PatchSlot>) {
        let mut p = PatchPane::default();
        p.open();
        let slots = vec![slot("d1", false, &[("drums", 0, NONE_QUEUED, 3), ("seq", 0, NONE_QUEUED, 1)]), PatchSlot::default(), PatchSlot::default(), PatchSlot::default()];
        p.models[0] = ModelState::Ready("d1".into(), parse_model(TEXT).unwrap());
        (p, slots)
    }

    fn calls(effects: &[PEffect]) -> Vec<String> {
        effects.iter().filter_map(|e| if let PEffect::Action { json, .. } = e { Some(json.clone()) } else { None }).collect()
    }

    #[test]
    fn the_model_reads_sequencers_knobs_and_drops_cells_out_of_range() {
        let m = parse_model(TEXT).unwrap();
        assert_eq!(m.seqs.len(), 2);
        let d = &m.seqs[0];
        assert_eq!((d.kind, d.steps, d.rows, d.name.as_str()), (SeqKind::Gate, 16, 4, "drums"));
        assert_eq!(d.cell(0, 0, 0), Some(1.0));
        assert_eq!(d.cell(0, 4, 1), Some(0.5));
        assert_eq!(d.patterns[0].cells.len(), 2, "step 99 and row 9 do not exist");
        assert_eq!(d.patterns.len(), PATTERNS);
        assert_eq!(m.seqs[1].kind, SeqKind::Mono);
        assert_eq!(m.knob_defaults, vec![("cutoff".to_string(), 0.3)]);
        assert!(parse_model("{").is_err() && parse_model("{}").is_err());
    }

    #[test]
    fn rows_are_labelled_by_drum_name_or_scale_degree() {
        let m = parse_model(TEXT).unwrap();
        assert_eq!(m.seqs[0].row_label(0), "kick");
        assert_eq!(m.seqs[0].row_label(3), "clap");
        // A minor from MIDI 45 (A2): degree 1 is A2, 3 is C3, 8 is A3
        assert_eq!(m.seqs[1].row_label(0), "1 A2");
        assert_eq!(m.seqs[1].row_label(2), "3 C3");
        assert_eq!(m.seqs[1].row_label(7), "8 A3");
        assert_eq!(note_name(60), "C4");
        let mut unknown = m.seqs[1].clone();
        unknown.scale = "wonky".into();
        assert_eq!(unknown.row_label(2), "3");
        let mut unnamed = m.seqs[0].clone();
        unnamed.row_names.clear();
        assert_eq!(unnamed.row_label(1), "row 2");
    }

    #[test]
    fn movement_stays_inside_the_grid_and_w_b_jump_beats() {
        let (mut p, slots) = pane();
        p.on_key(&k('1'), &slots);
        assert!(p.grid.is_some());
        for c in ['h', 'k'] {
            p.on_key(&k(c), &slots);
        }
        let g = p.grid.clone().unwrap();
        assert_eq!((g.cx, g.cy), (0, 0));
        for _ in 0..3 {
            p.on_key(&k('l'), &slots);
        }
        p.on_key(&k('j'), &slots);
        let g = p.grid.clone().unwrap();
        assert_eq!((g.cx, g.cy), (3, 1));
        p.on_key(&k('w'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().cx, 4);
        p.on_key(&k('w'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().cx, 8);
        p.on_key(&k('b'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().cx, 4);
        p.on_key(&k('l'), &slots);
        p.on_key(&k('b'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().cx, 4);
        p.on_key(&k('0'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().cx, 0);
        p.on_key(&shifted('4'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().cx, 15);
        for _ in 0..20 {
            p.on_key(&code(KeyCode::Right), &slots);
            p.on_key(&code(KeyCode::Down), &slots);
        }
        let g = p.grid.clone().unwrap();
        assert_eq!((g.cx, g.cy), (15, 3), "16 steps, 4 rows");
    }

    #[test]
    fn x_toggles_and_the_call_carries_the_cell() {
        let (mut p, slots) = pane();
        p.on_key(&k('1'), &slots);
        // (0,0) is a kick on pattern A: x clears it
        let off = p.on_key(&k('x'), &slots);
        let c = calls(&off);
        assert_eq!(c.len(), 1);
        let v: Value = serde_json::from_str(&c[0]).unwrap();
        assert_eq!((v["op"].as_str(), v["seq"].as_str(), v["pattern"].as_u64(), v["step"].as_u64(), v["row"].as_u64(), v["on"].as_bool()), (Some("cell"), Some("drums"), Some(0), Some(0), Some(0), Some(false)));
        assert_eq!(p.seq().unwrap().cell(0, 0, 0), None);
        // again: back on, at the full default velocity
        let on = calls(&p.on_key(&k('x'), &slots));
        let v: Value = serde_json::from_str(&on[0]).unwrap();
        assert_eq!((v["on"].as_bool(), v["velocity"].as_f64()), (Some(true), Some(1.0)));
        assert_eq!(p.seq().unwrap().cell(0, 0, 0), Some(1.0));
    }

    #[test]
    fn digits_set_the_velocity_of_a_drum_cell_and_the_next_x_uses_it() {
        assert_eq!(velocity_of_digit(1), 0.11);
        assert_eq!(velocity_of_digit(5), 0.56);
        assert_eq!(velocity_of_digit(9), 1.0);
        let (mut p, slots) = pane();
        p.on_key(&k('1'), &slots);
        // an empty cell turns on at the digit's velocity
        p.on_key(&k('l'), &slots);
        let c = calls(&p.on_key(&k('3'), &slots));
        let v: Value = serde_json::from_str(&c[0]).unwrap();
        assert_eq!((v["on"].as_bool(), v["velocity"].as_f64()), (Some(true), Some(0.33)));
        // a cell that is on changes velocity, without a toggle
        p.on_key(&k('7'), &slots);
        assert_eq!(p.seq().unwrap().cell(0, 1, 0), Some(0.78));
        // x on another cell remembers the last velocity
        p.on_key(&k('l'), &slots);
        p.on_key(&k('x'), &slots);
        assert_eq!(p.seq().unwrap().cell(0, 2, 0), Some(0.78));
    }

    #[test]
    fn a_note_step_holds_one_note_and_has_no_velocity() {
        let (mut p, slots) = pane();
        p.on_key(&k('1'), &slots);
        p.on_key(&code(KeyCode::Tab), &slots);
        assert_eq!(p.grid.as_ref().unwrap().seq, 1, "Tab: the next sequencer");
        let (g, seq) = (p.grid.clone().unwrap(), p.seq().unwrap().clone());
        assert_eq!(seq.row_at(g.cy), 8, "the top line is the highest note");
        // step 0 has row 3: moving to row 5 at step 0 and toggling replaces it
        for _ in 0..3 {
            p.on_key(&k('j'), &slots);
        }
        // cy 3 = data row 5
        let c = calls(&p.on_key(&k('x'), &slots));
        assert_eq!(c.len(), 2, "clear the old note, set the new one: {c:?}");
        assert!(c[0].contains("\"row\":3") && c[0].contains("\"on\":false"));
        assert!(c[1].contains("\"row\":5") && c[1].contains("\"on\":true"));
        let s = p.seq().unwrap();
        assert_eq!((s.cell(0, 0, 3), s.cell(0, 0, 5)), (None, Some(1.0)));
        // digits are for drums
        let r = p.on_key(&k('5'), &slots);
        assert!(matches!(&r[0], PEffect::Note(n) if n.contains("drum")));
    }

    #[test]
    fn capital_letters_choose_the_pattern_and_enter_queues_it() {
        let (mut p, slots) = pane();
        p.on_key(&k('1'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().pattern, 0);
        p.on_key(&shifted('c'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().pattern, 2);
        // edits go to the edited pattern
        let c = calls(&p.on_key(&k('x'), &slots));
        assert!(c[0].contains("\"pattern\":2"));
        let q = p.on_key(&code(KeyCode::Enter), &slots);
        let v: Value = serde_json::from_str(&calls(&q)[0]).unwrap();
        assert_eq!((v["op"].as_str(), v["seq"].as_str(), v["pattern"].as_u64(), v["now"].as_bool()), (Some("pattern"), Some("drums"), Some(2), Some(false)));
        assert!(matches!(&q[1], PEffect::Note(n) if n.contains("C queued") && n.contains("next bar")));
        // H is the last one
        p.on_key(&shifted('h'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().pattern, 7);
    }

    #[test]
    fn p_toggles_live_and_esc_steps_back() {
        let (mut p, mut slots) = pane();
        assert_eq!(p.on_key(&k('p'), &slots), vec![PEffect::Live(0, true)]);
        slots[0].live = true;
        assert_eq!(p.on_key(&k('p'), &slots), vec![PEffect::Live(0, false)]);
        p.sel = 2;
        assert!(matches!(&p.on_key(&k('p'), &slots)[0], PEffect::Note(n) if n.contains("empty")));
        p.sel = 0;
        p.on_key(&k('1'), &slots);
        assert!(p.grid.is_some());
        assert!(p.on_key(&code(KeyCode::Esc), &slots).is_empty());
        assert!(p.grid.is_none(), "Esc leaves the grid first");
        assert_eq!(p.on_key(&code(KeyCode::Esc), &slots), vec![PEffect::Close]);
    }

    #[test]
    fn undo_and_redo_follow_the_cells_and_call_the_daemon() {
        let (mut p, slots) = pane();
        p.on_key(&k('1'), &slots);
        p.on_key(&k('x'), &slots); // clears the kick
        p.on_key(&k('l'), &slots);
        p.on_key(&k('x'), &slots); // sets (1,0)
        assert_eq!(p.seq().unwrap().cell(0, 1, 0), Some(1.0));
        assert_eq!(p.on_key(&k('u'), &slots), vec![PEffect::Undo(0)]);
        assert_eq!(p.seq().unwrap().cell(0, 1, 0), None);
        assert_eq!(p.seq().unwrap().cell(0, 0, 0), None, "only the last edit is undone");
        let ctrl_r = NKey { ctrl: true, ..k('r') };
        assert_eq!(p.on_key(&ctrl_r, &slots), vec![PEffect::Redo(0)]);
        assert_eq!(p.seq().unwrap().cell(0, 1, 0), Some(1.0));
        p.on_key(&k('u'), &slots);
        p.on_key(&k('u'), &slots);
        assert_eq!(p.seq().unwrap().cell(0, 0, 0), Some(1.0));
        // a knob edit in between takes one undo that leaves the grid alone
        p.note_other_edit(0);
        p.note_other_edit(0);
        p.on_key(&k('u'), &slots);
        assert_eq!(p.seq().unwrap().cell(0, 0, 0), Some(1.0));
        // an empty history still asks the daemon (it may know edits from elsewhere)
        let mut fresh = PatchPane::default();
        fresh.models[0] = ModelState::Ready("d1".into(), parse_model(TEXT).unwrap());
        assert_eq!(fresh.on_key(&k('u'), &slots), vec![PEffect::Undo(0)]);
    }

    #[test]
    fn digits_pick_a_slot_in_the_overview_and_tab_walks_the_sequencers_across_slots() {
        let (mut p, mut slots) = pane();
        let r = p.on_key(&k('2'), &slots);
        assert_eq!(p.sel, 1);
        assert!(matches!(&r[0], PEffect::Note(n) if n.contains("P2 is empty") && n.contains(":patch load 2")));
        assert!(p.grid.is_none());
        slots[1] = slot("seq2", false, &[("seq", 0, NONE_QUEUED, 0)]);
        p.models[1] = ModelState::Ready("seq2".into(), parse_model(TEXT).unwrap());
        p.on_key(&k('2'), &slots);
        assert!(p.grid.is_some());
        // P2 has two sequencers (the test patch): Tab, Tab goes on to P1's
        p.on_key(&code(KeyCode::Tab), &slots);
        assert_eq!((p.sel, p.grid.as_ref().unwrap().seq), (1, 1));
        p.on_key(&code(KeyCode::Tab), &slots);
        assert_eq!((p.sel, p.grid.as_ref().unwrap().seq), (0, 0));
        // Shift+Tab goes back
        p.on_key(&NKey { shift: true, ..code(KeyCode::Tab) }, &slots);
        assert_eq!((p.sel, p.grid.as_ref().unwrap().seq), (1, 1));
        // a patch without sequencers says so
        p.grid = None;
        p.models[1] = ModelState::Ready("seq2".into(), PatchModel::default());
        let r = p.on_key(&k('2'), &slots);
        assert!(matches!(&r[0], PEffect::Note(n) if n.contains("no sequencer")));
    }

    #[test]
    fn opening_a_grid_starts_on_the_playing_pattern() {
        let (mut p, mut slots) = pane();
        slots[0].sequencers[0].1 = 3;
        p.on_key(&k('1'), &slots);
        assert_eq!(p.grid.as_ref().unwrap().pattern, 3);
    }

    #[test]
    fn syncing_reads_a_patch_when_it_loads_and_again_when_the_unsaved_flag_clears() {
        let mut p = PatchPane::default();
        let mut slots = vec![PatchSlot::default(); 4];
        let mut asked = Vec::new();
        slots[0] = slot("d1", false, &[]);
        p.sync(&slots, |i, n, _| asked.push((i, n)));
        assert_eq!(asked, vec![(0, "d1".to_string())]);
        assert_eq!(p.models[0], ModelState::Pending("d1".into()));
        // nothing new while it is pending or unchanged
        p.sync(&slots, |i, n, _| asked.push((i, n)));
        assert_eq!(asked.len(), 1);
        p.models[0] = ModelState::Ready("d1".into(), PatchModel::default());
        // dirty, then clean again (saved or undone): read again
        slots[0].dirty = true;
        p.sync(&slots, |i, n, _| asked.push((i, n)));
        assert_eq!(asked.len(), 1);
        slots[0].dirty = false;
        p.sync(&slots, |i, n, _| asked.push((i, n)));
        assert_eq!(asked.len(), 2);
        // another patch in the slot, and an unload
        slots[0].name = "acid".into();
        p.sync(&slots, |i, n, _| asked.push((i, n)));
        assert_eq!(asked.last().unwrap().1, "acid");
        slots[0] = PatchSlot::default();
        p.sync(&slots, |i, n, _| asked.push((i, n)));
        assert_eq!(p.models[0], ModelState::None);
    }

    #[test]
    fn a_fetched_source_becomes_the_model_and_a_stale_one_is_dropped() {
        let mut p = PatchPane::default();
        let mut slots = vec![PatchSlot::default(); 4];
        slots[0] = slot("d1", false, &[]);
        let mut tx = None;
        p.sync(&slots, |_, _, t| tx = Some(t));
        let tx = tx.unwrap();
        tx.send(Fetched::Source { slot: 0, name: "old".into(), text: Ok(TEXT.into()) }).unwrap();
        p.sync(&slots, |_, _, _| {});
        assert!(matches!(p.models[0], ModelState::Pending(_)), "an answer for another patch is ignored");
        tx.send(Fetched::Source { slot: 0, name: "d1".into(), text: Ok(TEXT.into()) }).unwrap();
        p.sync(&slots, |_, _, _| {});
        assert_eq!(p.model(0).unwrap().seqs.len(), 2);
        tx.send(Fetched::Source { slot: 0, name: "d1".into(), text: Err("gone".into()) }).unwrap();
        p.sync(&slots, |_, _, _| {});
        assert!(matches!(&p.models[0], ModelState::Failed(_, e) if e == "gone"));
    }

    #[test]
    fn help_and_unknown_keys() {
        let (mut p, slots) = pane();
        assert_eq!(p.on_key(&shifted('/'), &slots), vec![PEffect::Help]);
        assert!(p.on_key(&k('z'), &slots).is_empty());
        let alt = NKey { alt: true, ..k('x') };
        assert!(p.on_key(&alt, &slots).is_empty());
    }
}
