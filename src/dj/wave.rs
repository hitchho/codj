//! Waveform overview from codjd's `GetWaveform`, and drawing it into a ratatui buffer.
//!
//! Wire format: u32 LE column count n, u32 LE duration in ms, then n columns of 6 bytes
//! `[peak_low, peak_mid, peak_high, rms_low, rms_mid, rms_high]`. Amplitudes are companded
//! (linear = (byte / 255)^2), so the byte itself is already a good display scale.

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

#[derive(Debug, Clone, PartialEq)]
pub struct Waveform {
    pub duration_ms: u32,
    pub cols: Vec<[u8; 6]>,
}

/// One drawn column: several waveform columns merged
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub peak: [u8; 3],
    pub rms: [u8; 3],
}

impl Waveform {
    /// Parses the `GetWaveform` reply; `None` for an empty or damaged one
    pub fn decode(bytes: &[u8]) -> Option<Waveform> {
        if bytes.len() < 8 {
            return None;
        }
        let n = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
        let duration_ms = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
        let body = &bytes[8..];
        if n == 0 || body.len() != n.checked_mul(6)? {
            return None;
        }
        let cols = body
            .chunks_exact(6)
            .map(|c| [c[0], c[1], c[2], c[3], c[4], c[5]])
            .collect();
        Some(Waveform { duration_ms, cols })
    }

    pub fn duration(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }

    fn cols_per_sec(&self) -> f64 {
        if self.duration_ms == 0 {
            0.0
        } else {
            self.cols.len() as f64 / self.duration()
        }
    }

    /// Peak (max) and rms (mean) over the time range `[t0, t1)` seconds; `None` outside the track
    pub fn bar(&self, t0: f64, t1: f64) -> Option<Bar> {
        let cps = self.cols_per_sec();
        if cps <= 0.0 || t1 <= 0.0 || t0 >= self.duration() {
            return None;
        }
        let n = self.cols.len();
        let i0 = ((t0.max(0.0) * cps).floor() as usize).min(n - 1);
        let i1 = ((t1 * cps).ceil() as usize).clamp(i0 + 1, n);
        let mut peak = [0u8; 3];
        let mut rms = [0u32; 3];
        for c in &self.cols[i0..i1] {
            for b in 0..3 {
                peak[b] = peak[b].max(c[b]);
                rms[b] += c[3 + b] as u32;
            }
        }
        let count = (i1 - i0) as u32;
        Some(Bar {
            peak,
            rms: [(rms[0] / count) as u8, (rms[1] / count) as u8, (rms[2] / count) as u8],
        })
    }
}

impl Bar {
    /// 0..1 height of the peak
    pub fn height(&self) -> f64 {
        *self.peak.iter().max().unwrap_or(&0) as f64 / 255.0
    }

    pub fn rms_height(&self) -> f64 {
        *self.rms.iter().max().unwrap_or(&0) as f64 / 255.0
    }

    /// Colour by band energy: low red/orange, mid green, high blue, mixed by their share
    pub fn color(&self) -> Color {
        const LOW: [f64; 3] = [255.0, 70.0, 50.0];
        const MID: [f64; 3] = [70.0, 225.0, 100.0];
        const HIGH: [f64; 3] = [70.0, 150.0, 255.0];
        let max = (*self.rms.iter().max().unwrap_or(&0)).max(1) as f64;
        let w: Vec<f64> = self.rms.iter().map(|v| (*v as f64 / max).powi(3)).collect();
        let total: f64 = w.iter().sum::<f64>().max(1e-6);
        let mix = |ch: usize| {
            ((w[0] * LOW[ch] + w[1] * MID[ch] + w[2] * HIGH[ch]) / total).round() as u8
        };
        Color::Rgb(mix(0), mix(1), mix(2))
    }
}

fn dim(c: Color, f: f64) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            Color::Rgb((r as f64 * f) as u8, (g as f64 * f) as u8, (b as f64 * f) as u8)
        }
        c => c,
    }
}

const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Draws the waveform for the time range `[t_left, t_right)` across `area`, one bar per terminal
/// column, bottom anchored with 1/8 row steps. The part up to the rms level is bright, the
/// rest of the peak dim. Columns outside the track stay empty.
pub fn draw_strip(buf: &mut Buffer, area: Rect, wf: &Waveform, t_left: f64, t_right: f64) {
    if area.width == 0 || area.height == 0 || t_right <= t_left {
        return;
    }
    let dt = (t_right - t_left) / area.width as f64;
    let levels = area.height as usize * 8;
    for x in 0..area.width {
        let t = t_left + x as f64 * dt;
        let Some(bar) = wf.bar(t, t + dt) else {
            continue;
        };
        let fill = (bar.height() * levels as f64).round() as usize;
        let rms_fill = (bar.rms_height() * levels as f64).round() as usize;
        let color = bar.color();
        for r in 0..area.height as usize {
            let cell_fill = fill.saturating_sub(r * 8).min(8);
            if cell_fill == 0 {
                break;
            }
            let bright = r * 8 < rms_fill;
            let cell = &mut buf[(area.x + x, area.bottom() - 1 - r as u16)];
            cell.set_char(EIGHTHS[cell_fill]);
            cell.set_fg(if bright { color } else { dim(color, 0.45) });
        }
    }
}

/// Terminal column (relative to the area) of time `t` in a strip covering `[t_left, t_right)`
pub fn time_to_col(t: f64, t_left: f64, t_right: f64, width: u16) -> Option<u16> {
    if t < t_left || t >= t_right || width == 0 {
        return None;
    }
    Some((((t - t_left) / (t_right - t_left)) * width as f64) as u16).filter(|c| *c < width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(cols: &[[u8; 6]], ms: u32) -> Vec<u8> {
        let mut v = (cols.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(&ms.to_le_bytes());
        for c in cols {
            v.extend_from_slice(c);
        }
        v
    }

    #[test]
    fn decodes_the_wire_format() {
        let bytes = pack(&[[1, 2, 3, 4, 5, 6], [7, 8, 9, 10, 11, 12]], 40);
        let wf = Waveform::decode(&bytes).unwrap();
        assert_eq!(wf.duration_ms, 40);
        assert_eq!(wf.cols, vec![[1, 2, 3, 4, 5, 6], [7, 8, 9, 10, 11, 12]]);
    }

    #[test]
    fn rejects_empty_and_damaged_replies() {
        assert!(Waveform::decode(&[]).is_none());
        assert!(Waveform::decode(&[0; 8]).is_none());
        let mut bytes = pack(&[[0; 6]; 3], 60);
        bytes.pop();
        assert!(Waveform::decode(&bytes).is_none());
        // a count that would overflow the length check
        let mut huge = u32::MAX.to_le_bytes().to_vec();
        huge.extend_from_slice(&[0; 10]);
        assert!(Waveform::decode(&huge).is_none());
    }

    #[test]
    fn bar_merges_columns_and_ends_with_the_track() {
        // 4 columns over 80 ms = 50 per second
        let wf = Waveform {
            duration_ms: 80,
            cols: vec![[10, 0, 0, 10, 0, 0], [200, 0, 0, 100, 0, 0], [0; 6], [0, 0, 255, 0, 0, 255]],
        };
        let b = wf.bar(0.0, 0.04).unwrap();
        assert_eq!(b.peak, [200, 0, 0]);
        assert_eq!(b.rms, [55, 0, 0]);
        assert!(wf.bar(0.0, 1.0).is_some());
        assert!(wf.bar(0.08, 0.2).is_none());
        assert!(wf.bar(-1.0, -0.5).is_none());
        // finer than the data still finds the column
        assert_eq!(wf.bar(0.06, 0.0601).unwrap().peak, [0, 0, 255]);
    }

    #[test]
    fn colours_follow_the_dominant_band() {
        let low = Bar { peak: [200, 0, 0], rms: [200, 0, 0] }.color();
        let high = Bar { peak: [0, 0, 200], rms: [0, 0, 200] }.color();
        let Color::Rgb(lr, _, lb) = low else { panic!() };
        let Color::Rgb(hr, _, hb) = high else { panic!() };
        assert!(lr > lb);
        assert!(hb > hr);
    }

    #[test]
    fn strip_draws_bottom_anchored_bars() {
        // 10 columns over 1 s: loud first half in the lows, silent second half
        let cols: Vec<[u8; 6]> =
            (0..10).map(|i| if i < 5 { [255, 0, 0, 255, 0, 0] } else { [0; 6] }).collect();
        let wf = Waveform { duration_ms: 1000, cols };
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_strip(&mut buf, area, &wf, 0.0, 1.0);
        for x in 0..5 {
            assert_eq!(buf[(x, 1)].symbol(), "█");
            assert_eq!(buf[(x, 0)].symbol(), "█");
        }
        for x in 5..10 {
            assert_eq!(buf[(x, 1)].symbol(), " ");
        }
        // low band => reddish
        let Color::Rgb(r, _, b) = buf[(0, 1)].fg else { panic!() };
        assert!(r > b);
    }

    #[test]
    fn strip_half_height_uses_eighths_and_dims_the_peak_tail() {
        // peak 1.0 but rms 0.25 over 2 rows: bottom row bright, top row dim
        let wf = Waveform { duration_ms: 20, cols: vec![[255, 0, 0, 64, 0, 0]] };
        let area = Rect::new(0, 0, 1, 2);
        let mut buf = Buffer::empty(area);
        draw_strip(&mut buf, area, &wf, 0.0, 0.02);
        assert_eq!(buf[(0, 0)].symbol(), "█");
        let Color::Rgb(bright, ..) = buf[(0, 1)].fg else { panic!() };
        let Color::Rgb(dimmed, ..) = buf[(0, 0)].fg else { panic!() };
        assert!(bright > dimmed);
    }

    #[test]
    fn strip_before_the_start_is_empty() {
        let wf = Waveform { duration_ms: 1000, cols: vec![[255; 6]; 50] };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        // window is 2 s wide centred on 0: left half is before the track
        draw_strip(&mut buf, area, &wf, -1.0, 1.0);
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(buf[(9, 0)].symbol(), "█");
    }

    #[test]
    fn time_to_col_maps_the_window() {
        assert_eq!(time_to_col(5.0, 0.0, 10.0, 20), Some(10));
        assert_eq!(time_to_col(-0.1, 0.0, 10.0, 20), None);
        assert_eq!(time_to_col(10.0, 0.0, 10.0, 20), None);
    }
}
