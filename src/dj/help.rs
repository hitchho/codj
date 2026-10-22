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

/// `decks`: the area of both decks, which holds the one panel of the focused deck's keys;
/// `lower`: the mixer and the library together, which share the mixer's keys and the global
/// ones side by side, or one over the other when narrow.
pub fn draw(buf: &mut Buffer, decks: Rect, lower: Rect, focus: u8) {
    let colors = [Color::Cyan, Color::Magenta];
    let name = if focus == 0 { "A" } else { "B" };
    let title = format!(" FOCUSED DECK {name} keys · ? or any key closes ");
    list(buf, decks, &title, colors[focus as usize % 2], &rows(glossary::DECK_KEYS, focus as usize), 2);
    if lower.height < 3 {
        return;
    }
    let (mixer, global) = if lower.width >= 100 {
        let left = lower.width * 3 / 5;
        (Rect::new(lower.x, lower.y, left, lower.height), Rect::new(lower.x + left, lower.y, lower.width - left, lower.height))
    } else {
        // a narrow screen: the mixer keys on top, the global ones below
        let top = lower.height / 2;
        (Rect::new(lower.x, lower.y, lower.width, top), Rect::new(lower.x, lower.y + top, lower.width, lower.height - top))
    };
    list(buf, mixer, " MIXER keys (the focused deck's, or pick with f) ", Color::White, &rows(glossary::MIXER_KEYS, focus as usize), 1);
    list(buf, global, " everywhere ", Color::Yellow, &rows(glossary::GLOBAL_KEYS, focus as usize), 1);
}
