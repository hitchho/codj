//! Drawing the DJ screen. Pure: everything comes in through [`View`], so it is tested with a
//! `TestBackend` and fake deck state.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::wave::{draw_strip, time_to_col, Waveform};
use crate::dj_types::{DeckInfo, MixerInfo};
use crate::types::Music;

/// Seconds of track shown by the zoomed strip around the playhead
pub const ZOOM_S: f64 = 8.0;
const DECK_COLORS: [Color; 2] = [Color::Cyan, Color::Magenta];
const CUE_COLORS: [Color; 4] = [Color::Red, Color::Green, Color::Yellow, Color::Blue];
const HOTCUE_KEYS: [[&str; 4]; 2] = [["1", "2", "3", "4"], ["7", "8", "9", "0"]];
const DIM: Color = Color::DarkGray;
const MIN_SIZE: (u16, u16) = (60, 14);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Focus {
    /// performance keys are live
    #[default]
    Decks,
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
}

pub fn fmt_time(secs: f64) -> String {
    let t = secs.max(0.0);
    let tenths = (t * 10.0) as u64;
    format!("{:02}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

fn fmt_short(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        s.chars().take(w - 1).chain(std::iter::once('…')).collect()
    }
}

fn dim() -> Style {
    Style::default().fg(DIM)
}

pub fn draw(buf: &mut Buffer, area: Rect, v: &View) {
    // paint the whole screen, the previous (normal) view must not shine through
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        let msg = format!("DJ mode needs at least {}x{} (now {}x{})", MIN_SIZE.0, MIN_SIZE.1, area.width, area.height);
        buf.set_stringn(area.x, area.y, msg, area.width as usize, Style::default().fg(Color::Yellow));
        buf.set_stringn(area.x, area.y + 1, "Esc: back", area.width as usize, dim());
        return;
    }
    let body_h = area.height - 2;
    let mixer_h = 4;
    let lib_h = if body_h >= 20 { (body_h / 4).clamp(5, 9) } else { 3 };
    let decks_h = body_h.saturating_sub(mixer_h + lib_h);
    let header = Rect::new(area.x, area.y, area.width, 1);
    let decks = Rect::new(area.x, area.y + 1, area.width, decks_h);
    let mixer = Rect::new(area.x, decks.bottom(), area.width, mixer_h);
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
        draw_deck(buf, r, i, &info, v.waves[i], v.release_events);
    }
    draw_mixer(buf, mixer, v);
    draw_library(buf, lib, v);
    draw_status(buf, status, v);
}

fn draw_header(buf: &mut Buffer, area: Rect, v: &View) {
    let live = v.error.is_none();
    let left = Line::from(vec![
        Span::styled(" DJ ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(
            "  Esc back · Tab search · < > load A/B · Enter load · Shift+; command",
            dim(),
        ),
    ]);
    Paragraph::new(left).render(area, buf);
    let (text, color) = if live { ("● live ", Color::Green) } else { ("○ no data ", Color::Red) };
    let w = text.chars().count() as u16;
    if area.width > w {
        buf.set_string(area.right() - w, area.y, text, Style::default().fg(color));
    }
}

fn draw_status(buf: &mut Buffer, area: Rect, v: &View) {
    let w = area.width as usize;
    if v.focus == Focus::Command {
        buf.set_stringn(area.x, area.y, format!(":{}█", v.input), w, Style::default().fg(Color::White));
        return;
    }
    if let Some(e) = v.error {
        buf.set_stringn(area.x, area.y, e, w, Style::default().fg(Color::Red));
    } else if let Some(n) = v.note {
        buf.set_stringn(area.x, area.y, n, w, Style::default().fg(Color::Yellow));
    } else {
        let hint = "A: z cue x play c sync v loop a/s pitch q/w nudge 1-4 cues d/f jump | B: / . , m  k/l  o/p  7-0  j/; | Alt+qwer uiop eq/filter | g b h n t y faders";
        buf.set_stringn(area.x, area.y, hint, w, dim());
    }
}

fn draw_deck(buf: &mut Buffer, area: Rect, idx: usize, d: &DeckInfo, wave: WaveView, release: bool) {
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
    let title = Line::from(vec![
        Span::styled(format!(" DECK {name} "), Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {state} "), Style::default().fg(color)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if d.playing { color } else { DIM }))
        .title(title);
    let inner = block.inner(area);
    block.render(area, buf);
    let w = inner.width as usize;
    if !d.loaded {
        let msg = "— empty — pick a track below and press < or > to load";
        buf.set_stringn(inner.x + 1, inner.y + inner.height / 2, msg, w.saturating_sub(1), dim());
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    // title / artist
    let who = if d.artist.is_empty() { d.title.clone() } else { format!("{} — {}", d.title, d.artist) };
    lines.push(Line::from(Span::styled(trunc(&who, w), Style::default().add_modifier(Modifier::BOLD))));
    // tempo, key, rate
    let rate_pct = (d.rate - 1.0) * 100.0;
    let mut tempo = Vec::new();
    if d.has_grid() {
        tempo.push(Span::styled(format!("{:6.2}", d.bpm), Style::default().add_modifier(Modifier::BOLD)));
        tempo.push(Span::raw(" BPM "));
        tempo.push(Span::styled(format!("(trk {:.2})", d.track_bpm), dim()));
    } else {
        tempo.push(Span::styled("no beat grid", Style::default().fg(Color::Yellow)));
    }
    tempo.push(Span::raw("  key "));
    tempo.push(Span::styled(
        if d.key.is_empty() { "--".to_string() } else { d.key.clone() },
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    ));
    tempo.push(Span::raw("  "));
    tempo.push(Span::styled(
        format!("{rate_pct:+.2}%"),
        if rate_pct.abs() < 0.005 { dim() } else { Style::default().fg(Color::Yellow) },
    ));
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
    time.push(if d.loop_beats > 0.0 {
        Span::styled(
            format!(" LOOP {} ", trim_beats(d.loop_beats)),
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("loop --", dim())
    });
    time.push(Span::raw("  "));
    time.push(if d.headphones {
        Span::styled("HP ●", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("HP ○", dim())
    });
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
    lines.push(hotcue_line(d, idx, w));

    // text first, the waveforms get what is left (at least 2 rows if there is room)
    let n_text = lines.len().min(inner.height.saturating_sub(2).max(1) as usize);
    let text_area = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), n_text as u16);
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

fn hotcue_line(d: &DeckInfo, idx: usize, w: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for s in 0..4 {
        let label = HOTCUE_KEYS[idx][s];
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
        // slots 5-8 have no key, but cues set elsewhere still show
        spans.push(Span::styled("more ", dim()));
        for s in 4..8 {
            spans.push(if d.hotcue(s).is_some() {
                Span::styled("● ", Style::default().fg(Color::Yellow))
            } else {
                Span::styled("○ ", dim())
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
                    let cell = &mut buf[(ov.x + c, ov.y)];
                    cell.set_char('▾');
                    cell.set_fg(Color::Yellow);
                }
            }
        }
        if let Some(c) = time_to_col(d.position.min(dur - 1e-6), 0.0, dur, ov.width) {
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
                    let cell = &mut buf[(zoom.x + c, zoom.y)];
                    cell.set_char('▾');
                    cell.set_fg(Color::Yellow);
                }
            }
        }
        playhead(buf, zoom, zoom.width / 2);
        let _ = release;
    }
}

/// Dim background behind `[from, to)` seconds (a loop)
fn shade(buf: &mut Buffer, area: Rect, from: f64, to: f64, t0: f64, t1: f64) {
    for x in 0..area.width {
        let t = t0 + (x as f64 + 0.5) / area.width as f64 * (t1 - t0);
        if t >= from && t < to {
            for y in area.top()..area.bottom() {
                buf[(area.x + x, y)].set_bg(Color::Rgb(20, 60, 25));
            }
        }
    }
}

/// Thin line at the playhead; over a bar it only lightens the background
fn playhead(buf: &mut Buffer, area: Rect, col: u16) {
    if col >= area.width {
        return;
    }
    for y in area.top()..area.bottom() {
        let cell = &mut buf[(area.x + col, y)];
        if cell.symbol() == " " {
            cell.set_char('│');
            cell.set_fg(Color::White);
        } else {
            cell.set_bg(Color::Rgb(95, 95, 95));
        }
    }
}

/// Beat grid lines in the zoomed strip: bar starts bright, other beats dim, in the empty cells
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
        let (ch, color) = if in_bar == 0 { ('┃', Color::Gray) } else { ('╎', Color::DarkGray) };
        for y in area.top()..area.bottom() {
            let cell = &mut buf[(area.x + c, y)];
            if cell.symbol() == " " {
                cell.set_char(ch);
                cell.set_fg(color);
            }
        }
    }
}

fn bipolar(v: f64, width: usize, color: Color) -> Vec<Span<'static>> {
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

fn unipolar(v: f64, width: usize, color: Color) -> Vec<Span<'static>> {
    let filled = ((v.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(width - filled), dim()),
    ]
}

fn mixer_row(d: &DeckInfo, idx: usize, w: usize) -> Line<'static> {
    let color = DECK_COLORS[idx];
    let name = if idx == 0 { "A" } else { "B" };
    let mut spans = vec![Span::styled(
        format!(" {name} "),
        Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD),
    )];
    let slot_w = w.saturating_sub(4) / 5;
    let show_val = slot_w >= 17;
    let bar_w = slot_w.saturating_sub(5 + if show_val { 7 } else { 0 }).max(3);
    let bipolars = [("LOW", d.eq.0), ("MID", d.eq.1), ("HI ", d.eq.2), ("FLT", d.filter)];
    for (label, v) in bipolars {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label, Style::default().fg(color)));
        spans.push(Span::raw(" "));
        spans.extend(bipolar(v, bar_w, color));
        if show_val {
            spans.push(Span::styled(format!(" {v:+.2}"), dim()));
        }
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled("FAD", Style::default().fg(color)));
    spans.push(Span::raw(" "));
    spans.extend(unipolar(d.fader, bar_w + 2, color));
    if show_val {
        spans.push(Span::styled(format!(" {:.2}", d.fader), dim()));
    }
    Line::from(spans)
}

fn draw_mixer(buf: &mut Buffer, area: Rect, v: &View) {
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
    // crossfader
    let bar_w = w.saturating_sub(24).max(9);
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
    xf.push(Span::styled(format!("  {:+.2}", m.crossfader), dim()));
    Paragraph::new(vec![Line::from(title), mixer_row(a, 0, w), mixer_row(b, 1, w), Line::from(xf)])
        .render(area, buf);
}

fn draw_library(buf: &mut Buffer, area: Rect, v: &View) {
    let searching = v.focus == Focus::Search;
    let title = if searching {
        format!(" Library {}/{} — search: {}█ ", v.lib.len(), v.lib_total, v.input)
    } else if !v.search.is_empty() {
        format!(" Library {}/{} — /{} (Tab edits) ", v.lib.len(), v.lib_total, v.search)
    } else {
        format!(" Library {} (Tab: regex search · artist:/title:/genre:) ", v.lib_total)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if searching { Color::Yellow } else { DIM }))
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
        let right = format!("{tags:>6} {len:>5}");
        let w = inner.width as usize;
        let left_w = w.saturating_sub(right.chars().count() + 3);
        let style = if row == sel {
            Style::default().add_modifier(Modifier::REVERSED)
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
        let decks = [deck_a(), deck_b()];
        let mixer = MixerInfo { crossfader: -0.25, mode: "internal".into(), quantize: true, xruns: 0, ..Default::default() };
        let wf = wave();
        let lib = lib();
        let mut v = View {
            decks: &decks,
            mixer: &mixer,
            waves: [WaveView::Ready(&wf), WaveView::Loading],
            error: None,
            note: None,
            focus: Focus::Decks,
            input: "",
            search: "",
            lib: &lib,
            lib_total: 3,
            lib_sel: 1,
            release_events: true,
        };
        f(&mut v);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|frame| {
            let area = frame.area();
            draw(frame.buffer_mut(), area, &v);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
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
        // hot cues: slot 1 and 3 set, 2 and 4 empty
        assert!(text.contains(" 1  0:12 "), "{text}");
        assert!(text.contains(" 3  1:35 "), "{text}");
        assert!(text.contains(" 2  --"), "{text}");
        // a waveform was drawn (bars and the playhead)
        assert!(text.contains('█'));
        assert!(text.contains('│'));
        // phase meter: beat 3 lit
        assert!(text.contains("3 ██"));
        // exactly 35 lines, none wider than the screen
        assert_eq!(text.lines().count(), 35);
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
        assert!(text.contains('▾'));
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
        assert!(text.contains("129.55"));
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
        assert!(text.contains("search: tekno█"));
        let text = render(120, 35, |v| {
            v.focus = Focus::Command;
            v.input = "q";
        });
        assert!(text.lines().last().unwrap().starts_with(":q█"));
    }

    #[test]
    fn time_formats() {
        assert_eq!(fmt_time(83.44), "01:23.4");
        assert_eq!(fmt_time(-3.0), "00:00.0");
        assert_eq!(fmt_short(95.5), "1:35");
    }
}
