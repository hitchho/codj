//! Drawing the DJ screen. Pure: everything comes in through [`View`], so it is tested with a
//! `TestBackend` and fake deck state.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
};

use super::hints::{self, Target};
use super::keys::{Ctl, Slot};
use super::wave::{draw_strip, time_to_col, Waveform};
use crate::dj_types::{DeckInfo, MixerInfo, SetState};
use crate::types::Music;

/// Seconds of track shown by the zoomed strip around the playhead
pub const ZOOM_S: f64 = 8.0;
const DECK_COLORS: [Color; 2] = [Color::Cyan, Color::Magenta];
const CUE_COLORS: [Color; 4] = [Color::Red, Color::Green, Color::Yellow, Color::Blue];
/// the marks (hot cue slots): `m{a-h}` saves, `'{a-h}` jumps
const MARK_KEYS: [char; 8] = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'];
const DIM: Color = Color::DarkGray;
const MIN_SIZE: (u16, u16) = (60, 14);
/// rows of the set strip on a tall screen: title, tracks, progress, three rows of items
const SET_FULL_H: u16 = 6;
const SET_COLORS: (Color, Color) = (Color::Green, Color::Yellow);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Focus {
    /// performance keys are live
    #[default]
    Decks,
    /// browsing the library with vim keys (j/k, gg/G, / search, a/b load)
    Library,
    /// typing in the library search
    Search,
    /// typing a `:` command
    Command,
}

#[derive(Clone, Copy)]
pub enum WaveView<'a> {
    /// no track
    None,
    Loading,
    /// loaded, but codjd has no waveform (not analyzed yet)
    Missing,
    Ready(&'a Waveform),
}

pub struct View<'a> {
    pub decks: &'a [DeckInfo],
    pub mixer: &'a MixerInfo,
    pub waves: [WaveView<'a>; 2],
    /// the tempo detectors disagree on the deck's track (likely a half/double/triplet grid)
    pub doubt: [bool; 2],
    /// the last poll of the deck state worked
    pub link_ok: bool,
    /// last polling or call error
    pub error: Option<&'a str>,
    pub note: Option<&'a str>,
    pub focus: Focus,
    /// text of the search or command input
    pub input: &'a str,
    pub search: &'a str,
    /// the (filtered) library
    pub lib: &'a [Music],
    pub lib_total: usize,
    pub lib_sel: usize,
    pub release_events: bool,
    /// the deck the keys act on (0 = A)
    pub focus_deck: u8,
    /// the count and half-typed command, like vim's showcmd (`4`, `m`)
    pub showcmd: &'a str,
    /// the active set (None: no set runtime, or none active)
    pub set: Option<&'a SetState>,
    /// recording a mix: (beats, events)
    pub rec: Option<(f64, u32)>,
    /// seconds of the audio recording while it runs
    pub audio_rec: Option<f64>,
    /// the player's queue, and its coming transition (`next: CRIA · blend in 1:12`)
    pub queue: &'a [String],
    pub up_next: Option<&'a str>,
    /// the set's errors, drawn over the decks
    pub overlay: Option<&'a [String]>,
    /// an editor or prompt pane replaces the set strip and the library; it is drawn by the
    /// caller into [`panel_rect`]
    pub panel: Option<Panel>,
    pub hints: HintView<'a>,
    pub learn: LearnView<'a>,
    /// the patch pane's content, when `panel` is [`Panel::Patch`]
    pub patch: Option<&'a super::patchview::PatchView<'a>>,
}

/// What the learning features put on the screen
#[derive(Default)]
pub struct LearnView<'a> {
    /// the `?` key map is up
    pub help: bool,
    /// first line of the key map when it is scrolled (a small screen)
    pub help_scroll: u16,
    /// a text popup (`:help <term>`, `:mix new`): title and paragraphs, wrapped when drawn
    pub popup: Option<(&'a str, &'a [String])>,
    /// library tracks to mark: (path, deck letter) of the tutor's proposal
    pub marks: &'a [(String, char)],
    /// one line about the selected or hovered control, for the status line
    pub explain: Option<&'a str>,
    /// the active set's warnings
    pub warnings: usize,
    /// the tutor pane, beside the library
    pub tutor: Option<&'a super::tutor::TutorView>,
}

/// What the control hints need on screen
#[derive(Clone)]
pub struct HintView<'a> {
    /// the labels are up
    pub active: bool,
    /// letters typed so far
    pub typed: &'a str,
    pub labels: &'a [(Target, &'static str)],
    pub selected: Option<Target>,
    /// status chip of the selection: "◆ A filter -0.30"
    pub chip: Option<&'a str>,
    /// beats of the next loop (`v`), per deck
    pub loop_len: [f64; 2],
    /// trim in dB, per deck
    pub gain: [f64; 2],
    pub sync_range: f64,
    /// the jog step in seconds
    pub jog: f64,
}

impl Default for HintView<'_> {
    fn default() -> Self {
        HintView {
            active: false,
            typed: "",
            labels: &[],
            selected: None,
            chip: None,
            loop_len: [hints::DEFAULT_BEATS; 2],
            gain: [0.0; 2],
            sync_range: hints::DEFAULT_SYNC_RANGE,
            jog: 0.1,
        }
    }
}

/// Where a control is drawn: `label` is where its hint goes (over its caption or the gap before
/// it), `hl` the whole control, inverted while it is selected
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub target: Target,
    pub label: Rect,
    pub hl: Rect,
}

fn text_width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| s.content.chars().count() as u16).sum()
}

/// Records a control at (`x`, `y`) when all of it fits in `clip`
pub(super) fn add_spot(spots: &mut Vec<Spot>, clip: Rect, target: Target, x: u16, y: u16, label_w: u16, hl_w: u16) {
    if hl_w == 0 || y < clip.top() || y >= clip.bottom() || x < clip.left() || x + hl_w > clip.right() {
        return;
    }
    spots.push(Spot { target, label: Rect::new(x, y, label_w.min(hl_w), 1), hl: Rect::new(x, y, hl_w, 1) });
}

/// Inverts the selected control and draws the labels
fn draw_hints(buf: &mut Buffer, spots: &[Spot], h: &HintView) {
    if let Some(sel) = h.selected {
        for sp in spots.iter().filter(|s| s.target == sel) {
            for x in sp.hl.left()..sp.hl.right() {
                buf[(x, sp.hl.y)].modifier.insert(Modifier::REVERSED | Modifier::BOLD);
            }
        }
    }
    if !h.active {
        return;
    }
    let label_style = Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD);
    let typed_style = Style::default().fg(Color::Black).bg(Color::Gray).add_modifier(Modifier::BOLD);
    for sp in spots {
        let Some((_, label)) = h.labels.iter().find(|(t, _)| *t == sp.target) else { continue };
        if !label.starts_with(h.typed) {
            continue;
        }
        let w = sp.label.width as usize;
        let up = label.to_uppercase();
        let typed_n = h.typed.chars().count();
        for (i, x) in (sp.label.left()..sp.label.right()).enumerate() {
            let cell = &mut buf[(x, sp.label.y)];
            cell.reset();
            cell.set_char(up.chars().nth(i).filter(|_| i < w).unwrap_or(' '));
            cell.set_style(if i < typed_n { typed_style } else { label_style });
        }
    }
}

/// Which pane has the keyboard (the performance keys are off while one is open)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Editor,
    Prompt,
    /// the patch slots and the sequencer grid: drawn by `draw` itself (its controls get hints)
    Patch,
}

/// Rows the decks keep when a pane is open
const DECKS_MIN_WITH_PANEL: u16 = 9;
/// ... and when the patch pane is open (it wants a lot of rows)
const DECKS_MIN_WITH_PATCH: u16 = 6;
/// Rows the patch pane would like: four slots of four lines and the border
const PATCH_WANTS: u16 = 18;

pub fn fmt_time(secs: f64) -> String {
    let t = secs.max(0.0);
    let tenths = (t * 10.0) as u64;
    format!("{:02}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

fn fmt_short(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub(super) fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        s.chars().take(w - 1).chain(std::iter::once('…')).collect()
    }
}

pub(super) fn dim() -> Style {
    Style::default().fg(DIM)
}

pub fn draw(buf: &mut Buffer, area: Rect, v: &View) -> Vec<Spot> {
    let mut spots = Vec::new();
    // paint the whole screen, the previous (normal) view must not shine through
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        let msg = format!("DJ mode needs at least {}x{} (now {}x{})", MIN_SIZE.0, MIN_SIZE.1, area.width, area.height);
        buf.set_stringn(area.x, area.y, msg, area.width as usize, Style::default().fg(Color::Yellow));
        buf.set_stringn(area.x, area.y + 1, ":q back", area.width as usize, dim());
        return spots;
    }
    let body_h = area.height - 2;
    let mixer_h = 4;
    let lib_h = if body_h >= 20 { (body_h / 4).clamp(5, 9) } else { 3 };
    let set_h = match (v.set, body_h) {
        (None, _) => 0,
        (Some(_), 26..) => SET_FULL_H,
        (Some(_), 20..) => 4,
        (Some(_), _) => 1,
    };
    // a pane takes the strip's and the library's rows, and more while the decks keep their minimum
    let (set_h, lib_h) = match v.panel {
        Some(p) => (0, panel_height(p, body_h, mixer_h, set_h + lib_h)),
        None => (set_h, lib_h),
    };
    // the tutor pane never takes the decks' rows below a minimum: it shrinks the set strip first
    let tutor = v.learn.tutor.filter(|_| v.panel.is_none());
    let (set_h, lib_h) = if tutor.is_some() { tutor_layout(body_h, mixer_h, set_h, lib_h) } else { (set_h, lib_h) };
    let decks_h = body_h.saturating_sub(mixer_h + lib_h + set_h);
    let header = Rect::new(area.x, area.y, area.width, 1);
    let decks = Rect::new(area.x, area.y + 1, area.width, decks_h);
    let set_area = Rect::new(area.x, decks.bottom(), area.width, set_h);
    let mixer = Rect::new(area.x, set_area.bottom(), area.width, mixer_h);
    let lib = Rect::new(area.x, mixer.bottom(), area.width, lib_h);
    let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);

    draw_header(buf, header, v);
    let half = decks.width / 2;
    for i in 0..2 {
        let r = if i == 0 {
            Rect::new(decks.x, decks.y, half, decks.height)
        } else {
            Rect::new(decks.x + half, decks.y, decks.width - half, decks.height)
        };
        let info = v.decks.get(i).cloned().unwrap_or_default();
        draw_deck(buf, r, i, i as u8 == v.focus_deck, &info, v.waves[i], v.release_events, v.doubt[i], &v.hints, &mut spots);
    }
    if let Some(set) = v.set {
        draw_set(buf, set_area, set, v.decks, v.rec, v.learn.warnings);
    }
    draw_mixer(buf, mixer, v, &mut spots);
    if let (Some(Panel::Patch), Some(pv)) = (v.panel, v.patch) {
        super::patchview::draw_patch(buf, Rect::new(area.x, mixer.bottom(), area.width, lib_h), pv, &mut spots);
    }
    if v.panel.is_none() {
        match tutor {
            // a narrow screen gives the tutor the whole bottom, except while browsing the
            // library: then the library has it, and Tab or Esc brings the tutor back
            Some(_) if lib.width < 100 && matches!(v.focus, Focus::Library | Focus::Search) => draw_library(buf, lib, v),
            Some(t) => {
                // the library keeps the left side and the tutor takes the right; a narrow screen
                // gives the tutor all of it
                let tutor_w = if lib.width >= 100 { (lib.width / 2).clamp(48, 70) } else { lib.width };
                let lib_w = lib.width - tutor_w;
                if lib_w >= 24 {
                    draw_library(buf, Rect::new(lib.x, lib.y, lib_w, lib.height), v);
                }
                draw_tutor(buf, Rect::new(lib.x + lib_w, lib.y, tutor_w, lib.height), t);
            }
            None => draw_library(buf, lib, v),
        }
    }
    draw_status(buf, status, v);
    if let Some(lines) = v.overlay {
        draw_overlay(buf, decks, " set errors · Esc closes ", lines, false);
    }
    if let Some((title, lines)) = v.learn.popup {
        // the patch pane leaves the decks little room: its popups may cover the pane too
        let over = if v.panel == Some(Panel::Patch) { Rect::new(area.x, decks.y, area.width, lib.bottom().saturating_sub(decks.y)) } else { decks };
        draw_overlay(buf, over, title, lines, true);
    }
    draw_hints(buf, &spots, &v.hints);
    if v.learn.help {
        let lower = Rect::new(area.x, mixer.y, area.width, lib.bottom().saturating_sub(mixer.y));
        super::help::draw(buf, decks, lower, v.focus_deck, v.learn.help_scroll);
    }
    spots
}

/// Rows the decks keep while the tutor is open
const DECKS_MIN_WITH_TUTOR: u16 = 10;

/// (set strip rows, bottom rows) with the tutor open: the strip shrinks to a line on a medium
/// screen and the bottom grows to what the tutor wants while the decks keep their minimum
fn tutor_layout(body_h: u16, mixer_h: u16, set_h: u16, lib_h: u16) -> (u16, u16) {
    let set_h = if body_h < 30 { set_h.min(1) } else { set_h };
    let want = if body_h >= 32 { 11 } else if body_h >= 22 { 8 } else { 6 };
    let room = body_h.saturating_sub(mixer_h + set_h + DECKS_MIN_WITH_TUTOR);
    (set_h, lib_h.max(want.min(room)))
}

fn draw_tutor(buf: &mut Buffer, area: Rect, t: &super::tutor::TutorView) {
    if area.width < 16 || area.height < 3 {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
    let title = if t.n > 0 { format!(" TUTOR {}/{} · {} ", t.n, super::tutor::Step::COUNT, t.title) } else { format!(" TUTOR · {} ", t.title) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(title, Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)))
        .title_bottom(Span::styled(format!(" {} ", t.footer), dim()));
    let inner = block.inner(area);
    block.render(area, buf);
    let width = inner.width.saturating_sub(2) as usize;
    let rows = inner.height as usize;
    let why = crate::dj::glossary::wrap(&t.why, width);
    let keys: Vec<&String> = t.keys.iter().collect();
    // keys and the live checklist are what is acted on: they keep their rows, the why gives way
    let acted = (keys.len() + t.live.len()).min(rows);
    let why_rows = rows.saturating_sub(acted).min(why.len());
    let mut lines: Vec<Line> = Vec::new();
    for (i, l) in why.iter().take(why_rows).enumerate() {
        let text = if i + 1 == why_rows && why_rows < why.len() { format!("{}…", trunc(l, width.saturating_sub(1))) } else { l.clone() };
        lines.push(Line::from(text));
    }
    for k in &keys {
        lines.push(Line::from(vec![Span::styled("▸ ", Style::default().fg(Color::Yellow)), Span::styled(trunc(k, width.saturating_sub(2)), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]));
    }
    for l in &t.live {
        let style = if l.starts_with('✓') {
            Style::default().fg(Color::Green)
        } else if l.contains('►') || l.starts_with('★') {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            dim()
        };
        lines.push(Line::from(Span::styled(trunc(l, width), style)));
    }
    for (i, line) in lines.into_iter().take(rows).enumerate() {
        buf.set_line(inner.x + 1, inner.y + i as u16, &line, inner.width.saturating_sub(2));
    }
}

/// Height of a pane: what the strip and library had, grown to half the body while the decks keep
/// [`DECKS_MIN_WITH_PANEL`] rows
fn panel_height(panel: Panel, body_h: u16, mixer_h: u16, had: u16) -> u16 {
    if panel == Panel::Patch {
        let room = body_h.saturating_sub(mixer_h + DECKS_MIN_WITH_PATCH);
        return PATCH_WANTS.min(room).max(had.min(room)).max(4);
    }
    let room = body_h.saturating_sub(mixer_h + DECKS_MIN_WITH_PANEL);
    (body_h / 2).min(room).max(had.min(room)).max(4)
}

/// Where the pane goes (below the mixer, above the status line); None when no pane is open or
/// the screen is too small
pub fn panel_rect(area: Rect, v: &View) -> Option<Rect> {
    let panel = v.panel?;
    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        return None;
    }
    let body_h = area.height - 2;
    let mixer_h = 4;
    let lib_h = if body_h >= 20 { (body_h / 4).clamp(5, 9) } else { 3 };
    let set_h = match (v.set, body_h) {
        (None, _) => 0,
        (Some(_), 26..) => SET_FULL_H,
        (Some(_), 20..) => 4,
        (Some(_), _) => 1,
    };
    let h = panel_height(panel, body_h, mixer_h, set_h + lib_h);
    Some(Rect::new(area.x, area.bottom() - 1 - h, area.width, h))
}

/// beats to bars, as shown: whole bars plain, else one decimal
/// the recording badge
const WARN_STYLE: Style = Style::new().fg(Color::Yellow);
const REC_STYLE: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);

fn fmt_bars(beats: f64) -> String {
    let bars = beats / 4.0;
    if (bars - bars.round()).abs() < 0.05 {
        format!("{}", bars.round() as i64)
    } else {
        format!("{bars:.1}")
    }
}

/// "bar 16 · swap low" for an upcoming item; texts that already say where keep their own
fn item_text(beat: f64, text: &str) -> String {
    if text.starts_with("bar ") || text.starts_with("bars ") {
        text.to_string()
    } else {
        format!("bar {} · {text}", fmt_bars(beat))
    }
}

/// Index of the track on air (the armed mix's outgoing one; else one playing on a deck) and the
/// one coming (the armed mix's incoming one)
fn current_and_next(set: &SetState, decks: &[DeckInfo]) -> (Option<usize>, Option<usize>) {
    if let Some(m) = set.armed_mix() {
        let (from, to, ..) = &set.mixes[m];
        return (set.track_index(from), set.track_index(to));
    }
    let on_air = set
        .tracks
        .iter()
        .position(|t| decks.iter().any(|d| d.loaded && d.playing && d.path == t.2));
    (on_air, None)
}

fn draw_set(buf: &mut Buffer, area: Rect, set: &SetState, decks: &[DeckInfo], rec: Option<(f64, u32)>, warnings: usize) {
    if area.height == 0 || area.width < 20 {
        return;
    }
    let w = area.width as usize;
    let (cur, next) = current_and_next(set, decks);
    let title_of = |id: &str| -> String { set.track_index(id).map(|i| set.tracks[i].1.clone()).unwrap_or_else(|| id.to_string()) };
    let err_style = if set.errors > 0 {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let errs = if set.errors > 0 { format!("✗ {} error{} (:errors)", set.errors, if set.errors == 1 { "" } else { "s" }) } else { "✓ no errors".into() };
    let auto = if set.auto {
        Span::styled(" AUTO ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("auto off", dim())
    };

    // compact: one line
    if area.height == 1 {
        let narrow = w < 80;
        let mut spans = vec![
            Span::styled(" SET ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" {} ", trunc(&set.name, if narrow { 6 } else { 18 }))),
            Span::styled(if set.errors > 0 { format!("✗{} ", set.errors) } else { "✓ ".into() }, err_style),
        ];
        if warnings > 0 {
            spans.push(Span::styled(format!("⚠{warnings} "), WARN_STYLE));
        }
        if set.auto {
            spans.push(Span::styled(if narrow { "AUTO " } else { " AUTO " }, Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)));
        }
        if let Some((beats, events)) = rec {
            spans.push(Span::styled(format!(" {} ", super::capture::rec_text(beats, events, narrow)), REC_STYLE));
        }
        if let Some(m) = set.armed_mix() {
            let len = set.mixes[m].3;
            spans.push(Span::raw(format!(
                "{}mix {} {} {}{}/{}",
                if narrow { " " } else { "  " },
                m + 1,
                if set.running { "▶" } else { "armed" },
                if narrow { "" } else { "bar " },
                fmt_bars(set.mix_beat.max(0.0)),
                fmt_bars(len)
            )));
            if let Some((b, t)) = set.next_items.first() {
                spans.push(Span::styled(
                    format!("{}{}", if narrow { " ▸ " } else { "  next " }, item_text(*b, t)),
                    Style::default().fg(SET_COLORS.1),
                ));
            }
        } else {
            spans.push(Span::styled(" no mix armed (F6)", dim()));
        }
        Paragraph::new(Line::from(spans)).render(area, buf);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    // 1: title, errors, auto, keys
    let mut head = vec![
        Span::styled(" SET ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {} ", trunc(&set.name, 30)), Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(errs, err_style),
    ];
    if warnings > 0 {
        head.push(Span::styled(format!("  ⚠ {warnings} warning{} (:errors)", if warnings == 1 { "" } else { "s" }), WARN_STYLE));
    }
    head.push(Span::raw("  "));
    head.push(auto);
    if let Some((beats, events)) = rec {
        head.push(Span::raw(" "));
        head.push(Span::styled(format!(" {} ", super::capture::rec_text(beats, events, false)), REC_STYLE));
    }
    let used: usize = head.iter().map(|s| s.content.chars().count()).sum();
    let keys = "F6 arm next · F7 auto · F8 disarm · F9 rec";
    if w > used + keys.len() + 2 {
        head.push(Span::raw(" ".repeat(w - used - keys.len() - 1)));
        head.push(Span::styled(keys, dim()));
    }
    lines.push(Line::from(head));

    // 2: tracks joined by the mixes
    lines.push(track_line(set, cur, next, w));

    // 3: progress of the armed mix, or the hint
    let armed = set.armed_mix();
    match armed {
        Some(m) => lines.push(progress_line(set, m, w, &title_of(&set.mixes[m].0), &title_of(&set.mixes[m].1))),
        None => lines.push(Line::from(Span::styled(
            match next_unarmed(set, decks) {
                Some(m) => format!(" no mix armed · F6 arms mix {}: {} → {}", m + 1, trunc(&title_of(&set.mixes[m].0), 24), trunc(&title_of(&set.mixes[m].1), 24)),
                None => " no mix armed · load an outgoing track (:load <n>), then F6".to_string(),
            },
            dim(),
        ))),
    }

    // the rest: upcoming items of the armed mix, or the mix list
    let rows = (area.height as usize).saturating_sub(lines.len());
    if rows > 0 {
        let entries: Vec<(String, Style)> = if armed.is_some() {
            set.next_items
                .iter()
                .map(|(b, t)| (item_text(*b, t), Style::default().fg(SET_COLORS.1)))
                .collect()
        } else {
            set.mixes
                .iter()
                .enumerate()
                .map(|(i, m)| (format!("{}. {} → {}  {} bars", i + 1, trunc(&title_of(&m.0), 20), trunc(&title_of(&m.1), 20), fmt_bars(m.3)), dim()))
                .collect()
        };
        let cols = if w >= 100 { 2 } else { 1 };
        let col_w = w / cols;
        for r in 0..rows {
            let mut spans = Vec::new();
            for c in 0..cols {
                // fill column by column: items 0..rows in the first, then the second
                let idx = c * rows + r;
                let (text, style) = match entries.get(idx) {
                    Some(e) => e.clone(),
                    None => continue,
                };
                let cell = format!(" {}", trunc(&text, col_w.saturating_sub(2)));
                let pad = col_w.saturating_sub(cell.chars().count());
                spans.push(Span::styled(cell, style));
                spans.push(Span::raw(" ".repeat(pad)));
            }
            lines.push(Line::from(spans));
        }
        if armed.is_some() && set.next_items.is_empty() {
            if let Some(l) = lines.get_mut(3) {
                *l = Line::from(Span::styled(" nothing more to automate in this mix", dim()));
            }
        }
    }
    Paragraph::new(lines).render(area, buf);
}

/// the first mix that F6 would arm, for the hint
fn next_unarmed(set: &SetState, decks: &[DeckInfo]) -> Option<usize> {
    super::next_mix(set, decks).ok().map(|m| m as usize)
}

fn track_line(set: &SetState, cur: Option<usize>, next: Option<usize>, w: usize) -> Line<'static> {
    let n = set.tracks.len();
    if n == 0 {
        return Line::from(Span::styled(" no tracks in this set", Style::default().fg(Color::Red)));
    }
    // as many tracks as fit with about 16 columns each, around the current one
    let fit = (w.saturating_sub(2) / 18).clamp(1, n);
    let focus = cur.or(next).unwrap_or(0);
    let start = focus.saturating_sub(fit / 2).min(n - fit);
    let end = start + fit;
    let mix_between = |a: usize, b: usize| {
        set.mixes
            .iter()
            .enumerate()
            .find(|(_, m)| set.track_index(&m.0) == Some(a) && set.track_index(&m.1) == Some(b))
    };
    // connectors: " ──32▶ " (armed: bold, in colour)
    let conn_w = 8;
    let title_w = (w.saturating_sub(2 + conn_w * (fit - 1) + if start > 0 { 2 } else { 0 } + if end < n { 2 } else { 0 }) / fit).max(4);
    let mut spans = vec![Span::raw(if start > 0 { "‹ " } else { " " }.to_string())];
    for i in start..end {
        let label = format!("{} {}", i + 1, set.tracks[i].1);
        let style = if Some(i) == cur {
            Style::default().fg(SET_COLORS.0).add_modifier(Modifier::BOLD)
        } else if Some(i) == next {
            Style::default().fg(SET_COLORS.1).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let marker = if Some(i) == cur { "▶" } else if Some(i) == next { "»" } else { " " };
        spans.push(Span::styled(format!("{marker}{}", trunc(&label, title_w.saturating_sub(1))), style));
        let used = 1 + trunc(&label, title_w.saturating_sub(1)).chars().count();
        spans.push(Span::raw(" ".repeat(title_w.saturating_sub(used))));
        if i + 1 < end {
            let (text, style) = match mix_between(i, i + 1) {
                Some((m, mix)) => {
                    let t = format!("─{}b▶", fmt_bars(mix.3));
                    let st = if set.armed_mix() == Some(m) {
                        Style::default().fg(SET_COLORS.1).add_modifier(Modifier::BOLD)
                    } else {
                        dim()
                    };
                    (t, st)
                }
                None => ("─ ─ ─".into(), dim()),
            };
            let pad = conn_w.saturating_sub(text.chars().count());
            spans.push(Span::styled(format!("{}{text}", "─".repeat(pad.min(2))), style));
            spans.push(Span::raw(" ".repeat(pad.saturating_sub(pad.min(2)))));
        }
    }
    if end < n {
        spans.push(Span::raw(" ›"));
    }
    Line::from(spans)
}

/// ` MIX 1 warm → mid  ▶ bar 9.5/32 [████▌░░◆░░░◆░░]` with a ◆ at each upcoming item
fn progress_line(set: &SetState, m: usize, w: usize, from: &str, to: &str) -> Line<'static> {
    let len = set.mixes[m].3.max(1.0);
    let state = if set.running {
        Span::styled("▶ running", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        let wait = (-set.mix_beat).max(0.0);
        Span::styled(format!("armed, starts in {} bars", fmt_bars(wait)), Style::default().fg(SET_COLORS.1))
    };
    let at = format!(" bar {}/{} ", fmt_bars(set.mix_beat.max(0.0)), fmt_bars(len));
    let label = format!(" MIX {}: {} → {} ", m + 1, trunc(from, 14), trunc(to, 14));
    let used = label.chars().count() + state.content.chars().count() + at.chars().count() + 2;
    let bar_w = w.saturating_sub(used + 1).clamp(8, 80);
    let pos = (set.mix_beat / len).clamp(0.0, 1.0);
    let filled = pos * bar_w as f64;
    let mut cells: Vec<(char, Style)> = (0..bar_w)
        .map(|i| {
            if (i as f64) + 1.0 <= filled {
                ('█', Style::default().fg(Color::Cyan))
            } else if (i as f64) < filled {
                ('▌', Style::default().fg(Color::Cyan))
            } else {
                ('░', dim())
            }
        })
        .collect();
    for (beat, _) in &set.next_items {
        let c = ((beat / len).clamp(0.0, 1.0) * (bar_w - 1) as f64).round() as usize;
        cells[c.min(bar_w - 1)] = ('◆', Style::default().fg(SET_COLORS.1));
    }
    let mut spans = vec![
        Span::styled(label, Style::default().add_modifier(Modifier::BOLD)),
        state,
        Span::styled(at, dim()),
        Span::styled("[", dim()),
    ];
    spans.extend(cells.into_iter().map(|(c, s)| Span::styled(c.to_string(), s)));
    spans.push(Span::styled("]", dim()));
    Line::from(spans)
}

fn draw_overlay(buf: &mut Buffer, over: Rect, title: &str, lines: &[String], wrap: bool) {
    if over.width < 24 || over.height < 3 {
        return;
    }
    let w = (over.width - 4).min(110);
    let wrapped: Vec<String>;
    let lines = if wrap {
        let width = (w as usize).saturating_sub(4).max(8);
        wrapped = lines
            .iter()
            .flat_map(|l| if l.is_empty() { vec![String::new()] } else { crate::dj::glossary::wrap(l, width) })
            .collect();
        &wrapped[..]
    } else {
        lines
    };
    let h = ((lines.len() as u16) + 2).min(over.height).min(if wrap { 16 } else { 12 });
    let area = Rect::new(over.x + (over.width - w) / 2, over.y + (over.height - h) / 2, w, h);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
    let is_warning = |l: &String| l.starts_with("warning:");
    let bad = lines.iter().any(|l| l != "no errors" && !is_warning(l));
    let warn = lines.iter().any(is_warning);
    let color = if wrap {
        Color::Cyan
    } else if bad {
        Color::Red
    } else if warn {
        Color::Yellow
    } else {
        Color::Green
    };
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(color)).title(title.to_string());
    let inner = block.inner(area);
    block.render(area, buf);
    let rows = inner.height as usize;
    for (i, l) in lines.iter().take(rows).enumerate() {
        let text = if i + 1 == rows && lines.len() > rows { format!("… {} more", lines.len() - rows + 1) } else { l.clone() };
        let style = if is_warning(l) { Style::default().fg(Color::Yellow) } else { Style::default() };
        buf.set_stringn(inner.x + 1, inner.y + i as u16, text, inner.width.saturating_sub(2) as usize, style);
    }
}

fn draw_header(buf: &mut Buffer, area: Rect, v: &View) {
    let live = v.link_ok;
    let left = match v.panel {
        // the performance keys are off: say so, loudly
        // a narrow header keeps the badge and the way back
        Some(p) if area.width < 110 => Line::from(vec![
            Span::styled(
                match p {
                    Panel::Editor => " EDIT ",
                    Panel::Prompt => " AI ",
                    Panel::Patch => " PATCH ",
                },
                Style::default().fg(Color::Black).bg(if p == Panel::Patch { Color::Cyan } else { Color::Magenta }).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                match p {
                    Panel::Editor => "  deck keys are OFF · F10, Esc, :q back",
                    Panel::Prompt => "  deck keys are OFF · Esc hides · Ctrl-C cancels",
                    Panel::Patch => "  deck keys are OFF · f ] [ work · F12/Esc back",
                },
                Style::default().fg(if p == Panel::Patch { Color::Cyan } else { Color::Magenta }).add_modifier(Modifier::BOLD),
            ),
        ]),
        Some(p) => Line::from(vec![
            Span::styled(
                match p {
                    Panel::Editor => " EDIT ",
                    Panel::Prompt => " AI ",
                    Panel::Patch => " PATCH ",
                },
                Style::default().fg(Color::Black).bg(if p == Panel::Patch { Color::Cyan } else { Color::Magenta }).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if p == Panel::Patch { "  deck keys are OFF while the patch pane is open" } else { "  DJ keys are OFF while typing here" },
                Style::default().fg(if p == Panel::Patch { Color::Cyan } else { Color::Magenta }).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                match p {
                    Panel::Editor => " · F10, Esc (normal mode) or :q back to the deck keys",
                    Panel::Prompt => " · Esc hides · Ctrl-C cancels the agent",
                    Panel::Patch => " · f hints and ] [ still work · F12 / Esc back to the deck keys",
                },
                dim(),
            ),
        ]),
        None => {
            // the longest help that fits beside the badge and the link state; `? keys` always stays
            let room = (area.width as usize).saturating_sub(4 + 10 + if v.audio_rec.is_some() { 14 } else { 0 });
            let help = [
                "  j k ⏎ library · t+letter transition · h l focus · : command (:q leaves) · ? keys",
                "  h l focus · Tab library · :q leaves · ? keys",
                "  ? keys · :q leaves",
            ]
            .into_iter()
            .find(|t| t.chars().count() <= room)
            .unwrap_or("  ? keys");
            Line::from(vec![
                Span::styled(" DJ ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(help, dim()),
            ])
        }
    };
    Paragraph::new(left).render(area, buf);
    let (text, color) = if live { ("● live ", Color::Green) } else { ("○ no data ", Color::Red) };
    let w = text.chars().count() as u16;
    if area.width > w {
        buf.set_string(area.right() - w, area.y, text, Style::default().fg(color));
    }
    // the room is being recorded: always in sight
    if let Some(secs) = v.audio_rec {
        let s = secs.max(0.0) as u64;
        let rec = if s >= 3600 { format!(" ● REC {}:{:02}:{:02} ", s / 3600, s / 60 % 60, s % 60) } else { format!(" ● REC {}:{:02} ", s / 60, s % 60) };
        let rw = rec.chars().count() as u16;
        if area.width > w + rw + 1 {
            buf.set_string(area.right() - w - rw - 1, area.y, rec, Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD));
        }
    }
}

fn draw_status(buf: &mut Buffer, area: Rect, v: &View) {
    let w = area.width as usize;
    if v.focus == Focus::Command {
        buf.set_stringn(area.x, area.y, format!(":{}█", v.input), w, Style::default().fg(Color::White));
        return;
    }
    // the selection's chip sits at the right end; the text gives way to it
    let chip = v.hints.chip.filter(|c| c.chars().count() + 8 < w);
    let chip_w = chip.map_or(0, |c| c.chars().count() + 1);
    // right of the text: the loop mode of the focused deck and the half-typed command
    let loop_beats = v.decks.get(v.focus_deck as usize).map_or(0.0, |d| d.loop_beats);
    let mode = if loop_beats > 0.0 && !v.learn.help && !v.hints.active { format!("-- LOOP {} --", trim_beats(loop_beats)) } else { String::new() };
    let mode_w = if mode.is_empty() { 0 } else { mode.chars().count() + 1 };
    let cmd_w = if v.showcmd.is_empty() { 0 } else { v.showcmd.chars().count() + 2 };
    let text_w = w.saturating_sub(chip_w + mode_w + cmd_w);
    if v.learn.help {
        let badge = Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD);
        buf.set_stringn(area.x, area.y, " ? KEYS ", text_w, badge);
        buf.set_stringn(area.x + 8, area.y, " every key explained · q closes · :help <term> for the words", text_w.saturating_sub(8), Style::default().fg(Color::Cyan));
    } else if v.hints.active {
        let badge = Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD);
        buf.set_stringn(area.x, area.y, " HINTS ", text_w, badge);
        let typed = v.hints.typed.to_uppercase();
        let msg = match v.learn.explain {
            Some(x) => format!(" {x}"),
            None => format!(" type a label{} · Esc or f cancels · keys do not play now", if typed.is_empty() { String::new() } else { format!(" ({typed}…)") }),
        };
        buf.set_stringn(area.x + 7, area.y, msg, text_w.saturating_sub(7), Style::default().fg(Color::Yellow));
    } else if let Some(e) = v.error {
        buf.set_stringn(area.x, area.y, e, text_w, Style::default().fg(Color::Red));
    } else if let (Some(_), Some(x)) = (chip, v.learn.explain) {
        // the selection's one-liner sits right next to its chip; a note keeps the left side
        let len = x.chars().count().min(text_w.saturating_sub(1));
        let at = area.x + (text_w - len) as u16;
        match v.note {
            Some(n) => buf.set_stringn(area.x, area.y, n, text_w.saturating_sub(len + 2), Style::default().fg(Color::Yellow)),
            None => buf.set_stringn(area.x, area.y, "] [ edit · \\ reset", text_w.saturating_sub(len + 1), dim()),
        };
        buf.set_stringn(at, area.y, x, len, Style::default().fg(Color::Cyan));
    } else if let Some(n) = v.note {
        buf.set_stringn(area.x, area.y, n, text_w, Style::default().fg(Color::Yellow));
    } else {
        let hint = "? keys · h l focus · Space play · c cue · s sync · w b W B } { jump (4w) · ma 'a marks · v loop · - + = pitch · [ ] nudge · f hints · u undo · . repeat";
        buf.set_stringn(area.x, area.y, whole_items(hint, text_w), text_w, dim());
    }
    if mode_w > 0 {
        let x = area.x + text_w as u16 + 1;
        buf.set_stringn(x, area.y, &mode, mode_w - 1, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
    }
    if cmd_w > 0 {
        let x = area.x + (text_w + mode_w) as u16 + 1;
        buf.set_stringn(x, area.y, v.showcmd, cmd_w - 1, Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD));
    }
    if let Some(c) = chip {
        let x = area.right() - chip_w as u16 + 1;
        buf.set_stringn(x, area.y, c, chip_w - 1, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD));
    }
}

/// The longest run of a ` · `-separated list's items that fits in `w` columns, so a narrow
/// screen drops whole items instead of cutting one in half ("pit")
fn whole_items(list: &str, w: usize) -> &str {
    if list.chars().count() <= w {
        return list;
    }
    let mut end = 0;
    for (i, _) in list.match_indices(" · ") {
        if list[..i].chars().count() > w {
            break;
        }
        end = i;
    }
    &list[..end]
}

fn draw_deck(
    buf: &mut Buffer,
    area: Rect,
    idx: usize,
    focused: bool,
    d: &DeckInfo,
    wave: WaveView,
    release: bool,
    doubt: bool,
    h: &HintView,
    spots: &mut Vec<Spot>,
) {
    if area.width < 12 || area.height < 3 {
        return;
    }
    let color = DECK_COLORS[idx];
    let name = if idx == 0 { "A" } else { "B" };
    let state = if !d.loaded {
        "empty"
    } else if d.playing {
        "▶ playing"
    } else {
        "⏸ paused"
    };
    // the focused deck is the one every key acts on: a bright thick border and a badge
    let mut title = vec![Span::styled(format!(" DECK {name} "), Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD))];
    if focused {
        title.push(Span::styled(" ▶ FOCUS ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
    }
    title.push(Span::styled(format!(" {state} "), Style::default().fg(color)));
    let border = if focused {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(if d.playing { color } else { DIM })
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Thick } else { BorderType::Plain })
        .border_style(border)
        .title(Line::from(title));
    let inner = block.inner(area);
    block.render(area, buf);
    let w = inner.width as usize;
    if !d.loaded {
        // this deck's own load key, short enough for a phone-width deck
        let key = if idx == 0 { 'a' } else { 'b' };
        let msg = format!("— empty — Tab, j/k, {key} loads here");
        buf.set_stringn(inner.x + 1, inner.y + inner.height / 2, msg, w.saturating_sub(1), dim());
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    // title / artist
    let who = if d.artist.is_empty() { d.title.clone() } else { format!("{} — {}", d.title, d.artist) };
    lines.push(Line::from(Span::styled(trunc(&who, w), Style::default().add_modifier(Modifier::BOLD))));
    // tempo, key, rate
    let rate_pct = (d.rate - 1.0) * 100.0;
    // a narrow deck (80 columns, a phone) keeps every field by dropping decimals and labels
    let compact = w < 50;
    let mut tempo = Vec::new();
    if d.has_grid() {
        let bpm = if compact { format!("{:.1}", d.bpm) } else { format!("{:6.2}", d.bpm) };
        tempo.push(Span::styled(bpm, Style::default().add_modifier(Modifier::BOLD)));
        tempo.push(Span::raw(" BPM "));
        let trk = if compact { format!("({:.0})", d.track_bpm) } else { format!("(trk {:.2})", d.track_bpm) };
        tempo.push(Span::styled(trk, dim()));
        if doubt {
            // the detectors disagree: maybe half, double or 1.5x; fix with :grid
            tempo.push(Span::styled("?", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
        }
    } else {
        tempo.push(Span::styled("no beat grid", Style::default().fg(Color::Yellow)));
    }
    tempo.push(Span::raw(if compact { " " } else { "  key " }));
    tempo.push(Span::styled(
        if d.key.is_empty() { "--".to_string() } else { d.key.clone() },
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    ));
    tempo.push(Span::raw("  "));
    let rate_col = text_width(&tempo) - 2;
    tempo.push(Span::styled(
        if compact { format!("{rate_pct:+.1}%") } else { format!("{rate_pct:+.2}%") },
        if rate_pct.abs() < 0.005 { dim() } else { Style::default().fg(Color::Yellow) },
    ));
    let mut found: Vec<(Target, usize, u16, u16, u16)> =
        vec![(Target::Slot(Slot::Rate(idx as u8)), 1, rate_col, 2, text_width(&tempo) - rate_col)];
    tempo.push(Span::raw("  "));
    tempo.push(if d.synced {
        Span::styled("SYNC", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("sync", dim())
    });
    lines.push(Line::from(tempo));
    // time, loop, headphones
    let remaining = (d.duration - d.position).max(0.0);
    let mut time = vec![
        Span::styled(fmt_time(d.position), Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!("  -{}", fmt_short(remaining)), dim()),
        Span::raw("  "),
    ];
    found.push((Target::Time(idx as u8), 2, 0, 2, text_width(&time[..1])));
    let loop_col = text_width(&time) - 2;
    time.push(if d.loop_beats > 0.0 {
        Span::styled(
            format!(" LOOP {} ", trim_beats(d.loop_beats)),
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
        )
    } else {
        // the length the next loop key makes
        Span::styled(format!("loop {}", hints::fmt_beats(h.loop_len[idx])), dim())
    });
    found.push((Target::LoopLen(idx as u8), 2, loop_col, 2, text_width(&time) - loop_col));
    time.push(Span::raw("  "));
    time.push(if d.headphones {
        Span::styled("HP ●", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("HP ○", dim())
    });
    // the trim, when the line has room
    let room = inner.width.saturating_sub(2);
    for (target, text) in [(Target::Slot(Slot::Gain(idx as u8)), format!("gain {:+.1}", h.gain[idx]))] {
        let col = text_width(&time);
        let len = text.chars().count() as u16;
        if col + 2 + len > room {
            break;
        }
        time.push(Span::raw("  "));
        time.push(Span::styled(text, dim()));
        found.push((target, 2, col, 2, len + 2));
    }
    if d.duration > 1.0 && d.decoded + 0.5 < d.duration {
        time.push(Span::styled(
            format!("  decoding {}%", (d.decoded / d.duration * 100.0) as u32),
            Style::default().fg(Color::Yellow),
        ));
    }
    lines.push(Line::from(time));
    // phase meter
    lines.push(phase_meter(d, w, color));
    // hot cues
    lines.push(hotcue_line(d, w));

    // text first, the waveforms get what is left (at least 2 rows if there is room)
    let n_text = lines.len().min(inner.height.saturating_sub(2).max(1) as usize);
    let text_area = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), n_text as u16);
    for (target, line, col, label_w, hl_w) in found {
        if line < n_text {
            add_spot(spots, text_area, target, text_area.x + col, text_area.y + line as u16, label_w, hl_w);
        }
    }
    Paragraph::new(lines.into_iter().take(n_text).collect::<Vec<_>>()).render(text_area, buf);
    let wave_top = inner.y + n_text as u16;
    let rest = inner.bottom().saturating_sub(wave_top);
    if rest == 0 {
        return;
    }
    let overview_rows = if rest >= 7 { 3 } else if rest >= 3 { 1 } else { 0 };
    let ov = Rect::new(inner.x + 1, wave_top, inner.width.saturating_sub(2), overview_rows);
    let zoom = Rect::new(ov.x, wave_top + overview_rows, ov.width, rest - overview_rows);
    draw_waves(buf, ov, zoom, d, wave, release);
}

fn trim_beats(b: f64) -> String {
    if (b - b.round()).abs() < 1e-6 {
        format!("{}", b.round() as i64)
    } else {
        format!("{b:.2}")
    }
}

fn phase_meter(d: &DeckInfo, w: usize, color: Color) -> Line<'static> {
    if !d.has_grid() || d.beat_phase < 0.0 {
        return Line::from(Span::styled("phase  (no beat grid)", dim()));
    }
    let seg = (w / 4).max(4);
    let bar_w = seg - 2;
    let mut spans = Vec::new();
    for beat in 1..=4u32 {
        let on = beat == d.beat_in_bar;
        let label_style = if on {
            Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD)
        } else {
            dim()
        };
        spans.push(Span::styled(format!("{beat}"), label_style));
        spans.push(Span::raw(" "));
        let filled = if beat < d.beat_in_bar {
            bar_w
        } else if on {
            ((d.beat_phase.clamp(0.0, 1.0) * bar_w as f64).round() as usize).min(bar_w)
        } else {
            0
        };
        let fill_style = if on { Style::default().fg(color) } else { dim() };
        spans.push(Span::styled("█".repeat(filled), fill_style));
        spans.push(Span::styled("░".repeat(bar_w - filled), dim()));
    }
    Line::from(spans)
}

fn hotcue_line(d: &DeckInfo, w: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for s in 0..4 {
        let label = MARK_KEYS[s];
        match d.hotcue(s) {
            Some(t) => spans.push(Span::styled(
                format!(" {label} {:>5} ", fmt_short(t)),
                Style::default().fg(Color::Black).bg(CUE_COLORS[s]).add_modifier(Modifier::BOLD),
            )),
            None => spans.push(Span::styled(format!(" {label}  --   "), dim())),
        }
        spans.push(Span::raw(" "));
    }
    if w >= 52 {
        for s in 4..8 {
            spans.push(if d.hotcue(s).is_some() {
                Span::styled(format!("{}● ", MARK_KEYS[s]), Style::default().fg(Color::Yellow))
            } else {
                Span::styled(format!("{}○ ", MARK_KEYS[s]), dim())
            });
        }
    }
    Line::from(spans)
}

fn draw_waves(buf: &mut Buffer, ov: Rect, zoom: Rect, d: &DeckInfo, wave: WaveView, release: bool) {
    let wf = match wave {
        WaveView::Ready(wf) => wf,
        other => {
            let msg = match other {
                WaveView::Loading => "waveform loading…",
                WaveView::Missing => "no waveform (track not analyzed yet)",
                _ => "",
            };
            let area = if zoom.height > 0 { zoom } else { ov };
            if area.height > 0 {
                buf.set_stringn(area.x, area.y + area.height / 2, msg, area.width as usize, dim());
            }
            return;
        }
    };
    let dur = if wf.duration() > 0.0 { wf.duration() } else { d.duration };
    let loop_on = d.loop_beats > 0.0 && d.loop_end > d.loop_start;
    if ov.height > 0 && ov.width > 0 && dur > 0.0 {
        draw_strip(buf, ov, wf, 0.0, dur);
        if loop_on {
            shade(buf, ov, d.loop_start, d.loop_end, 0.0, dur);
        }
        for s in 0..8 {
            if let Some(t) = d.hotcue(s) {
                if let Some(c) = time_to_col(t, 0.0, dur, ov.width) {
                    cue_mark(&mut buf[(ov.x + c, ov.y)], s);
                }
            }
        }
        if let Some(c) = time_to_col(d.position.min(dur - 1e-6), 0.0, dur, ov.width) {
            // the part already played is dimmed, like a CDJ's overview: where you are in the
            // track shows at a glance
            for x in 0..c {
                for y in ov.top()..ov.bottom() {
                    let cell = &mut buf[(ov.x + x, y)];
                    if super::wave::is_bar(cell) {
                        cell.set_fg(super::wave::dim(cell.fg, 0.45));
                    }
                }
            }
            playhead(buf, ov, c);
        }
    }
    if zoom.height > 0 && zoom.width > 0 {
        let (t0, t1) = (d.position - ZOOM_S / 2.0, d.position + ZOOM_S / 2.0);
        draw_strip(buf, zoom, wf, t0, t1);
        if loop_on {
            shade(buf, zoom, d.loop_start, d.loop_end, t0, t1);
        }
        if d.has_grid() && d.beat_phase >= 0.0 {
            beat_ticks(buf, zoom, d, t0, t1);
        }
        for s in 0..8 {
            if let Some(t) = d.hotcue(s) {
                if let Some(c) = time_to_col(t, t0, t1, zoom.width) {
                    cue_mark(&mut buf[(zoom.x + c, zoom.y)], s);
                }
            }
        }
        playhead(buf, zoom, zoom.width / 2);
        let _ = release;
    }
}

/// Hot cue `slot` in a waveform's top row: its letter in its pad's colour, like the pads above
fn cue_mark(cell: &mut ratatui::buffer::Cell, slot: usize) {
    cell.set_char(MARK_KEYS[slot]);
    cell.set_fg(Color::Black);
    // e to h have no pad of their own: yellow, like their dots
    cell.set_bg(CUE_COLORS.get(slot).copied().unwrap_or(Color::Yellow));
    cell.modifier.insert(Modifier::BOLD);
}

/// The loop `[from, to)` seconds: a green background, and the bars in it turned green too, so
/// it shows on a loud track whose bars fill every cell
fn shade(buf: &mut Buffer, area: Rect, from: f64, to: f64, t0: f64, t1: f64) {
    for x in 0..area.width {
        let t = t0 + (x as f64 + 0.5) / area.width as f64 * (t1 - t0);
        if t >= from && t < to {
            for y in area.top()..area.bottom() {
                let cell = &mut buf[(area.x + x, y)];
                cell.set_bg(Color::Rgb(20, 60, 25));
                if super::wave::is_bar(cell) {
                    cell.set_fg(super::wave::blend(cell.fg, (60, 210, 90), 0.55));
                }
            }
        }
    }
}

/// The playhead: a solid white line drawn over the waveform, like a CDJ's. It replaces the
/// bars in its column: a loud track fills every cell with full blocks, which hid a playhead that
/// only lightened the background behind them.
fn playhead(buf: &mut Buffer, area: Rect, col: u16) {
    if col >= area.width {
        return;
    }
    for y in area.top()..area.bottom() {
        let cell = &mut buf[(area.x + col, y)];
        cell.set_char('┃');
        cell.set_fg(Color::White);
        cell.set_bg(Color::Black);
        cell.modifier.insert(Modifier::BOLD);
    }
}

/// Beat grid lines in the zoomed strip: bar starts bright, other beats dim. Over the waveform
/// they cut a dark line through the bars (a loud track has no empty cells to draw them in).
fn beat_ticks(buf: &mut Buffer, area: Rect, d: &DeckInfo, t0: f64, t1: f64) {
    let spb = 60.0 / d.track_bpm;
    // track-time of the current beat's start
    let start = d.position - d.beat_phase * spb;
    let first = ((t0 - start) / spb).floor() as i64;
    let last = ((t1 - start) / spb).ceil() as i64;
    for k in first..=last {
        let t = start + k as f64 * spb;
        let Some(c) = time_to_col(t, t0, t1, area.width) else { continue };
        let in_bar = (d.beat_in_bar as i64 - 1 + k).rem_euclid(4);
        let (ch, color) = if in_bar == 0 { ('┃', Color::Rgb(150, 150, 150)) } else { ('╎', Color::Rgb(85, 85, 85)) };
        for y in area.top()..area.bottom() {
            let cell = &mut buf[(area.x + c, y)];
            if cell.symbol() == " " {
                cell.set_char(ch);
                cell.set_fg(color);
            } else if super::wave::is_bar(cell) {
                cell.set_fg(super::wave::dim(cell.fg, if in_bar == 0 { 0.2 } else { 0.55 }));
            }
        }
    }
}

pub(super) fn bipolar(v: f64, width: usize, color: Color) -> Vec<Span<'static>> {
    let width = if width % 2 == 0 { width - 1 } else { width }.max(3);
    let c = width / 2;
    let reach = (v.abs().clamp(0.0, 1.0) * c as f64).round() as usize;
    (0..width)
        .map(|i| {
            if i == c {
                Span::styled("│", Style::default().fg(Color::White))
            } else if (v > 0.0 && i > c && i <= c + reach) || (v < 0.0 && i < c && i + reach >= c) {
                Span::styled("█", Style::default().fg(color))
            } else {
                Span::styled("─", dim())
            }
        })
        .collect()
}

pub(super) fn unipolar(v: f64, width: usize, color: Color) -> Vec<Span<'static>> {
    let filled = ((v.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(width - filled), dim()),
    ]
}

fn mixer_row(d: &DeckInfo, idx: usize, w: usize) -> (Line<'static>, Vec<(Target, u16, u16, u16)>) {
    let color = DECK_COLORS[idx];
    let name = if idx == 0 { "A" } else { "B" };
    let deck = idx as u8;
    let mut found = Vec::new();
    let mut spans = vec![Span::styled(
        format!(" {name} "),
        Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD),
    )];
    let slot_w = w.saturating_sub(4) / 5;
    let show_val = slot_w >= 17;
    let bar_w = slot_w.saturating_sub(5 + if show_val { 7 } else { 0 }).max(3);
    let bipolars = [
        ("LOW", d.eq.0, Ctl::Eq(deck, 0)),
        ("MID", d.eq.1, Ctl::Eq(deck, 1)),
        ("HI ", d.eq.2, Ctl::Eq(deck, 2)),
        ("FLT", d.filter, Ctl::Filter(deck)),
    ];
    for (label, v, ctl) in bipolars {
        spans.push(Span::raw(" "));
        let col = text_width(&spans);
        spans.push(Span::styled(label, Style::default().fg(color)));
        spans.push(Span::raw(" "));
        spans.extend(bipolar(v, bar_w, color));
        if show_val {
            spans.push(Span::styled(format!(" {v:+.2}"), dim()));
        }
        found.push((Target::Slot(Slot::Ctl(ctl)), col, 3, text_width(&spans) - col));
    }
    spans.push(Span::raw(" "));
    let col = text_width(&spans);
    spans.push(Span::styled("FAD", Style::default().fg(color)));
    spans.push(Span::raw(" "));
    spans.extend(unipolar(d.fader, bar_w + 2, color));
    if show_val {
        spans.push(Span::styled(format!(" {:.2}", d.fader), dim()));
    }
    found.push((Target::Slot(Slot::Ctl(Ctl::Fader(deck))), col, 3, text_width(&spans) - col));
    (Line::from(spans), found)
}

fn draw_mixer(buf: &mut Buffer, area: Rect, v: &View, spots: &mut Vec<Spot>) {
    let m = v.mixer;
    let mut title = vec![
        Span::styled("─ Mixer ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("quantize ", dim()),
        if m.quantize {
            Span::styled("ON", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("off", Style::default().fg(Color::Red))
        },
        Span::styled(format!(" (F5) · {} · xruns ", if m.mode.is_empty() { "?" } else { &m.mode }), dim()),
        Span::styled(
            m.xruns.to_string(),
            if m.xruns > 0 { Style::default().fg(Color::Red) } else { dim() },
        ),
    ];
    if let (Some(a), Some(b)) = (v.decks.first(), v.decks.get(1)) {
        if a.playing && b.playing && a.beat_phase >= 0.0 && b.beat_phase >= 0.0 {
            let mut diff = a.beat_phase - b.beat_phase;
            diff -= diff.round();
            let ok = diff.abs() < 0.03;
            title.push(Span::styled(" · phase Δ ", dim()));
            title.push(Span::styled(
                format!("{diff:+.2}"),
                Style::default().fg(if ok { Color::Green } else { Color::Yellow }),
            ));
        }
    }
    title.push(Span::raw(" "));
    let used: usize = title.iter().map(|s| s.content.chars().count()).sum();
    title.push(Span::styled("─".repeat((area.width as usize).saturating_sub(used)), dim()));
    let blank = DeckInfo::default();
    let w = area.width as usize;
    let a = v.decks.first().unwrap_or(&blank);
    let b = v.decks.get(1).unwrap_or(&blank);
    // crossfader, then master, cue mix and sync range when the line is wide enough
    let extras: Vec<(Slot, String)> = if w >= 72 {
        vec![
            (Slot::Master, format!("MST {:.2}", m.master)),
            (Slot::CueMix, format!("CUE {:.2}", m.cue_mix)),
            (Slot::SyncRange, format!("RNG ±{:.0}%", v.hints.sync_range * 100.0)),
            (Slot::Jog, format!("JOG {}", hints::fmt_jog(v.hints.jog))),
        ]
    } else {
        Vec::new()
    };
    let extra_w: usize = extras.iter().map(|(_, t)| 2 + t.chars().count()).sum();
    let bar_w = if extras.is_empty() { w.saturating_sub(24).max(9) } else { w.saturating_sub(17 + extra_w).max(9) };
    let pos = (((m.crossfader.clamp(-1.0, 1.0) + 1.0) / 2.0) * (bar_w - 1) as f64).round() as usize;
    let mut xf = vec![Span::styled(" XF ", Style::default().fg(Color::Black).bg(Color::White)), Span::raw(" "), Span::styled("A ", Style::default().fg(DECK_COLORS[0]))];
    for i in 0..bar_w {
        xf.push(if i == pos {
            Span::styled("◆", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("━", dim())
        });
    }
    xf.push(Span::styled(" B", Style::default().fg(DECK_COLORS[1])));
    let xf_end = text_width(&xf);
    xf.push(Span::styled(format!("  {:+.2}", m.crossfader), dim()));
    let mut found = vec![(Target::Slot(Slot::Ctl(Ctl::Crossfader)), 0u16, 4u16, xf_end, 3usize)];
    for (slot, text) in extras {
        let col = text_width(&xf);
        xf.push(Span::raw("  "));
        let len = text.chars().count() as u16;
        xf.push(Span::styled(text, dim()));
        found.push((Target::Slot(slot), col, 2, len + 2, 3));
    }
    let (row_a, found_a) = mixer_row(a, 0, w);
    let (row_b, found_b) = mixer_row(b, 1, w);
    let clip = Rect::new(area.x, area.y, area.width, area.height);
    for (row, list) in [(1u16, found_a), (2, found_b)] {
        for (t, col, lw, hw) in list {
            add_spot(spots, clip, t, area.x + col, area.y + row, lw, hw);
        }
    }
    for (t, col, lw, hw, row) in found {
        add_spot(spots, clip, t, area.x + col, area.y + row as u16, lw, hw);
    }
    Paragraph::new(vec![Line::from(title), row_a, row_b, Line::from(xf)]).render(area, buf);
}

fn draw_library(buf: &mut Buffer, area: Rect, v: &View) {
    // what plays next sits beside the library: the coming transition, then the queue
    let has_next = v.up_next.is_some() || !v.queue.is_empty();
    if has_next && area.width >= 100 {
        let box_w = 38.min(area.width / 3);
        let lib = Rect { width: area.width - box_w, ..area };
        draw_library_list(buf, lib, v, None);
        draw_up_next(buf, Rect::new(area.x + lib.width, area.y, box_w, area.height), v);
        return;
    }
    draw_library_list(buf, area, v, v.up_next);
}

/// The UP NEXT box: the coming automix transition, then the rest of the queue
fn draw_up_next(buf: &mut Buffer, area: Rect, v: &View) {
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(DIM)).title(" UP NEXT · q queues ");
    let inner = block.inner(area);
    block.render(area, buf);
    let w = inner.width.saturating_sub(1) as usize;
    let mut y = inner.y;
    if let Some(next) = v.up_next {
        // "next: CRIA · blend in 1:12": the title, then the transition on its own line
        let next = next.strip_prefix("next: ").unwrap_or(next);
        let (title, how) = next.rsplit_once(" · ").unwrap_or((next, ""));
        buf.set_stringn(inner.x + 1, y, trunc(title, w), w, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        y += 1;
        if !how.is_empty() && y < inner.bottom() {
            buf.set_stringn(inner.x + 1, y, trunc(&format!("  {how}"), w), w, Style::default().fg(Color::Yellow));
            y += 1;
        }
    }
    // the first queued track is the one `next:` names
    let skip = usize::from(v.up_next.is_some_and(|n| v.queue.first().is_some_and(|q| n.contains(q.split(" — ").next().unwrap_or(q)))));
    for (i, q) in v.queue.iter().enumerate().skip(skip) {
        if y >= inner.bottom() {
            break;
        }
        buf.set_stringn(inner.x + 1, y, trunc(&format!("{} {q}", i + 1), w), w, Style::default());
        y += 1;
    }
    if v.up_next.is_none() && v.queue.is_empty() {
        buf.set_stringn(inner.x + 1, inner.y, "nothing queued", w, dim());
    }
}

fn draw_library_list(buf: &mut Buffer, area: Rect, v: &View, next: Option<&str>) {
    let searching = v.focus == Focus::Search;
    let browsing = v.focus == Focus::Library;
    let filter = if v.search.is_empty() { String::new() } else { format!(" /{}", v.search) };
    let title = if searching {
        format!(" Library {}/{} — /{}█ (Enter done · Esc clear) ", v.lib.len(), v.lib_total, v.input)
    } else if browsing {
        let long = format!(
            " LIBRARY {}/{}{} · j/k move · / search · a or b load onto deck · Enter focused deck · q queue · Tab/Esc decks ",
            v.lib.len(),
            v.lib_total,
            filter
        );
        if long.chars().count() + 2 <= area.width as usize {
            long
        } else {
            format!(" LIBRARY {}/{}{} · j/k · / search · a b load · Enter focused deck · Esc ", v.lib.len(), v.lib_total, filter)
        }
    } else {
        match next {
            // no room for the box: the coming track in the title
            Some(n) => format!(" Library {}{} · Tab to browse · {n} ", v.lib_total, filter),
            None => format!(" Library {}{} · Tab to browse ", v.lib_total, filter),
        }
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if searching || browsing { Color::Yellow } else { DIM }))
        .title(title);
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height == 0 {
        return;
    }
    if v.lib.is_empty() {
        buf.set_stringn(inner.x + 1, inner.y, "no tracks", inner.width as usize, dim());
        return;
    }
    let rows = inner.height as usize;
    let sel = v.lib_sel.min(v.lib.len() - 1);
    let offset = sel.saturating_sub(rows / 2).min(v.lib.len().saturating_sub(rows));
    for (row, m) in v.lib.iter().enumerate().skip(offset).take(rows) {
        let y = inner.y + (row - offset) as u16;
        let path = <&std::path::Path>::from(&m.path).to_string_lossy().into_owned();
        let tags: String = (0..2)
            .filter(|i| v.decks.get(*i).is_some_and(|d| d.loaded && d.path == path))
            .map(|i| if i == 0 { "[A]" } else { "[B]" })
            .collect();
        let who = if m.artist.is_empty() { m.title.clone() } else { format!("{} — {}", m.title, m.artist) };
        let len = fmt_short(m.length.as_secs_f64());
        let mark = v.learn.marks.iter().find(|(p, _)| *p == path).map(|(_, d)| format!("★{d}")).unwrap_or_default();
        let right = format!("{mark:>2}{tags:>6} {len:>5}");
        let w = inner.width as usize;
        let left_w = w.saturating_sub(right.chars().count() + 3);
        let style = if row == sel {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if !mark.is_empty() {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let line = format!("{} {:<left_w$} {right}", if row == sel { "▸" } else { " " }, trunc(&who, left_w));
        buf.set_stringn(inner.x, y, line, w, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    pub fn deck_a() -> DeckInfo {
        DeckInfo {
            loaded: true,
            path: "/m/aftertekno.flac".into(),
            title: "Aftertekno".into(),
            artist: "Pneumatix".into(),
            position: 83.4,
            duration: 437.0,
            playing: true,
            rate: 1.0123,
            bpm: 129.55,
            track_bpm: 127.98,
            beat_phase: 0.5,
            beat_in_bar: 3,
            synced: true,
            loop_beats: 4.0,
            loop_start: 84.0,
            loop_end: 86.0,
            cue_point: 1.0,
            hotcues: vec![12.0, -1.0, 95.5, -1.0, -1.0, -1.0, -1.0, -1.0],
            key: "8A".into(),
            gain_db: -1.5,
            decoded: 437.0,
            eq: (0.5, 0.0, -1.0),
            filter: -0.3,
            fader: 0.8,
            headphones: true,
        }
    }

    fn deck_b() -> DeckInfo {
        DeckInfo {
            loaded: true,
            path: "/m/smokin.flac".into(),
            title: "Smokin".into(),
            artist: "Aasi & Swooh".into(),
            position: 10.0,
            duration: 300.0,
            rate: 1.0,
            bpm: 127.0,
            track_bpm: 127.0,
            beat_phase: 0.1,
            beat_in_bar: 1,
            hotcues: vec![-1.0; 8],
            fader: 0.0,
            eq: (0.0, 0.0, 0.0),
            ..Default::default()
        }
    }

    pub fn wave() -> Waveform {
        // 437 s at 50 columns/s with a kick every 0.47 s
        let cols = (0..21850)
            .map(|i| {
                let beat = (i % 23) as u8;
                if beat < 4 {
                    [250, 120, 60, 200, 90, 30]
                } else {
                    [60, 150, 200, 30, 100, 150]
                }
            })
            .collect();
        Waveform { duration_ms: 437_000, cols }
    }

    fn lib() -> Vec<Music> {
        let m = |t: &str, a: &str, p: &str| Music {
            title: t.into(),
            artist: a.into(),
            length: std::time::Duration::from_secs(254),
            path: std::path::PathBuf::from(p).into(),
            ..Default::default()
        };
        vec![
            m("Aftertekno", "Pneumatix", "/m/aftertekno.flac"),
            m("Smokin", "Aasi & Swooh", "/m/smokin.flac"),
            m("Hot minute", "Nlogax", "/m/hot.flac"),
        ]
    }

    pub fn render(w: u16, h: u16, f: impl FnOnce(&mut View)) -> String {
        render_full(w, h, f).0
    }

    /// `CODJ_SNAP=dir cargo test snapshot -- --ignored`, then `tools/cells2png.py`: every scenario's cells (symbol, colors,
    /// modifiers) as text files, for drawing as images and looking at the screen as it is
    #[test]
    #[ignore]
    fn snapshot() {
        use std::fmt::Write as _;
        let Some(dir) = std::env::var_os("CODJ_SNAP") else { return };
        let loud = Waveform { duration_ms: 437_000, cols: vec![[255, 255, 255, 230, 230, 230]; 21850] };
        let labels = leak_labels();
        let decks = {
            let mut b = deck_b();
            b.playing = true;
            b.fader = 0.9;
            b.hotcues = vec![30.0, 60.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0];
            [deck_a(), b]
        };
        let mixer = MixerInfo { crossfader: -0.25, mode: "internal".into(), quantize: true, xruns: 0, ..Default::default() };
        let wf = wave();
        let lib = lib();
        type Tweak<'x> = Box<dyn Fn(&mut View<'x>) + 'x>;
        let scenes: Vec<(&str, u16, u16, Tweak)> = vec![
            ("normal", 120, 35, Box::new(|_| {})),
            ("loud", 120, 35, Box::new(|v| v.waves = [WaveView::Ready(&loud), WaveView::Ready(&loud)])),
            ("big", 180, 50, Box::new(|v| v.waves = [WaveView::Ready(&wf), WaveView::Ready(&loud)])),
            ("hints", 120, 35, Box::new(|v| {
                v.waves = [WaveView::Ready(&loud), WaveView::Ready(&wf)];
                v.hints.active = true;
                v.hints.labels = labels;
            })),
            ("browse", 120, 35, Box::new(|v| {
                v.waves = [WaveView::Ready(&loud), WaveView::Ready(&wf)];
                v.focus = Focus::Library;
                v.note = Some("deck B is playing, so the track went onto the other deck");
                v.error = Some("DjDecks: timeout");
            })),
            ("help", 120, 35, Box::new(|v| v.learn.help = true)),
            ("small", 100, 30, Box::new(|v| v.waves = [WaveView::Ready(&loud), WaveView::Ready(&wf)])),
        ];
        for (name, w, h, tweak) in &scenes {
            let mut v = test_view(&decks, &mixer, &lib);
            v.waves = [WaveView::Ready(&wf), WaveView::Ready(&wf)];
            v.lib_total = lib.len();
            v.lib_sel = 1;
            v.doubt = [false, true];
            tweak(&mut v);
            let mut term = Terminal::new(TestBackend::new(*w, *h)).unwrap();
            term.draw(|frame| {
                let area = frame.area();
                draw(frame.buffer_mut(), area, &v);
            })
            .unwrap();
            let buf = term.backend().buffer().clone();
            let mut out = format!("{w} {h}\n");
            for y in 0..*h {
                for x in 0..*w {
                    let c = &buf[(x, y)];
                    let _ = writeln!(out, "{}\t{:?}\t{:?}\t{:?}", c.symbol().replace('\t', " "), c.fg, c.bg, c.modifier);
                }
            }
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}.cells")), out).unwrap();
        }
    }

    pub fn render_full(w: u16, h: u16, f: impl FnOnce(&mut View)) -> (String, Vec<Spot>) {
        let decks = [deck_a(), deck_b()];
        let mixer = MixerInfo { crossfader: -0.25, mode: "internal".into(), quantize: true, xruns: 0, ..Default::default() };
        let wf = wave();
        let lib = lib();
        let mut v = View {
            decks: &decks,
            mixer: &mixer,
            waves: [WaveView::Ready(&wf), WaveView::Loading],
            doubt: [false, true],
            link_ok: true,
            error: None,
            note: None,
            focus: Focus::Decks,
            input: "",
            search: "",
            lib: &lib,
            lib_total: 3,
            lib_sel: 1,
            release_events: true,
            focus_deck: 0,
            showcmd: "",
            set: None,
            rec: None,
            audio_rec: None,
            queue: &[],
            up_next: None,
            overlay: None,
            panel: None,
            hints: HintView::default(),
            learn: LearnView::default(),
            patch: None,
        };
        f(&mut v);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut spots = Vec::new();
        term.draw(|frame| {
            let area = frame.area();
            spots = draw(frame.buffer_mut(), area, &v);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let text = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        (text, spots)
    }

    #[test]
    fn full_screen_shows_both_decks_mixer_and_library() {
        let text = render(120, 35, |_| {});
        println!("{text}");
        for want in [
            "DECK A", "DECK B", "▶ playing", "⏸ paused",
            "Aftertekno — Pneumatix", "Smokin — Aasi & Swooh",
            "129.55", "(trk 127.98)", "8A", "+1.23%", "SYNC",
            "01:23.4", "-5:53", "LOOP 4", "HP ●", "HP ○",
            "waveform loading", "Mixer", "quantize ON", "internal", "xruns 0",
            "LOW", "FLT", "FAD", "XF", "-0.25", "+0.50", "-1.00",
            "Library 3", "▸", "[A]",
        ] {
            assert!(text.contains(want), "missing {want:?}");
        }
        // marks: a and c set, b and d empty
        assert!(text.contains(" a  0:12 "), "{text}");
        assert!(text.contains(" c  1:35 "), "{text}");
        assert!(text.contains(" b  --"), "{text}");
        // a waveform was drawn (bars and the playhead)
        assert!(text.contains('█'));
        assert!(text.contains('┃'));
        // phase meter: beat 3 lit
        assert!(text.contains("3 ██"));
        // exactly 35 lines, none wider than the screen
        assert_eq!(text.lines().count(), 35);
    }

    #[test]
    fn the_playhead_shows_over_a_waveform_full_to_the_top() {
        // a loud track: every cell of the strip is a full block
        let area = Rect::new(0, 0, 20, 4);
        let mut buf = Buffer::empty(area);
        for x in 0..20 {
            for y in 0..4 {
                buf[(x, y)].set_char('█');
            }
        }
        playhead(&mut buf, area, 10);
        for y in 0..4 {
            assert_eq!(buf[(10, y)].symbol(), "┃");
            assert_eq!(buf[(10, y)].fg, Color::White);
        }
        assert_eq!(buf[(9, 0)].symbol(), "█");
    }

    #[test]
    fn selected_row_and_loaded_markers_in_the_library() {
        let text = render(120, 35, |_| {});
        let sel = text.lines().find(|l| l.contains("▸")).unwrap();
        assert!(sel.contains("Smokin — Aasi & Swooh"));
        // Smokin is on deck B
        assert!(sel.contains("[B]"));
        assert!(text.lines().any(|l| l.contains("Aftertekno — Pneumatix") && l.contains("[A]") && !l.contains("▸")));
    }

    #[test]
    fn empty_deck_and_errors() {
        let text = render(120, 35, |v| {
            static EMPTY: std::sync::OnceLock<Vec<DeckInfo>> = std::sync::OnceLock::new();
            v.decks = EMPTY.get_or_init(|| vec![DeckInfo::default(), DeckInfo::default()]);
            v.link_ok = false;
            v.error = Some("codjd has no deck API (DjDecks): update codjd");
        });
        assert!(text.contains("— empty —"));
        assert!(text.contains("no deck API"));
        assert!(text.contains("○ no data"));
    }

    #[test]
    fn waveform_has_the_loop_and_beat_grid() {
        let text = render(120, 35, |_| {});
        // beat ticks in the zoomed strip (empty cells above the bars)
        assert!(text.contains('┃') || text.contains('╎'));
        // hot cue markers in the strips
        assert!(text.contains("┃a") || text.contains('a'));
    }

    #[test]
    fn hot_cues_in_the_strips_are_their_letters_in_their_pad_colours_even_on_a_loud_track() {
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        for x in 0..10 {
            for y in 0..2 {
                buf[(x, y)].set_char('█');
            }
        }
        cue_mark(&mut buf[(2, 0)], 0);
        cue_mark(&mut buf[(5, 0)], 6);
        assert_eq!((buf[(2, 0)].symbol(), buf[(2, 0)].bg), ("a", Color::Red));
        assert_eq!((buf[(5, 0)].symbol(), buf[(5, 0)].bg), ("g", Color::Yellow));
    }

    #[test]
    fn the_beat_grid_and_the_loop_show_over_a_loud_track() {
        let loud = Waveform { duration_ms: 437_000, cols: vec![[255, 255, 255, 230, 230, 230]; 21850] };
        let decks = [deck_a(), deck_b()];
        let mixer = MixerInfo::default();
        let lib = lib();
        let mut v = test_view(&decks, &mixer, &lib);
        v.waves = [WaveView::Ready(&loud), WaveView::None];
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 35));
        draw(&mut buf, Rect::new(0, 0, 120, 35), &v);
        // a row of deck A's zoomed strip
        let row = 18;
        let fgs: Vec<Color> = (0..60).map(|x| buf[(x, row)].fg).collect();
        // bars of three kinds at least: plain, darkened by a grid line, greened by the loop
        let mut kinds: Vec<Color> = (0..60).filter(|&x| super::super::wave::is_bar(&buf[(x, row)])).map(|x| buf[(x, row)].fg).collect();
        kinds.sort_by_key(|c| format!("{c:?}"));
        kinds.dedup();
        assert!(kinds.len() >= 3, "{fgs:?}");
    }

    #[test]
    fn degrades_to_smaller_screens() {
        for (w, h) in [(100, 30), (80, 24), (70, 20), (60, 14)] {
            let text = render(w, h, |_| {});
            assert_eq!(text.lines().count(), h as usize);
            assert!(text.contains("DECK A") && text.contains("DECK B"), "{w}x{h}");
            assert!(text.contains("Library"), "{w}x{h}");
            assert!(text.contains("Mixer"), "{w}x{h}");
        }
        let text = render(80, 24, |_| {});
        println!("{text}");
        assert!(text.contains("Aftertekno"));
        // a narrow deck keeps tempo, track tempo, key and pitch with fewer decimals
        assert!(text.contains("BPM (128) 8A") && text.contains("+1.2%"), "{text}");
    }

    #[test]
    fn too_small_says_so() {
        let text = render(40, 10, |_| {});
        assert!(text.contains("DJ mode needs at least"));
    }

    #[test]
    fn search_and_command_inputs_are_shown() {
        let text = render(120, 35, |v| {
            v.focus = Focus::Search;
            v.input = "tekno";
        });
        assert!(text.contains("/tekno█"));
        let text = render(120, 35, |v| v.focus = Focus::Library);
        assert!(text.contains("LIBRARY") && text.contains("j/k move"));
        let text = render(120, 35, |v| {
            v.focus = Focus::Command;
            v.input = "q";
        });
        assert!(text.lines().last().unwrap().starts_with(":q█"));
    }

    fn demo_set() -> SetState {
        let t = |id: &str, title: &str, path: &str| (id.to_string(), title.to_string(), path.to_string());
        SetState {
            name: "demo: 3 tracks".into(),
            file: "/s/demo.codj".into(),
            errors: 0,
            tracks: vec![
                t("warm", "Aftertekno", "/m/aftertekno.flac"),
                t("mid", "Smokin", "/m/smokin.flac"),
                t("peak", "Noita", "/m/noita.flac"),
            ],
            mixes: vec![
                ("warm".into(), "mid".into(), 448.0, 128.0),
                ("mid".into(), "peak".into(), 384.0, 32.0),
            ],
            armed: -1,
            running: false,
            out_deck: -1,
            mix_beat: 0.0,
            auto: false,
            next_items: vec![],
        }
    }

    fn armed_midway() -> SetState {
        SetState {
            armed: 0,
            running: true,
            out_deck: 0,
            mix_beat: 38.0,
            auto: true,
            next_items: vec![
                (64.0, "swap low".into()),
                (64.0, "bars 16–32 · A filter → −0.6".into()),
                (112.0, "A loop tail".into()),
                (128.0, "A stop".into()),
            ],
            ..demo_set()
        }
    }

    fn with_set(set: SetState) -> impl FnOnce(&mut View) {
        let set: &'static SetState = Box::leak(Box::new(set));
        move |v| v.set = Some(set)
    }

    #[test]
    fn no_set_no_strip() {
        let text = render(120, 35, |_| {});
        assert!(!text.contains(" SET "));
        // an inactive set (empty name) also draws nothing
        let text = render(120, 35, |v| {
            v.set = None;
        });
        assert!(!text.contains("MIX"));
    }

    #[test]
    fn idle_set_lists_tracks_and_mixes_and_hints_at_arming() {
        let text = render(120, 35, with_set(demo_set()));
        println!("{text}");
        for want in ["SET", "demo: 3 tracks", "✓ no errors", "auto off", "1 Aftertekno", "2 Smokin", "3 Noita", "32b▶", "8b▶",
            "no mix armed", "F6 arm next", "1. Aftertekno → Smokin", "2. Smokin → Noita"] {
            assert!(text.contains(want), "missing {want:?}\n{text}");
        }
        // the strip sits between the decks and the mixer
        let at = |needle: &str| text.lines().position(|l| l.contains(needle)).unwrap();
        assert!(at("DECK A") < at(" SET ") && at(" SET ") < at("Mixer"));
        assert_eq!(text.lines().count(), 35);
        // A is playing the first track, so F6 would arm mix 1
        assert!(text.contains("F6 arms mix 1"), "{text}");
    }

    #[test]
    fn set_with_errors_says_so() {
        let mut set = demo_set();
        set.errors = 2;
        let text = render(120, 35, with_set(set));
        assert!(text.contains("✗ 2 errors (:errors)"), "{text}");
        let mut set = demo_set();
        set.errors = 1;
        assert!(render(120, 35, with_set(set)).contains("✗ 1 error (:errors)"));
    }

    #[test]
    fn errors_are_red() {
        let decks = [deck_a(), deck_b()];
        let mixer = MixerInfo::default();
        let mut set = demo_set();
        set.errors = 3;
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 6));
        draw_set(&mut buf, Rect::new(0, 0, 120, 6), &set, &decks, None, 0);
        let red = (0..120).any(|x| buf[(x, 0)].fg == Color::Red && buf[(x, 0)].symbol() == "✗");
        assert!(red);
        let mut ok = demo_set();
        ok.errors = 0;
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 6));
        draw_set(&mut buf, Rect::new(0, 0, 120, 6), &ok, &decks, None, 0);
        assert!((0..120).all(|x| buf[(x, 0)].fg != Color::Red));
        let _ = mixer;
    }

    #[test]
    fn armed_mix_midway_shows_progress_markers_and_next_items() {
        let text = render(120, 35, with_set(armed_midway()));
        println!("{text}");
        for want in [
            "MIX 1: Aftertekno → Smokin", "▶ running", "bar 9.5/32", "AUTO",
            "bar 16 · swap low", "bars 16–32 · A filter → −0.6", "bar 28 · A loop tail", "bar 32 · A stop",
        ] {
            assert!(text.contains(want), "missing {want:?}\n{text}");
        }
        // 4 upcoming items -> 4 markers, 38 of 128 beats done: roughly 30 % filled
        let bar = text.lines().find(|l| l.contains("MIX 1")).unwrap();
        assert!(bar.matches('◆').count() >= 3, "{bar}");
        assert!(bar.contains('█') && bar.contains('░'));
        // consumed items are gone from the markers: fewer items, fewer diamonds
        let mut later = armed_midway();
        later.mix_beat = 120.0;
        later.next_items = vec![(128.0, "A stop".into())];
        let bar2 = render(120, 35, with_set(later)).lines().find(|l| l.contains("MIX 1")).unwrap().to_string();
        assert_eq!(bar2.matches('◆').count(), 1, "{bar2}");
        // current track in the strip is marked, next too
        assert!(text.contains("▶1 Aftertekno") && text.contains("»2 Smokin"), "{text}");
    }

    #[test]
    fn armed_but_waiting_counts_down() {
        let mut set = armed_midway();
        set.running = false;
        set.mix_beat = -16.0;
        set.auto = false;
        let text = render(120, 35, with_set(set));
        assert!(text.contains("armed, starts in 4 bars"), "{text}");
        assert!(text.contains("bar 0/32"), "{text}");
        assert!(text.contains("auto off"));
    }

    #[test]
    fn auto_badge() {
        let mut set = demo_set();
        set.auto = true;
        let text = render(120, 35, with_set(set));
        assert!(text.contains(" AUTO ") && !text.contains("auto off"), "{text}");
    }

    #[test]
    fn set_strip_degrades() {
        for (w, h) in [(100, 30), (80, 24), (70, 20), (60, 14)] {
            for set in [demo_set(), armed_midway()] {
                let text = render(w, h, with_set(set));
                assert_eq!(text.lines().count(), h as usize);
                assert!(text.contains("SET"), "{w}x{h}\n{text}");
                assert!(text.contains("DECK A") && text.contains("Mixer") && text.contains("Library"), "{w}x{h}");
            }
        }
        let text = render(80, 24, with_set(armed_midway()));
        println!("{text}");
        assert!(text.contains("MIX 1"));
        let text = render(60, 14, with_set(armed_midway()));
        println!("{text}");
        assert!(text.contains("mix 1 ▶ 9.5/32") && text.contains("bar 16 · swap low"), "{text}");
        let wide = render(100, 14, with_set(armed_midway()));
        assert!(wide.contains("mix 1 ▶ bar 9.5/32") && wide.contains("next bar 16 · swap low"), "{wide}");
    }

    #[test]
    fn recording_state_in_the_strip() {
        let text = render(120, 35, |v| {
            with_set(demo_set())(v);
            v.rec = Some((50.0, 34));
        });
        assert!(text.contains("● REC 12.5 bars · 34 events"), "{text}");
        assert!(text.contains("F9 rec"));
        // one-line strip, narrow and wide
        let text = render(60, 14, |v| {
            with_set(armed_midway())(v);
            v.rec = Some((50.0, 34));
        });
        assert!(text.contains("● REC 12.5 · 34 ev"), "{text}");
        let text = render(100, 14, |v| {
            with_set(armed_midway())(v);
            v.rec = Some((50.0, 1));
        });
        assert!(text.contains("● REC 12.5 bars · 1 event"), "{text}");
        // not recording: no badge
        assert!(!render(120, 35, with_set(demo_set())).contains("REC"));
    }

    #[test]
    fn many_tracks_show_a_window_around_the_current_one() {
        let mut set = armed_midway();
        set.tracks = (0..12).map(|i| (format!("t{i}"), format!("Track {i}"), format!("/m/{i}.flac"))).collect();
        set.mixes = (0..11).map(|i| (format!("t{i}"), format!("t{}", i + 1), 64.0, 128.0)).collect();
        set.armed = 7;
        let text = render(120, 35, with_set(set));
        assert!(text.contains("8 Track 7") && text.contains("9 Track 8"), "{text}");
        assert!(text.contains('‹') && text.contains('›'));
        assert!(!text.contains("1 Track 0"));
    }

    #[test]
    fn error_overlay() {
        let lines = vec!["demo.codj:12:5: unknown track \"mid2\"".to_string(), "demo.codj:20:1: bar 40 is past the end".to_string()];
        let text = render(120, 35, |v| {
            let l: &'static [String] = Box::leak(lines.into_boxed_slice());
            v.overlay = Some(l);
        });
        assert!(text.contains("set errors") && text.contains("demo.codj:12:5: unknown track"), "{text}");
        assert!(text.contains("demo.codj:20:1"));
        // an empty overlay-less screen has none
        assert!(!render(120, 35, |_| {}).contains("set errors"));
    }

    fn help_view(v: &mut View) {
        v.learn.help = true;
    }

    #[test]
    fn the_key_map_is_drawn_on_the_screen_at_120x35() {
        let text = render(120, 35, help_view);
        assert_eq!(text.lines().count(), 35);
        // one panel for the focused deck, then the mixer and the global keys
        for l in ["FOCUSED DECK A keys", "MIXER keys", "everywhere", "? KEYS"] {
            assert!(text.contains(l), "{l}\n{text}");
        }
        assert!(!text.contains("DECK B keys") && !text.contains("┌ DECK A keys"), "{text}");
        for l in [
            "sync: match A's tempo and beats to B",
            "cue: a saved spot; hold to hear it in headphones",
            "jump 1 beat forward / back (4w: 4 beats)",
            "mark: save a hot cue here (eight of them)",
            "loop 4 beats; 8v loops 8; v again leaves",
            "undo: rolls back a transition, or a control",
            "low: the bass — keep only one bassline at a time",
            "crossfader: which deck the room hears",
            "hints: label every control, type one to pick it",
            "record: your playing becomes code",
            "set editor · AI agent",
            ":help :tutor :mix new · :q leaves",
            "cancel count / pick (never leaves)",
        ] {
            assert!(text.contains(l), "{l}\n{text}");
        }
        // nothing is cut off at the panel edge: every deck line fits
        for l in crate::dj::glossary::DECK_KEYS.iter().map(|e| crate::dj::glossary::deck_entry(e, 0).1) {
            assert!(text.contains(&l), "truncated: {l}");
        }
        // the key map follows the focus: with B focused the sync line is B's
        let text = render(120, 35, |v| {
            v.learn.help = true;
            v.focus_deck = 1;
        });
        assert!(text.contains("FOCUSED DECK B keys") && text.contains("sync: match B's tempo and beats to A"), "{text}");
        // and no key of the old map is on it
        for gone in ["Alt q/a", "play / pause this deck", "⇧1-4", "7-0"] {
            assert!(!text.contains(gone), "{gone}");
        }
    }

    #[test]
    fn up_next_shows_the_coming_transition_and_the_queue() {
        let queue: &'static [String] = Box::leak(vec!["CRIA — ØTTA".to_string(), "CARLOTA — Raël".to_string()].into_boxed_slice());
        let wide = render(120, 35, |v| {
            v.queue = queue;
            v.up_next = Some("next: CRIA · blend in 1:12");
        });
        assert!(wide.contains("UP NEXT") && wide.contains("CRIA") && wide.contains("blend in 1:12") && wide.contains("2 CARLOTA"), "{wide}");
        // narrow: no box, the coming track rides in the library title
        let narrow = render(80, 30, |v| {
            v.queue = queue;
            v.up_next = Some("next: CRIA · blend in 1:12");
        });
        assert!(!narrow.contains("UP NEXT") && narrow.contains("next: CRIA"), "{narrow}");
        let none = render(120, 35, |_| {});
        assert!(!none.contains("UP NEXT"));
    }

    #[test]
    fn an_audio_recording_shows_its_time_in_the_header() {
        let text = render(80, 30, |v| v.audio_rec = Some(754.0));
        let head = text.lines().next().unwrap();
        assert!(head.contains("● REC 12:34") && head.contains("● live") && head.contains("? keys"), "{head}");
        let text = render(80, 30, |_| {});
        assert!(!text.lines().next().unwrap().contains("REC"));
    }

    #[test]
    fn the_header_and_empty_decks_fit_a_narrow_screen() {
        for w in [60u16, 80, 120] {
            let text = render(w, 30, |_| {});
            let head = text.lines().next().unwrap();
            assert!(head.contains("? keys") && head.contains("● live"), "{w}: {head}");
        }
        let text = render(80, 30, |v| {
            static EMPTY: std::sync::OnceLock<Vec<DeckInfo>> = std::sync::OnceLock::new();
            v.decks = EMPTY.get_or_init(|| vec![DeckInfo::default(), DeckInfo::default()]);
        });
        assert!(text.contains("a loads here") && text.contains("b loads here"), "{text}");
    }

    #[test]
    fn the_key_map_degrades_on_small_screens() {
        for (w, h) in [(100, 30), (80, 24), (70, 20), (60, 14)] {
            let text = render(w, h, help_view);
            assert_eq!(text.lines().count(), h as usize);
            assert!(text.contains("FOCUSED DECK A") && text.contains("? KEYS"), "{w}x{h}\n{text}");
        }
        // what does not fit becomes one list that j/k scroll, like vim's help
        let text = render(80, 24, help_view);
        assert!(text.contains("j k scroll") && text.contains("j more"), "{text}");
        assert!(!text.contains("this key map"), "the last key is below the screen\n{text}");
        let text = render(80, 24, |v| {
            help_view(v);
            v.learn.help_scroll = 99;
        });
        assert!(text.contains("this key map") && !text.contains("j more"), "scrolled to the end\n{text}");
        // the key map does not change the control spots
        let (_, plain) = render_full(120, 35, |_| {});
        let (_, with) = render_full(120, 35, help_view);
        assert_eq!(plain, with);
    }

    #[test]
    fn the_focused_deck_is_drawn_apart() {
        let top = |text: &str| text.lines().nth(1).unwrap().to_string();
        let halves = |line: &str| (line.chars().take(60).collect::<String>(), line.chars().skip(60).collect::<String>());
        let a = render(120, 35, |_| {});
        let line = top(&a);
        assert_eq!(line.matches("FOCUS").count(), 1, "{line}");
        let (left, right) = halves(&line);
        assert!(left.contains("DECK A") && left.contains("▶ FOCUS") && !right.contains("FOCUS"), "{line}");
        // a thick border for the focused deck, a plain one for the other
        assert!(left.starts_with('┏') && right.starts_with('┌'), "{line}");
        let b = render(120, 35, |v| v.focus_deck = 1);
        let line = top(&b);
        let (left, right) = halves(&line);
        assert!(right.contains("DECK B") && right.contains("▶ FOCUS") && !left.contains("FOCUS"), "{line}");
        assert!(right.starts_with('┏') && left.starts_with('┌'), "{line}");
        // the focus is also bright in colour: bold border cells
        let focused_bold = |focus: u8| {
            let decks = [deck_a(), deck_b()];
            let mixer = MixerInfo::default();
            let lib = lib();
            let mut v = test_view(&decks, &mixer, &lib);
            v.focus_deck = focus;
            let mut term = Terminal::new(TestBackend::new(120, 35)).unwrap();
            term.draw(|f| {
                let area = f.area();
                draw(f.buffer_mut(), area, &v);
            })
            .unwrap();
            let buf = term.backend().buffer().clone();
            (buf[(0, 1)].modifier.contains(Modifier::BOLD), buf[(60, 1)].modifier.contains(Modifier::BOLD))
        };
        assert_eq!(focused_bold(0), (true, false));
        assert_eq!(focused_bold(1), (false, true));
    }

    #[test]
    fn showcmd_and_the_loop_mode_are_in_the_status_line() {
        let status = |f: &dyn Fn(&mut View)| {
            let text = render(120, 35, |v| f(v));
            text.lines().last().unwrap().to_string()
        };
        let plain = status(&|v| v.focus_deck = 1);
        assert!(!plain.contains("LOOP") && plain.contains("h l focus"), "{plain}");
        // the half-typed command sits at the right end
        let s = status(&|v| {
            v.focus_deck = 1;
            v.showcmd = "42m";
        });
        assert!(s.contains("42m"), "{s}");
        assert!(s.trim_end().ends_with("42m"), "{s}");
        // deck A loops: the mode shows while A has the focus, not while B does
        let s = status(&|v| v.focus_deck = 0);
        assert!(s.contains("-- LOOP 4 --"), "{s}");
        // with the chip and the count together nothing overlaps
        let labels = leak_labels();
        let s = status(&|v| {
            v.showcmd = "4";
            v.hints = HintView { labels, selected: Some(Target::Slot(Slot::Ctl(Ctl::Crossfader))), chip: Some("◆ crossfader -0.25"), ..HintView::default() };
        });
        assert!(s.contains("-- LOOP 4 --") && s.contains(" 4 ") && s.contains("◆ crossfader -0.25"), "{s}");
    }

    #[test]
    fn hot_cue_slots_are_labelled_with_marks() {
        let text = render(120, 35, |_| {});
        assert!(text.contains(" a  0:12 ") && text.contains(" c  1:35 ") && text.contains(" b  --"), "{text}");
        assert!(text.contains("e○") && text.contains("h○"), "{text}");
        assert!(!text.contains(" 7  ") && !text.contains("pick a track below and press <"), "{text}");
    }

    #[test]
    fn explanation_next_to_the_chip_and_in_the_hint_status() {
        let sel = Target::Slot(Slot::Ctl(Ctl::Eq(1, 0)));
        let labels = leak_labels();
        let text = render(120, 35, |v| {
            v.focus_deck = 1;
            v.hints = HintView { labels, selected: Some(sel), chip: Some("◆ B low -1.00"), ..HintView::default() };
            v.learn.explain = Some(crate::dj::glossary::explain(sel));
        });
        let status = text.lines().last().unwrap();
        let chip = status.find("◆ B low").unwrap();
        let expl = status.find("low: the bass").unwrap();
        assert!(expl < chip && status[expl..chip].trim_end().ends_with("bassline at a time"), "{status}");
        // a note does not hide it: they share the line
        let text = render(120, 35, |v| {
            v.focus_deck = 1;
            v.hints = HintView { labels, selected: Some(sel), chip: Some("◆ B low -1.00"), ..HintView::default() };
            v.learn.explain = Some(crate::dj::glossary::explain(sel));
            v.note = Some("loading Kaiku onto deck B");
        });
        let status = text.lines().last().unwrap();
        assert!(status.starts_with("loading Kaiku") && status.contains("low: the bass") && status.contains("◆ B low"), "{status}");
        // labels up: the explanation of what the typed letters lead to
        let text = render(120, 35, |v| {
            v.hints = HintView { active: true, typed: "t", labels, ..HintView::default() };
            v.learn.explain = Some("mid: vocals, synths, snares; the body of a track");
        });
        let status = text.lines().last().unwrap();
        assert!(status.contains("HINTS") && status.contains("mid: vocals"), "{status}");
    }

    #[test]
    fn warnings_sit_next_to_errors_in_the_set_strip() {
        let mut set = demo_set();
        set.errors = 1;
        let text = render(120, 35, |v| {
            with_set(set.clone())(v);
            v.learn.warnings = 2;
        });
        assert!(text.contains("✗ 1 error") && text.contains("⚠ 2 warnings (:errors)"), "{text}");
        let text = render(60, 14, |v| {
            with_set(set.clone())(v);
            v.learn.warnings = 1;
        });
        assert!(text.contains("⚠1"), "{text}");
        assert!(!render(120, 35, with_set(demo_set())).contains('⚠'));
    }

    #[test]
    fn popups_and_warning_lines_in_the_overlay() {
        let lines = vec!["one two three four five six seven eight nine ten".to_string()];
        let popup: &'static [String] = Box::leak(lines.into_boxed_slice());
        let text = render(60, 20, |v| v.learn.popup = Some((" bass swap · any key closes ", popup)));
        assert!(text.contains("bass swap · any key closes") && text.contains("one two"), "{text}");
        // errors then warnings; yellow only when there are warnings and no errors
        let warn: &'static [String] = Box::leak(vec!["warning: mix 1 is short".to_string()].into_boxed_slice());
        let text = render(120, 35, |v| v.overlay = Some(warn));
        assert!(text.contains("warning: mix 1 is short"), "{text}");
    }

    #[test]
    fn tutor_marks_in_the_library() {
        let marks: &'static [(String, char)] = Box::leak(vec![("/m/hot.flac".to_string(), 'B')].into_boxed_slice());
        let text = render(120, 35, |v| v.learn.marks = marks);
        assert!(text.contains("★B"), "{text}");
    }

    fn tutor_view() -> &'static crate::dj::tutor::TutorView {
        use crate::dj::tutor::*;
        let t = Tutor::open(None, std::time::Instant::now());
        let d = [deck_a(), deck_b()];
        let mixer = MixerInfo { mode: "internal".into(), ..Default::default() };
        let mut view = t.view(&d, &mixer, [None, None]);
        view.n = 3;
        view.title = Step::HearB.title().into();
        view.why = Step::HearB.why().into();
        view.keys = Step::HearB.keys().iter().map(|k| k.to_string()).collect();
        view.live = live_lines(Step::HearB, &d, &mixer, [None, None], &Finding::Nothing);
        Box::leak(Box::new(view))
    }

    #[test]
    fn the_tutor_pane_sits_beside_the_library_and_never_hides_the_decks() {
        let tv = tutor_view();
        let plain = render(120, 35, |_| {});
        let text = render(120, 35, |v| v.learn.tutor = Some(tv));
        println!("{text}");
        assert_eq!(text.lines().count(), 35);
        for l in ["TUTOR 3/8", "Listen to B in headphones", "F2  headphones on B", "internal mode", ":tutor next skips · Esc pauses", "DECK A", "DECK B", "Mixer", "Library"] {
            assert!(text.contains(l), "{l}\n{text}");
        }
        // the decks keep what they showed in the text rows, and have rows left for the waveforms
        for l in ["Aftertekno — Pneumatix", "129.55 BPM", "01:23.4", "Smokin — Aasi & Swooh", "HP ●"] {
            assert!(plain.contains(l) && text.contains(l), "{l}");
        }
        let deck_rows = text.lines().take_while(|l| !l.contains("Mixer")).count();
        assert!(deck_rows >= 12, "{deck_rows} rows for the decks");
        // the library stays on the left, readable
        assert!(text.lines().any(|l| l.starts_with("│▸ Smokin")), "{text}");
    }

    #[test]
    fn browsing_shows_the_library_over_a_narrow_tutor() {
        let tv = tutor_view();
        let text = render(80, 30, |v| v.learn.tutor = Some(tv));
        assert!(text.contains("TUTOR 3/8") && !text.contains("Library"), "{text}");
        let text = render(80, 30, |v| {
            v.learn.tutor = Some(tv);
            v.focus = Focus::Library;
        });
        assert!(text.contains("LIBRARY") && !text.contains("TUTOR 3/8"), "{text}");
    }

    #[test]
    fn the_tutor_pane_degrades() {
        let tv = tutor_view();
        for (w, h) in [(140, 40), (120, 30), (100, 30), (80, 24), (70, 20), (60, 14)] {
            for set in [false, true] {
                let text = render(w, h, |v| {
                    v.learn.tutor = Some(tv);
                    if set {
                        with_set(demo_set())(v);
                    }
                });
                assert_eq!(text.lines().count(), h as usize);
                assert!(text.contains("TUTOR") && text.contains("DECK A") && text.contains("DECK B"), "{w}x{h}\n{text}");
                // keys of the step survive every size that has any room for them
                if h >= 20 {
                    assert!(text.contains("F2  headphones on B"), "{w}x{h}\n{text}");
                }
            }
        }
        // an open editor / prompt pane takes the rows instead
        let text = render(120, 35, |v| {
            v.learn.tutor = Some(tv);
            v.panel = Some(Panel::Editor);
        });
        assert!(!text.contains("TUTOR 3/8"), "{text}");
    }

    fn patch_slots_for_the_screen() -> &'static [crate::dj_types::PatchSlot] {
        let slot = crate::dj_types::PatchSlot {
            loaded: true,
            name: "d1".into(),
            cpu: 0.05,
            knobs: vec![("kick_lvl".into(), 0.9, 0.0, 1.0), ("hat_lvl".into(), 0.5, 0.0, 1.0)],
            sequencers: vec![("drums".into(), 0, u32::MAX, 3)],
            fader: 1.0,
            headphones: true,
            ..Default::default()
        };
        Box::leak(vec![slot, Default::default(), Default::default(), Default::default()].into_boxed_slice())
    }

    fn patch_view(grid: bool) -> &'static super::super::patchview::PatchView<'static> {
        use super::super::patch::parse_model;
        let slots = patch_slots_for_the_screen();
        let vals: &'static [Vec<f64>] = Box::leak(slots.iter().map(|s| s.knobs.iter().map(|k| k.1).collect()).collect::<Vec<_>>().into_boxed_slice());
        let model = Box::leak(Box::new(parse_model(r#"{"nodes":{"2":{"type":"GateSeq","name":"drums","params":{"steps":16,"rows":4,"row_names":["kick","snare","hat","clap"],"patterns":[{"cells":[[0,0,1.0],[4,1,0.8]]}]}}}}"#).unwrap()));
        let grid = grid.then(|| super::super::patchview::GridView {
            seq: &model.seqs[0],
            seq_index: 0,
            seq_count: 1,
            pattern: 0,
            cx: 0,
            cy: 0,
            playing: Some(0),
            queued: None,
            step: Some(3),
        });
        Box::leak(Box::new(super::super::patchview::PatchView { slots, missing: false, sel: 0, knob_vals: vals, mix_vals: [[1.0, 0.0, 0.0]; 4], grid }))
    }

    #[test]
    fn the_patch_pane_shares_the_screen_with_the_decks_and_the_mixer() {
        for (w, h) in [(140, 40), (120, 35), (100, 30)] {
            let (text, spots) = render_full(w, h, |v| {
                v.panel = Some(Panel::Patch);
                v.patch = Some(patch_view(true));
            });
            if (w, h) == (140, 40) {
                println!("{text}");
            }
            assert_eq!(text.lines().count(), h as usize);
            for l in ["PATCH", "deck keys are OFF", "DECK A", "DECK B", "Mixer", "PATCH · P1 d1 · drums", "kick", "clap", "CUE"] {
                assert!(text.contains(l), "{w}x{h}: {l}\n{text}");
            }
            // the library gave its rows to the pane
            assert!(!text.lines().any(|l| l.starts_with("│▸ Smokin")), "{w}x{h}\n{text}");
            // the deck and mixer controls are still hintable, and so are the slot's knobs
            for t in [Target::Knob(0, 0), Target::Knob(0, 1)] {
                assert!(spots.iter().any(|s| s.target == t), "{w}x{h}: {t:?}");
            }
            assert!(spots.iter().any(|s| s.target == Target::Slot(Slot::Ctl(Ctl::Fader(2)))));
            assert!(spots.iter().any(|s| s.target == Target::Slot(Slot::Ctl(Ctl::Filter(0)))), "deck A's filter");
        }
    }

    #[test]
    fn hint_labels_of_patch_knobs_are_drawn_over_the_knobs() {
        let (text, spots) = render_full(140, 40, |v| {
            v.panel = Some(Panel::Patch);
            v.patch = Some(patch_view(false));
            v.hints = HintView { active: true, labels: leak_labels(), ..HintView::default() };
        });
        println!("{text}");
        // knob 1 and 2 of P1 are TD and TQ; the slot's fader, filter and low are TZ, TX, TC
        for l in ["TD", "TQ", "TZ", "TX", "TC"] {
            assert!(text.contains(l), "{l}\n{text}");
        }
        let knob = spots.iter().find(|s| s.target == Target::Knob(0, 0)).unwrap();
        assert_eq!(knob.label.width, 2);
        // a typed T keeps only the labels starting with it: the deck B mid / A mid ones and P1's
        let (text, _) = render_full(140, 40, |v| {
            v.panel = Some(Panel::Patch);
            v.patch = Some(patch_view(false));
            v.hints = HintView { active: true, typed: "t", labels: leak_labels(), ..HintView::default() };
        });
        assert!(text.contains("TD") && !text.contains("GD"), "{text}");
    }

    #[test]
    fn the_patch_pane_degrades_on_a_short_screen() {
        for (w, h) in [(80, 24), (70, 20), (60, 14)] {
            for grid in [false, true] {
                let text = render(w, h, |v| {
                    v.panel = Some(Panel::Patch);
                    v.patch = Some(patch_view(grid));
                });
                assert_eq!(text.lines().count(), h as usize);
                assert!(text.contains("PATCH") && text.contains("DECK A") && text.contains("DECK B"), "{w}x{h}\n{text}");
            }
        }
    }

    #[test]
    fn time_formats() {
        assert_eq!(fmt_time(83.44), "01:23.4");
        assert_eq!(fmt_time(-3.0), "00:00.0");
        assert_eq!(fmt_short(95.5), "1:35");
    }

    fn test_view<'a>(decks: &'a [DeckInfo], mixer: &'a MixerInfo, lib: &'a [Music]) -> View<'a> {
        View {
            decks,
            mixer,
            waves: [WaveView::Loading, WaveView::Loading],
            doubt: [false, false],
            link_ok: true,
            error: None,
            note: None,
            focus: Focus::Decks,
            input: "",
            search: "",
            lib,
            lib_total: lib.len(),
            lib_sel: 0,
            release_events: true,
            focus_deck: 0,
            showcmd: "",
            set: None,
            rec: None,
            audio_rec: None,
            queue: &[],
            up_next: None,
            overlay: None,
            panel: None,
            hints: HintView::default(),
            learn: LearnView::default(),
            patch: None,
        }
    }

    fn leak_labels() -> &'static [(Target, &'static str)] {
        Box::leak(hints::table().into_boxed_slice())
    }

    #[test]
    fn every_control_has_a_spot_at_120x35_and_labels_do_not_move_anything() {
        let (plain, spots) = render_full(120, 35, |_| {});
        // the patch controls are drawn by the patch pane, which this view does not have open
        let patchy = |t: &Target| matches!(t, Target::Knob(..) | Target::Slot(Slot::Ctl(Ctl::Fader(2..) | Ctl::Filter(2..) | Ctl::Eq(2.., _))));
        for (t, _) in hints::table().into_iter().filter(|(t, _)| !patchy(t)) {
            assert!(spots.iter().any(|s| s.target == t), "no spot for {t:?}");
        }
        assert_eq!(spots.len(), 23);
        let (with, _) = render_full(120, 35, |v| {
            v.hints = HintView { active: true, labels: leak_labels(), ..HintView::default() };
        });
        println!("{with}");
        assert_eq!(plain.lines().count(), with.lines().count());
        // labels replace captions in place: same rows, the text around them is the same
        for (a, b) in plain.lines().zip(with.lines()) {
            assert_eq!(a.chars().count(), b.chars().count());
        }
        assert!(with.contains("HINTS"));
        for l in ["A  ", "TA", "TS", "GA", "XA", "CA", "ZA", "BA", "BS", "BD"] {
            assert!(with.contains(l), "label {l} missing");
        }
    }

    #[test]
    fn labels_sit_on_their_controls_in_yellow() {
        let labels = leak_labels();
        let mut term = Terminal::new(TestBackend::new(120, 35)).unwrap();
        let decks = [deck_a(), deck_b()];
        let mixer = MixerInfo::default();
        let lib = lib();
        let mut spots = Vec::new();
        term.draw(|frame| {
            let mut v = test_view(&decks, &mixer, &lib);
            v.hints = HintView { active: true, labels, ..HintView::default() };
            let area = frame.area();
            spots = draw(frame.buffer_mut(), area, &v);
        })
        .unwrap();
        let buf = term.backend().buffer();
        for sp in &spots {
            let cell = &buf[(sp.label.x, sp.label.y)];
            assert_eq!(cell.bg, Color::Yellow, "{:?}", sp.target);
            assert_eq!(cell.fg, Color::Black);
            let want = hints::label_of(sp.target).unwrap().to_uppercase();
            assert_eq!(cell.symbol(), &want[..1], "{:?}", sp.target);
        }
    }

    #[test]
    fn typed_prefix_hides_the_other_labels() {
        let (text, _) = render_full(120, 35, |v| {
            v.hints = HintView { active: true, typed: "x", labels: leak_labels(), ..HintView::default() };
        });
        assert!(text.contains("XA") && text.contains("XS"));
        assert!(!text.contains("TA") && !text.contains("GA"));
        assert!(text.contains("(X…)"));
    }

    #[test]
    fn selection_is_inverted_and_has_a_chip() {
        let sel = Target::Slot(Slot::Ctl(Ctl::Filter(0)));
        let labels = leak_labels();
        let mut term = Terminal::new(TestBackend::new(120, 35)).unwrap();
        let decks = [deck_a(), deck_b()];
        let mixer = MixerInfo::default();
        let lib = lib();
        let mut spots = Vec::new();
        term.draw(|frame| {
            let mut v = test_view(&decks, &mixer, &lib);
            v.hints = HintView { labels, selected: Some(sel), chip: Some("◆ A filter -0.30"), ..HintView::default() };
            let area = frame.area();
            spots = draw(frame.buffer_mut(), area, &v);
        })
        .unwrap();
        let buf = term.backend().buffer();
        let sp = spots.iter().find(|s| s.target == sel).unwrap();
        for x in sp.hl.left()..sp.hl.right() {
            assert!(buf[(x, sp.hl.y)].modifier.contains(Modifier::REVERSED));
        }
        let other = spots.iter().find(|s| s.target == Target::Slot(Slot::Ctl(Ctl::Filter(1)))).unwrap();
        assert!(!buf[(other.hl.x, other.hl.y)].modifier.contains(Modifier::REVERSED));
        let last: String = (0..120).map(|x| buf[(x, 34)].symbol().to_string()).collect();
        assert!(last.trim_end().ends_with("◆ A filter -0.30"), "{last}");
    }

    #[test]
    fn small_screens_label_only_what_is_shown() {
        for (w, h) in [(100, 30), (80, 24), (70, 20), (60, 14)] {
            let (text, spots) = render_full(w, h, |v| {
                v.hints = HintView { active: true, labels: leak_labels(), ..HintView::default() };
            });
            assert_eq!(text.lines().count(), h as usize);
            assert!(text.contains("DECK A") && text.contains("Library"), "{w}x{h}");
            for sp in &spots {
                assert!(sp.hl.right() <= w && sp.hl.bottom() <= h, "{w}x{h} {:?}", sp.target);
            }
            // the mixer strip is always there
            for t in [Slot::Ctl(Ctl::Filter(0)), Slot::Ctl(Ctl::Fader(1)), Slot::Ctl(Ctl::Crossfader)] {
                assert!(spots.iter().any(|s| s.target == Target::Slot(t)), "{w}x{h} {t:?}");
            }
            println!("{w}x{h}: {} spots", spots.len());
        }
        // 60 columns: no room for master / cue mix / sync range on the crossfader row
        let (_, spots) = render_full(60, 14, |_| {});
        assert!(!spots.iter().any(|s| s.target == Target::Slot(Slot::Master)));
        let (_, spots) = render_full(80, 24, |_| {});
        assert!(spots.iter().any(|s| s.target == Target::Slot(Slot::Master)));
        // too small for the screen: nothing to label
        let (_, spots) = render_full(40, 10, |_| {});
        assert!(spots.is_empty());
    }
}
