//! The `?` overlay: the key map drawn on the actual screen, which-key style. One panel over the
//! decks lists the focused deck's keys, the area of the mixer and library lists the mixer's and the global keys,
//! every entry with a short musical explanation (`glossary`). Pure: it only draws.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use super::glossary::{self, Entry};

fn clear(buf: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
}

fn truncated(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        s.chars().take(w - 1).chain(std::iter::once('…')).collect()
    }
}

/// Draws a box with `rows` (keys, text) lines in one or two columns; what does not fit ends in
/// a "… N more" line
fn list(buf: &mut Buffer, area: Rect, title: &str, color: Color, rows: &[(String, String)], columns: usize) {
    if area.width < 12 || area.height < 3 {
        return;
    }
    clear(buf, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(title.to_string());
    let inner = block.inner(area);
    block.render(area, buf);
    let room = inner.height as usize;
    let columns = if inner.width >= 84 { columns.max(1) } else { 1 };
    let col_w = inner.width / columns as u16;
    let capacity = room * columns;
    let more = rows.len() > capacity;
    let shown = if more { capacity.saturating_sub(1) } else { rows.len() };
    let keys = Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD);
    // rows fill the first column, then the next
    let per_col = if more { room } else { rows.len().div_ceil(columns).min(room).max(1) };
    let width = col_w.saturating_sub(2) as usize;
    let kw = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0).min(width / 3);
    for (i, (k, text)) in rows.iter().take(shown).enumerate() {
        let (col, row) = (i / per_col, i % per_col);
        let x = inner.x + col as u16 * col_w;
        let y = inner.y + row as u16;
        buf.set_stringn(x + 1, y, format!("{:<kw$}", truncated(k, kw)), kw, keys);
        buf.set_stringn(x + 2 + kw as u16, y, truncated(text, width.saturating_sub(kw + 1)), width.saturating_sub(kw + 1), Style::default());
    }
    if more && room > 0 {
        let y = inner.y + room as u16 - 1;
        let note = format!("… {} more: :help keys", rows.len() - shown);
        let x = inner.x + (columns as u16 - 1) * col_w;
        buf.set_stringn(x + 1, y, note, width, Style::default().fg(Color::DarkGray));
    }
}

fn rows(entries: &[Entry], focus: usize) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|e| {
            let (k, t) = glossary::deck_entry(e, focus);
            (k.to_string(), t)
        })
        .collect()
}

/// Lines of the scrolled key map (section headings included): the most `?` can scroll
pub fn lines(focus: u8) -> usize {
    // a heading per section and a blank line between sections
    sections(focus).iter().map(|(_, _, rows)| rows.len() + 2).sum::<usize>() - 1
}

fn sections(focus: u8) -> [(String, Color, Vec<(String, String)>); 3] {
    let name = if focus == 0 { "A" } else { "B" };
    let deck = if focus == 0 { Color::Cyan } else { Color::Magenta };
    [
        (format!("FOCUSED DECK {name}"), deck, rows(glossary::DECK_KEYS, focus as usize)),
        ("MIXER (the focused deck's, or pick with f)".into(), Color::White, rows(glossary::MIXER_KEYS, focus as usize)),
        ("EVERYWHERE".into(), Color::Yellow, rows(glossary::GLOBAL_KEYS, focus as usize)),
    ]
}

/// `decks`: the area of both decks, which holds the one panel of the focused deck's keys;
/// `lower`: the mixer and the library together, which share the mixer's keys and the global
/// ones side by side, or one over the other when narrow. When that does not fit (a small
/// terminal), every key goes into one list that `scroll` moves, like vim's help.
pub fn draw(buf: &mut Buffer, decks: Rect, lower: Rect, focus: u8, scroll: u16) {
    let [(_, deck_color, deck_rows), (_, _, mixer_rows), (_, _, global_rows)] = sections(focus);
    let name = if focus == 0 { "A" } else { "B" };
    let title = format!(" FOCUSED DECK {name} keys · q closes ");
    // the deck panel takes the lines its keys need; the rest goes to the mixer and global keys
    let columns = if decks.width.saturating_sub(2) >= 84 { 2 } else { 1 };
    let need = deck_rows.len().div_ceil(columns) as u16 + 2;
    let full = Rect::new(lower.x, decks.y, lower.width, lower.bottom().saturating_sub(decks.y));
    let (decks, lower) = if decks.height > need && decks.bottom() <= lower.y {
        let top = decks.y + need;
        (Rect { height: need, ..decks }, Rect::new(lower.x, top, lower.width, lower.bottom().saturating_sub(top)))
    } else {
        (decks, lower)
    };
    let (m, g) = (mixer_rows.len() as u16 + 2, global_rows.len() as u16 + 2);
    let wide = lower.width >= 100;
    let fits = decks.height >= need && if wide { lower.height >= m.max(g) } else { lower.height >= m + g };
    if !fits {
        scrolled(buf, full, focus, scroll);
        return;
    }
    list(buf, decks, &title, deck_color, &deck_rows, columns);
    let (mixer, global) = if wide {
        let left = lower.width * 3 / 5;
        (Rect::new(lower.x, lower.y, left, lower.height), Rect::new(lower.x + left, lower.y, lower.width - left, lower.height))
    } else {
        // a narrow screen: the mixer keys on top, the global ones below
        (Rect { height: m, ..lower }, Rect::new(lower.x, lower.y + m, lower.width, lower.height - m))
    };
    list(buf, mixer, " MIXER keys (the focused deck's, or pick with f) ", Color::White, &mixer_rows, 1);
    list(buf, global, " everywhere ", Color::Yellow, &global_rows, 1);
}

/// Every key in one list from line `scroll` on, with a heading per section
fn scrolled(buf: &mut Buffer, area: Rect, focus: u8, scroll: u16) {
    if area.width < 12 || area.height < 3 {
        return;
    }
    clear(buf, area);
    let mut lines: Vec<(Option<Color>, String, String)> = Vec::new();
    for (i, (head, color, rows)) in sections(focus).into_iter().enumerate() {
        if i > 0 {
            lines.push((None, String::new(), String::new()));
        }
        lines.push((Some(color), head, String::new()));
        lines.extend(rows.into_iter().map(|(k, t)| (None, k, t)));
    }
    let room = area.height.saturating_sub(2) as usize;
    let top = (scroll as usize).min(lines.len().saturating_sub(room));
    let last = (top + room).min(lines.len());
    let more = if last < lines.len() { " · j more" } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(format!(" ? KEYS {}-{} of {} · j k scroll · q closes{more} ", top + 1, last, lines.len()));
    let inner = block.inner(area);
    block.render(area, buf);
    let width = inner.width.saturating_sub(2) as usize;
    let kw = lines.iter().filter(|l| l.0.is_none()).map(|l| l.1.chars().count()).max().unwrap_or(0).min(width / 3);
    for (row, (head, k, text)) in lines[top..last].iter().enumerate() {
        let y = inner.y + row as u16;
        match head {
            Some(color) => {
                buf.set_stringn(inner.x + 1, y, k, width, Style::default().fg(*color).add_modifier(Modifier::BOLD));
            }
            None if !k.is_empty() => {
                let keys = Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD);
                buf.set_stringn(inner.x + 1, y, format!("{:<kw$}", truncated(k, kw)), kw, keys);
                let w = width.saturating_sub(kw + 1);
                buf.set_stringn(inner.x + 2 + kw as u16, y, truncated(text, w), w, Style::default());
            }
            None => {}
        }
    }
}
