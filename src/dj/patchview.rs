//! Drawing the patch pane (slots, knobs, sequencers and the step grid). Pure, like `render`: it
//! takes a [`PatchView`] and records a [`Spot`] for every control that gets a hint label.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Widget},
};

use super::hints::{self, Target, KNOBS_LABELLED};
use super::keys::Ctl;
use super::patch::{pattern_letter, SeqKind, SeqModel, NONE_QUEUED, PATTERNS};
use super::patchdef::SLOTS;
use super::render::{add_spot, bipolar, dim, trunc, unipolar, Spot};
use crate::dj_types::PatchSlot;

/// the sequencer being edited, with the cursor and what the daemon says it is playing
pub struct GridView<'a> {
    pub seq: &'a SeqModel,
    /// index among the slot's sequencers and how many there are
    pub seq_index: usize,
    pub seq_count: usize,
    /// the pattern being edited
    pub pattern: usize,
    pub cx: usize,
    pub cy: usize,
    /// the pattern that plays, the queued one, the step that plays
    pub playing: Option<u32>,
    pub queued: Option<u32>,
    pub step: Option<u32>,
}

pub struct PatchView<'a> {
    pub slots: &'a [PatchSlot],
    /// codjd has no patch decks
    pub missing: bool,
    /// the selected slot, 0..3
    pub sel: usize,
    /// per slot, the value of every knob as the screen should show it
    pub knob_vals: &'a [Vec<f64>],
    /// per slot: fader, filter, low as shown
    pub mix_vals: [[f64; 3]; SLOTS],
    pub grid: Option<GridView<'a>>,
}

const LABEL_W: usize = 8;
const KNOB_NAME_W: usize = 9;
const KNOB_BAR_W: usize = 6;
/// width of a knob group: name, bar and value
const KNOB_W: usize = KNOB_NAME_W + 1 + KNOB_BAR_W + 1 + 5;
const VEL: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const CUE_BADGE: Style = Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD);
const LIVE_BADGE: Style = Style::new().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD);

fn dot(v: f64) -> char {
    VEL[((v.clamp(0.0, 1.0) * 7.99) as usize).min(7)]
}

/// One control: spans drawn from `x`, hinted as `target` when it has one
struct Group {
    spans: Vec<Span<'static>>,
    target: Option<Target>,
}

fn group(text: impl Into<String>, style: Style) -> Group {
    Group { spans: vec![Span::styled(text.into(), style)], target: None }
}

fn plain(text: impl Into<String>) -> Group {
    group(text, Style::default())
}

fn width(g: &Group) -> u16 {
    g.spans.iter().map(|s| s.content.chars().count() as u16).sum()
}

/// Draws groups left to right on row `y`, clipped to `clip`; hinted ones record a spot (only when
/// they fit whole). Returns the x after the last one.
fn put(buf: &mut Buffer, clip: Rect, spots: &mut Vec<Spot>, x: u16, y: u16, groups: &[Group]) -> u16 {
    let mut x = x;
    for g in groups {
        let w = width(g);
        if let Some(t) = g.target {
            add_spot(spots, clip, t, x, y, 2, w);
        }
        let mut sx = x;
        for s in &g.spans {
            if sx >= clip.right() {
                break;
            }
            let room = (clip.right() - sx) as usize;
            buf.set_stringn(sx, y, s.content.as_ref(), room, s.style);
            sx += s.content.chars().count() as u16;
        }
        x += w;
    }
    x
}

fn slot_line(slot: usize, s: &PatchSlot, selected: bool, mix: [f64; 3], w: usize) -> Vec<Group> {
    let mark = if selected { "▸" } else { " " };
    let head = group(format!("{mark}P{} ", slot + 1), Style::default().add_modifier(Modifier::BOLD).fg(if selected { Color::Yellow } else { Color::White }));
    if !s.loaded {
        return vec![head, group("— empty", dim()), group(format!("   :patch load {} <name>", slot + 1), dim())];
    }
    let name_w = if w >= 100 { 14 } else { 10 };
    let mut g = vec![head, group(trunc(&s.name, name_w), Style::default().add_modifier(Modifier::BOLD))];
    if !s.title.is_empty() && s.title != s.name && w >= 90 {
        g.push(group(format!(" “{}”", trunc(&s.title, 16)), dim()));
    }
    if s.dirty {
        g.push(group(" ●", Style::default().fg(Color::Yellow)));
    }
    g.push(plain(" "));
    g.push(if s.live { group(" LIVE ", LIVE_BADGE) } else { group(" CUE ", CUE_BADGE) });
    let cpu = format!(" cpu {:>2.0}%", s.cpu * 100.0);
    g.push(group(cpu, if s.cpu >= 0.25 { Style::default().fg(Color::Red) } else { dim() }));
    let mixer = |name: &str, spans: Vec<Span<'static>>, target: Target| {
        let mut all = vec![Span::raw(format!(" {name} "))];
        all.extend(spans);
        Group { spans: all, target: Some(target) }
    };
    let deck = hints::slot_deck(slot as u8);
    let t = |c: Ctl| Target::Slot(super::keys::Slot::Ctl(c));
    // the label sits over the first two letters of the caption, after the leading space
    g.push(mixer("fader", unipolar(mix[0], 6, Color::Green), t(Ctl::Fader(deck))));
    g.push(mixer("filt", bipolar(mix[1], 7, Color::Yellow), t(Ctl::Filter(deck))));
    g.push(mixer("low", bipolar(mix[2], 7, Color::Red), t(Ctl::Eq(deck, 0))));
    if !s.muted_reason.is_empty() {
        g.push(group(format!("  ✖ MUTED: {}", s.muted_reason), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
    }
    g
}

fn knob_group(slot: usize, i: usize, k: &(String, f64, f64, f64), shown: f64) -> Group {
    let span = (k.3 - k.2).abs().max(f64::EPSILON);
    let frac = ((shown - k.2) / span).clamp(0.0, 1.0);
    let mut spans = vec![Span::styled(format!("{:<KNOB_NAME_W$}", trunc(&k.0, KNOB_NAME_W)), Style::default().fg(Color::White))];
    spans.push(Span::raw(" "));
    spans.extend(unipolar(frac, KNOB_BAR_W, Color::Cyan));
    spans.push(Span::styled(format!(" {shown:>5.2}"), Style::default().fg(Color::White)));
    Group { spans, target: (i < KNOBS_LABELLED).then_some(Target::Knob(slot as u8, i as u8)) }
}

fn seq_line(s: &PatchSlot) -> Vec<Group> {
    let mut g = vec![group("    ▶ ", Style::default().fg(Color::Green))];
    if s.sequencers.is_empty() {
        g.push(group("no sequencer (knobs only)", dim()));
    }
    for (i, (name, pattern, queued, step)) in s.sequencers.iter().enumerate() {
        if i > 0 {
            g.push(group("  ·  ", dim()));
        }
        g.push(group(format!("{name} "), Style::default().fg(Color::White)));
        g.push(group(pattern_letter(*pattern as usize).to_string(), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
        if *queued != NONE_QUEUED {
            g.push(group(format!("→{}", pattern_letter(*queued as usize)), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
        }
        g.push(group(format!("  step {}", step + 1), dim()));
    }
    g
}

/// Rows the slot's knobs take at `w` columns, at most `cap`
fn knob_rows(s: &PatchSlot, w: usize, cap: usize) -> Vec<Vec<usize>> {
    let per = (w.saturating_sub(4) / (KNOB_W + 2)).max(1);
    let all: Vec<usize> = (0..s.knobs.len()).collect();
    all.chunks(per).take(cap).map(|c| c.to_vec()).collect()
}

/// Draws the pane into `area` (a bordered box) and records the hinted controls
pub fn draw_patch(buf: &mut Buffer, area: Rect, pv: &PatchView, spots: &mut Vec<Spot>) {
    if area.width < 30 || area.height < 4 {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
    let footer = if pv.grid.is_some() {
        " hjkl move · x toggle · 1-9 velocity · A-H pattern · ⏎ queue · p live/cue · u undo · :w save · Tab seq · ? keys "
    } else {
        " 1-4 open the grid · p live/cue · u ^r undo · f hints then ] [ edit · :patch new|load|edit · ? keys · Esc closes "
    };
    let sel = pv.slots.get(pv.sel).filter(|s| s.loaded);
    let title = match (&pv.grid, sel) {
        (Some(g), Some(s)) => format!(" PATCH · P{} {} · {} ({}/{}) ", pv.sel + 1, s.name, if g.seq.name.is_empty() { "sequencer" } else { &g.seq.name }, g.seq_index + 1, g.seq_count),
        _ => " PATCH DECKS · P1–P4 ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(title, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)))
        .title_bottom(Span::styled(footer.to_string(), dim()));
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height == 0 {
        return;
    }
    let w = inner.width as usize;
    let x0 = inner.x + 1;
    let clip = Rect::new(inner.x, inner.y, inner.width, inner.height);
    if pv.missing {
        buf.set_stringn(x0, inner.y, "codjd has no patch decks: update codjd", w.saturating_sub(2), Style::default().fg(Color::Yellow));
        return;
    }
    let h = inner.height as usize;
    let mut y = inner.y;
    let bottom = inner.bottom();
    let line = |buf: &mut Buffer, spots: &mut Vec<Spot>, y: &mut u16, groups: Vec<Group>| {
        if *y < bottom {
            put(buf, clip, spots, x0, *y, &groups);
            *y += 1;
        }
    };
    let knobs = |buf: &mut Buffer, spots: &mut Vec<Spot>, y: &mut u16, i: usize, cap: usize| {
        let s = &pv.slots[i];
        for row in knob_rows(s, w, cap) {
            let mut gs = vec![plain("    ")];
            for k in row {
                let shown = pv.knob_vals.get(i).and_then(|v| v.get(k)).copied().unwrap_or(s.knobs[k].1);
                gs.push(knob_group(i, k, &s.knobs[k], shown));
                gs.push(plain("  "));
            }
            if *y < bottom {
                put(buf, clip, spots, x0, *y, &gs);
                *y += 1;
            }
        }
    };

    let Some(g) = &pv.grid else {
        // overview: the lines every slot gets depend on the room
        let per = (h / SLOTS).clamp(1, 4);
        for (i, s) in pv.slots.iter().enumerate().take(SLOTS) {
            line(buf, spots, &mut y, slot_line(i, s, i == pv.sel, pv.mix_vals[i], w));
            if !s.loaded {
                continue;
            }
            if per >= 2 {
                knobs(buf, spots, &mut y, i, if per >= 4 { 2 } else { 1 });
            }
            if per >= 3 && !s.sequencers.is_empty() {
                line(buf, spots, &mut y, seq_line(s));
            }
        }
        return;
    };

    // grid: the slots shrink to what is left after the grid's own rows
    let grid_rows = g.seq.rows;
    let need_full = 2 + grid_rows + 4 + 3;
    let need_sel = 2 + grid_rows + 3;
    if h >= need_full {
        for (i, s) in pv.slots.iter().enumerate().take(SLOTS) {
            line(buf, spots, &mut y, slot_line(i, s, i == pv.sel, pv.mix_vals[i], w));
            if i == pv.sel && s.loaded {
                knobs(buf, spots, &mut y, i, 1);
                line(buf, spots, &mut y, seq_line(s));
            }
        }
    } else if let Some(s) = sel {
        line(buf, spots, &mut y, slot_line(pv.sel, s, true, pv.mix_vals[pv.sel], w));
        if h >= need_sel {
            knobs(buf, spots, &mut y, pv.sel, 1);
            line(buf, spots, &mut y, seq_line(s));
        }
    }
    let grid_area = Rect::new(inner.x, y, inner.width, bottom.saturating_sub(y));
    draw_grid(buf, grid_area, g);
}

/// the header, the beat ruler and the rows of a sequencer
fn draw_grid(buf: &mut Buffer, area: Rect, g: &GridView) {
    if area.height < 3 || area.width < 20 {
        return;
    }
    let seq = g.seq;
    let x0 = area.x + 1;
    // header: the eight patterns, the edited one inverted, the playing one green, the queued one yellow
    let mut x = x0;
    let label = format!("{} ", if seq.kind == SeqKind::Gate { "drums" } else { "notes" });
    buf.set_string(x, area.y, &label, dim());
    x += label.chars().count() as u16;
    for p in 0..PATTERNS {
        let mut style = Style::default().fg(Color::White);
        if g.playing == Some(p as u32) {
            style = style.fg(Color::Green).add_modifier(Modifier::BOLD);
        }
        if g.queued == Some(p as u32) {
            style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        if g.pattern == p {
            style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        }
        buf.set_string(x, area.y, format!(" {} ", pattern_letter(p)), style);
        x += 3;
    }
    let mut status = String::new();
    if let Some(p) = g.playing {
        status.push_str(&format!("  plays {}", pattern_letter(p as usize)));
    }
    if let Some(q) = g.queued {
        status.push_str(&format!(" · {} next bar", pattern_letter(q as usize)));
    }
    if g.playing.is_some_and(|p| p as usize != g.pattern) {
        status.push_str(" · editing another pattern: ⏎ queues it");
    }
    buf.set_stringn(x, area.y, status, (area.right().saturating_sub(x)) as usize, dim());

    let label_w = (0..seq.rows).map(|r| seq.row_label(r).chars().count()).max().unwrap_or(4).clamp(4, LABEL_W);
    let cells_x = x0 + label_w as u16 + 1;
    let avail = area.right().saturating_sub(cells_x) as usize;
    // first visible step: scroll sideways to keep the cursor in view (2 columns a step, a gap per beat)
    let cols = |n: usize| n * 2 + n / 4;
    let mut first = 0;
    while first < g.cx && cols(g.cx - first + 1) > avail {
        first += 1;
    }
    let mut last = first;
    while last < seq.steps && cols(last - first + 1) <= avail {
        last += 1;
    }
    let x_of = |step: usize| cells_x + (((step - first) * 2) + (step / 4 - first / 4)) as u16;
    // ruler: the beat numbers, and a marker over the playing step
    let ruler_y = area.y + 1;
    for step in first..last {
        if step % 4 == 0 {
            buf.set_string(x_of(step), ruler_y, format!("{}", step / 4 + 1), dim());
        }
        if g.step == Some(step as u32) && g.playing.map_or(true, |p| p as usize == g.pattern) {
            buf.set_string(x_of(step), ruler_y, "▾", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
        }
    }
    // rows, scrolled to keep the cursor row in view
    let visible = (area.height - 2) as usize;
    let top = if g.cy < visible { 0 } else { g.cy + 1 - visible };
    for (i, cy) in (top..seq.rows.min(top + visible)).enumerate() {
        let y = area.y + 2 + i as u16;
        let row = seq.row_at(cy);
        let on_row = cy == g.cy;
        let name = format!("{:<label_w$}", trunc(&seq.row_label(row), label_w));
        buf.set_string(x0, y, name, if on_row { Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow) } else { Style::default().fg(Color::Gray) });
        for step in first..last {
            let cell = seq.cell(g.pattern, step, row);
            let at_step = g.step == Some(step as u32) && g.playing.map_or(true, |p| p as usize == g.pattern);
            let mut style = match cell {
                Some(_) => Style::default().fg(if at_step { Color::Yellow } else { Color::Cyan }).add_modifier(if at_step { Modifier::BOLD } else { Modifier::empty() }),
                None => dim(),
            };
            if at_step {
                style = style.bg(Color::Rgb(40, 40, 40));
            }
            if on_row && step == g.cx {
                style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
            }
            let ch = match (cell, seq.kind) {
                (Some(v), SeqKind::Gate) => dot(v),
                (Some(_), SeqKind::Mono) => '●',
                (None, _) => '·',
            };
            buf.set_string(x_of(step), y, ch.to_string(), style);
            // the background of the moving column also fills the gap after the cell
            if at_step {
                buf.set_string(x_of(step) + 1, y, " ", Style::default().bg(Color::Rgb(40, 40, 40)));
            }
        }
    }
    // a cursor on a drum cell tells its velocity
    if let Some(v) = seq.cell(g.pattern, g.cx, seq.row_at(g.cy)).filter(|_| seq.kind == SeqKind::Gate) {
        let text = format!("velocity {:.2}", v);
        let len = text.chars().count() as u16;
        if area.width > len + 2 {
            buf.set_string(area.right() - len - 1, area.y + 1, text, dim());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::patch::{parse_model, PatchModel};
    use ratatui::{backend::TestBackend, Terminal};

    const TEXT: &str = r#"{"nodes":{
      "2":{"type":"GateSeq","name":"drums","params":{"steps":16,"rows":4,"row_names":["kick","snare","hat","clap"],
           "patterns":[{"cells":[[0,0,1.0],[4,0,1.0],[4,1,0.5],[2,2,0.6]]}]}},
      "3":{"type":"MonoSeq","name":"bass","params":{"scale":"A minor","root":45,"steps":8,"rows":8,"patterns":[{"cells":[[0,0],[2,3]]}]}}}}"#;

    fn slot(name: &str, live: bool) -> PatchSlot {
        PatchSlot {
            loaded: true,
            name: name.into(),
            title: "title of it".into(),
            live,
            cpu: 0.07,
            knobs: vec![("cutoff".into(), 0.3, 0.0, 1.0), ("reso".into(), 0.6, 0.0, 0.95)],
            sequencers: vec![("drums".into(), 0, 2, 5)],
            fader: 0.8,
            headphones: !live,
            ..Default::default()
        }
    }

    fn slots() -> Vec<PatchSlot> {
        let mut muted = slot("hog", true);
        muted.muted_reason = "cpu overrun".into();
        muted.dirty = true;
        vec![slot("d1", false), muted, PatchSlot::default(), PatchSlot::default()]
    }

    fn text(buf: &Buffer, area: Rect) -> String {
        (area.top()..area.bottom())
            .map(|y| (area.left()..area.right()).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw<'a>(w: u16, h: u16, f: impl FnOnce(&mut PatchView<'a>)) -> (String, Vec<Spot>) {
        let slots: &'static [PatchSlot] = Box::leak(slots().into_boxed_slice());
        let vals: &'static [Vec<f64>] = Box::leak(slots.iter().map(|s| s.knobs.iter().map(|k| k.1).collect()).collect::<Vec<_>>().into_boxed_slice());
        let mut pv = PatchView { slots, missing: false, sel: 0, knob_vals: vals, mix_vals: [[0.8, 0.0, 0.0]; 4], grid: None };
        f(&mut pv);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut spots = Vec::new();
        term.draw(|f| {
            let area = f.area();
            draw_patch(f.buffer_mut(), area, &pv, &mut spots)
        })
        .unwrap();
        let area = Rect::new(0, 0, w, h);
        (text(term.backend().buffer(), area), spots)
    }

    fn model() -> PatchModel {
        parse_model(TEXT).unwrap()
    }

    #[test]
    fn the_overview_shows_slots_badges_cpu_knobs_and_sequencers() {
        let (t, spots) = draw(120, 20, |_| {});
        println!("{t}");
        for l in ["PATCH DECKS", "▸P1 d1", "title of it", "CUE", "LIVE", "cpu  7%", "cutoff", "0.30", "reso", "drums A→C", "step 6", "P3 — empty", ":patch load 3 <name>", "P4 — empty"] {
            assert!(t.contains(l), "{l}\n{t}");
        }
        // the slot that hogs the cpu is marked: dirty dot, muted reason
        assert!(t.contains("P2 hog “title of it” ●") && t.contains("MUTED: cpu overrun"), "{t}");
        // every loaded slot has its three mixer controls and its knobs hinted
        for slot in 0..2u8 {
            for i in 0..2u8 {
                assert!(spots.iter().any(|s| s.target == Target::Knob(slot, i)), "P{} knob {i}", slot + 1);
            }
            for c in [Ctl::Fader(2 + slot), Ctl::Filter(2 + slot), Ctl::Eq(2 + slot, 0)] {
                assert!(spots.iter().any(|s| s.target == Target::Slot(super::super::keys::Slot::Ctl(c))), "{c:?}");
            }
        }
        // empty slots have nothing to pick; no two controls share a place
        assert!(!spots.iter().any(|s| matches!(s.target, Target::Knob(2, _) | Target::Knob(3, _))));
        assert_eq!(spots.len(), 2 * (2 + 3));
        for (i, a) in spots.iter().enumerate() {
            for b in &spots[i + 1..] {
                assert!(a.hl.intersection(b.hl).area() == 0, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn live_and_cue_are_told_apart_and_a_missing_daemon_says_so() {
        let (t, _) = draw(120, 20, |_| {});
        let p1 = t.lines().find(|l| l.contains("P1 d1")).unwrap();
        let p2 = t.lines().find(|l| l.contains("P2 hog")).unwrap();
        assert!(p1.contains(" CUE ") && !p1.contains("LIVE"), "{p1}");
        assert!(p2.contains(" LIVE ") && !p2.contains(" CUE "), "{p2}");
        let (t, spots) = draw(100, 12, |pv| pv.missing = true);
        assert!(t.contains("codjd has no patch decks: update codjd"), "{t}");
        assert!(spots.is_empty());
    }

    #[test]
    fn muted_reason_is_red_and_the_cpu_turns_red_at_the_budget() {
        let slots = slots();
        let vals: Vec<Vec<f64>> = slots.iter().map(|s| s.knobs.iter().map(|k| k.1).collect()).collect();
        let mut hot = slots.clone();
        hot[0].cpu = 0.3;
        let pv = PatchView { slots: &hot, missing: false, sel: 0, knob_vals: &vals, mix_vals: [[0.0; 3]; 4], grid: None };
        let mut term = Terminal::new(TestBackend::new(120, 20)).unwrap();
        let mut spots = Vec::new();
        term.draw(|f| {
            let area = f.area();
            draw_patch(f.buffer_mut(), area, &pv, &mut spots)
        })
        .unwrap();
        let buf = term.backend().buffer();
        let find = |needle: &str| {
            (0..20u16).find_map(|y| {
                let line: String = (0..120u16).map(|x| buf[(x, y)].symbol().to_string()).collect();
                line.find(needle).map(|i| (line[..i].chars().count() as u16, y))
            })
        };
        let (x, y) = find("MUTED").unwrap();
        assert_eq!(buf[(x, y)].fg, Color::Red);
        let (x, y) = find("cpu 30%").unwrap();
        assert_eq!(buf[(x + 1, y)].fg, Color::Red);
        let (x, y) = find("cpu  7%").unwrap();
        assert_ne!(buf[(x + 1, y)].fg, Color::Red);
    }

    #[test]
    fn the_grid_has_row_names_cells_pattern_letters_and_a_moving_column() {
        let m = model();
        let slots = slots();
        let _ = &slots;
        let (t, _) = draw(120, 22, |pv| {
            pv.grid = Some(GridView { seq: &m.seqs[0], seq_index: 0, seq_count: 2, pattern: 0, cx: 4, cy: 1, playing: Some(0), queued: Some(2), step: Some(4) });
        });
        println!("{t}");
        for l in ["PATCH · P1 d1 · drums (1/2)", "kick", "snare", "hat", "clap", "A  B  C", "plays A", "C next bar", "hjkl move", "velocity 0.50"] {
            assert!(t.contains(l), "{l}\n{t}");
        }
        let rows: Vec<&str> = t.lines().filter(|l| l.contains("kick") || l.contains("snare") || l.contains("clap")).collect();
        // kick has steps 0 and 4 on, as full blocks; the snare has a half-velocity hit at step 4
        let kick = rows.iter().find(|l| l.contains("kick")).unwrap();
        assert!(kick.contains("█"), "{kick}");
        let snare = rows.iter().find(|l| l.contains("snare")).unwrap();
        assert!(snare.contains("▄"), "velocity 0.5 is a mid-height block: {snare}");
        // the playing step is marked over the grid
        assert!(t.contains("▾"), "{t}");
        // another pattern is being edited: the playing marker goes, a note says how to queue it
        let (t, _) = draw(120, 22, |pv| {
            pv.grid = Some(GridView { seq: &m.seqs[0], seq_index: 0, seq_count: 2, pattern: 3, cx: 0, cy: 0, playing: Some(0), queued: None, step: Some(4) });
        });
        assert!(!t.contains("▾") && t.contains("editing another pattern"), "{t}");
    }

    #[test]
    fn note_rows_are_scale_degrees_highest_on_top() {
        let m = model();
        let (t, _) = draw(120, 24, |pv| {
            pv.grid = Some(GridView { seq: &m.seqs[1], seq_index: 1, seq_count: 2, pattern: 0, cx: 0, cy: 0, playing: None, queued: None, step: None });
        });
        println!("{t}");
        let lines: Vec<&str> = t.lines().collect();
        let top = lines.iter().position(|l| l.contains("8 A3")).unwrap();
        let bottom = lines.iter().position(|l| l.contains("1 A2")).unwrap();
        assert!(top < bottom, "the highest degree is drawn first\n{t}");
        assert!(lines[bottom].contains("●"), "step 0 plays degree 1: {}", lines[bottom]);
        assert!(t.contains("notes"));
    }

    #[test]
    fn the_grid_scrolls_to_its_cursor_and_degrades_on_a_short_screen() {
        let m = model();
        // a 16-step pattern in a narrow box: the cursor column is always drawn
        for cx in [0, 8, 15] {
            let slots = slots();
            let _ = &slots;
            let (t, _) = draw(50, 20, |pv| {
                pv.grid = Some(GridView { seq: &m.seqs[0], seq_index: 0, seq_count: 2, pattern: 0, cx, cy: 0, playing: None, queued: None, step: None });
            });
            assert!(t.contains("kick"), "{t}");
            let header = t.lines().position(|l| l.contains("drums  A")).unwrap();
            let ruler = t.lines().nth(header + 1).unwrap();
            assert!(ruler.contains(&format!("{}", cx / 4 + 1)), "step {cx} is visible: {ruler:?}\n{t}");
        }
        // too short for the slots: only the selected slot and the grid stay, then the grid alone
        for h in [14, 10, 8, 7] {
            let (t, _) = draw(120, h, |pv| {
                pv.grid = Some(GridView { seq: &m.seqs[0], seq_index: 0, seq_count: 2, pattern: 0, cx: 0, cy: 3, playing: None, queued: None, step: None });
            });
            assert_eq!(t.lines().count(), h as usize);
            assert!(t.contains("clap"), "the cursor row is always visible at {h}\n{t}");
        }
    }

    #[test]
    fn a_too_small_box_draws_nothing() {
        let (t, spots) = draw(20, 3, |_| {});
        assert!(t.trim().is_empty() && spots.is_empty());
    }
}
