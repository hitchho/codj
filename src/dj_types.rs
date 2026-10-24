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
    /// "internal" | "external" | "split"
    pub mode: String,
    pub quantize: bool,
    pub xruns: u64,
}

/// State of the set runtime, see "Set runtime (D-Bus)" in `docs/dj-api.md`. D-Bus signature
/// `(ssua(sss)a(ssdd)ibidba(ds))`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct SetState {
    /// active set, "" = none
    pub name: String,
    pub file: String,
    /// errors in the file (the last good version stays active)
    pub errors: u32,
    /// (id, title, path) in file order
    pub tracks: Vec<(String, String, String)>,
    /// (from id, to id, start beat on from's grid, length in beats)
    pub mixes: Vec<(String, String, f64, f64)>,
    /// armed or running mix, -1 = none
    pub armed: i32,
    /// the armed mix has started
    pub running: bool,
    /// deck playing the outgoing track of the armed mix, -1
    pub out_deck: i32,
    /// beats since the armed mix's start (negative before it), 0 if none
    pub mix_beat: f64,
    pub auto: bool,
    /// upcoming automation of the armed mix: (mix beat, text), at most 8
    pub next_items: Vec<(f64, String)>,
}

impl SetState {
    pub fn is_active(&self) -> bool {
        !self.name.is_empty()
    }

    /// the armed mix as an index, if there is one
    pub fn armed_mix(&self) -> Option<usize> {
        usize::try_from(self.armed).ok().filter(|i| *i < self.mixes.len())
    }

    /// index of the track with this id
    pub fn track_index(&self, id: &str) -> Option<usize> {
        self.tracks.iter().position(|t| t.0 == id)
    }
}

/// One patch slot (P1..P4), see `docs/patches.md` in the codjd repo. D-Bus signature
/// `(bssbbdsa(sddd)a(suuu)db)`; `PatchState` returns an array of these.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct PatchSlot {
    pub loaded: bool,
    pub name: String,
    pub title: String,
    pub dirty: bool,
    /// sent to the master (else headphones only)
    pub live: bool,
    /// share of the audio callback, 0..1
    pub cpu: f64,
    /// "" or e.g. "cpu overrun"
    pub muted_reason: String,
    /// (name, value, min, max)
    pub knobs: Vec<(String, f64, f64, f64)>,
    /// (name, pattern, queued (u32::MAX = none), step)
    pub sequencers: Vec<(String, u32, u32, u32)>,
    pub fader: f64,
    pub headphones: bool,
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
        assert_eq!(<SetState as Type>::SIGNATURE.to_string(), "(ssua(sss)a(ssdd)ibidba(ds))");
        assert_eq!(<PatchSlot as Type>::SIGNATURE.to_string(), "(bssbbdsa(sddd)a(suuu)db)");
        assert_eq!(<Vec<PatchSlot> as Type>::SIGNATURE.to_string(), "a(bssbbdsa(sddd)a(suuu)db)");
    }
}
