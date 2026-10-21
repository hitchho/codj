//! The `?` overlay: the key map drawn on the actual screen, which-key style. Each deck panel
//! lists its own keys, the area of the mixer and library lists the mixer's and the global keys,
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

/// Draws a box with `rows` (keys, text) lines; what does not fit ends in a "… N more" line
fn list(buf: &mut Buffer, area: Rect, title: &str, color: Color, rows: &[(String, String)], footer: &str) {
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
    let width = inner.width.saturating_sub(2) as usize;
    let keys = Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD);
    let kw = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0).min(width / 3);
    let more = rows.len() > room;
    let shown = if more { room.saturating_sub(1) } else { room };
    for (i, (k, text)) in rows.iter().take(shown).enumerate() {
        let y = inner.y + i as u16;
        buf.set_stringn(inner.x + 1, y, format!("{:<kw$}", truncated(k, kw)), kw, keys);
        buf.set_stringn(inner.x + 2 + kw as u16, y, truncated(text, width.saturating_sub(kw + 1)), width.saturating_sub(kw + 1), Style::default());
    }
    if more && room > 0 {
        let y = inner.y + room as u16 - 1;
        let note = format!("… {} more: :help keys", rows.len() - shown);
        buf.set_stringn(inner.x + 1, y, note, width, Style::default().fg(Color::DarkGray));
    } else if !footer.is_empty() && rows.len() < room {
        let y = inner.bottom() - 1;
        buf.set_stringn(inner.x + 1, y, footer, width, Style::default().fg(Color::DarkGray));
    }
}

fn rows(entries: &[Entry], deck: Option<usize>) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|e| match deck {
            Some(d) => {
                let (k, t) = glossary::deck_entry(e, d);
                (k.to_string(), t)
            }
            None => (e.a.to_string(), e.text.to_string()),
        })
        .collect()
}

/// `decks`: the two deck rectangles, `lower`: the mixer and the library together. The mixer keys
/// and the global keys share `lower` side by side, or one over the other when it is narrow.
pub fn draw(buf: &mut Buffer, decks: [Rect; 2], lower: Rect) {
    let colors = [Color::Cyan, Color::Magenta];
    for (i, area) in decks.into_iter().enumerate() {
        let name = if i == 0 { "A" } else { "B" };
        let title = format!(" DECK {name} keys · ? or any key closes ");
        list(buf, area, &title, colors[i], &rows(glossary::DECK_KEYS, Some(i)), "");
    }
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
    list(buf, mixer, " MIXER keys (deck A · deck B) ", Color::White, &rows(glossary::MIXER_KEYS, None), "");
    list(buf, global, " everywhere ", Color::Yellow, &rows(glossary::GLOBAL_KEYS, None), "");
}
