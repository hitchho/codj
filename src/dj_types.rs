//! Wire types of codjd's deck API, see `docs/dj-api.md` in the codjd repo. The field order is the
//! D-Bus signature, so it must match the daemon's structs exactly. This file has no dependency on
//! the rest of the crate so that `examples/fake_codjd.rs` can include it.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

/// One deck's state, polled at ~30 Hz. D-Bus signature `(bsssddbddddubddddadsdd(ddd)ddb)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct DeckInfo {
    pub loaded: bool,
    pub path: String,
    /// from tags, or the file name without extension
    pub title: String,
    pub artist: String,
    /// seconds
    pub position: f64,
    /// seconds (0 while unknown)
    pub duration: f64,
    pub playing: bool,
    /// playback speed, 1.0 = original
    pub rate: f64,
    /// effective tempo now (track bpm x rate x nudge), 0 = no grid
    pub bpm: f64,
    /// the grid's tempo, 0 = no grid
    pub track_bpm: f64,
    /// 0..1 position within the current beat, -1 = no grid
    pub beat_phase: f64,
    /// 1..=beats_per_bar, 0 = no grid
    pub beat_in_bar: u32,
    pub synced: bool,
    /// 0 = no loop
    pub loop_beats: f64,
    /// seconds, when looping
    pub loop_start: f64,
    pub loop_end: f64,
    /// seconds
    pub cue_point: f64,
    /// 8 entries, seconds, -1 = empty
    pub hotcues: Vec<f64>,
    /// Camelot, "" = unknown
    pub key: String,
    /// applied auto-gain
    pub gain_db: f64,
    /// seconds decoded into RAM so far
    pub decoded: f64,
    /// low, mid, high, -1..1
    pub eq: (f64, f64, f64),
    /// -1..1
    pub filter: f64,
    /// 0..1
    pub fader: f64,
    /// cue send on
    pub headphones: bool,
}

impl DeckInfo {
    /// the deck has a beat grid (tempo and phase are meaningful)
    pub fn has_grid(&self) -> bool {
        self.beat_in_bar > 0 && self.track_bpm > 0.0
    }

    /// seconds of hot cue `slot`, if set
    pub fn hotcue(&self, slot: usize) -> Option<f64> {
        self.hotcues.get(slot).copied().filter(|t| *t >= 0.0)
    }
}

/// Mixer-wide state. D-Bus signature `(dddsbt)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct MixerInfo {
    /// -1 A .. 1 B
    pub crossfader: f64,
    pub master: f64,
    pub cue_mix: f64,
    /// "internal" | "external"
    pub mode: String,
    pub quantize: bool,
    pub xruns: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_match_the_contract() {
        assert_eq!(
            <DeckInfo as Type>::SIGNATURE.to_string(),
            "(bsssddbddddubddddadsdd(ddd)ddb)"
        );
        assert_eq!(<MixerInfo as Type>::SIGNATURE.to_string(), "(dddsbt)");
    }
}
