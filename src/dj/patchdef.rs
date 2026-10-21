//! What codj knows about patches without asking codjd: the node catalog (for agents building a
//! patch, `patch_node_types`) and the three starter templates of `:patch new`. The catalog
//! mirrors the "Nodes (v1)" table of `docs/patches.md` in the codjd repo. Pure data and tests.

use serde_json::{json, Value};

/// codjd has this many patch slots, P1..P4
pub const SLOTS: usize = 4;

/// One node type: (type, inputs as (name, default), outputs, params as (name, default), note)
pub struct NodeType {
    pub name: &'static str,
    pub inputs: &'static [(&'static str, f64)],
    pub outputs: &'static [&'static str],
    pub params: &'static [(&'static str, &'static str)],
    pub note: &'static str,
}

const fn nt(
    name: &'static str,
    inputs: &'static [(&'static str, f64)],
    outputs: &'static [&'static str],
    params: &'static [(&'static str, &'static str)],
    note: &'static str,
) -> NodeType {
    NodeType { name, inputs, outputs, params, note }
}

const HIT: &[(&str, f64)] = &[("trig", 0.0), ("level", 1.0), ("tune", 0.0), ("decay", 0.0)];

pub const CATALOG: &[NodeType] = &[
    nt("Const", &[], &["out"], &[("value", "number")], "a constant"),
    nt("Knob", &[], &["out"], &[("min", "number"), ("max", "number"), ("value", "number")], "exposed control; needs a unique `name`"),
    nt("Add", &[("in0", 0.0), ("in1", 0.0)], &["out"], &[], "in0 + in1"),
    nt("Sub", &[("in0", 0.0), ("in1", 0.0)], &["out"], &[], "in0 - in1"),
    nt("Mul", &[("in0", 0.0), ("in1", 0.0)], &["out"], &[], "in0 * in1 (apply an envelope or a gain)"),
    nt("Div", &[("in0", 0.0), ("in1", 0.0)], &["out"], &[], "in0 / in1, division by 0 gives 0"),
    nt("Mod", &[("in0", 0.0), ("in1", 0.0)], &["out"], &[], "in0 modulo in1"),
    nt("Equal", &[("in0", 0.0), ("in1", 0.0)], &["out"], &[], "1 when equal, else 0"),
    nt("Greater", &[("in0", 0.0), ("in1", 0.0)], &["out"], &[], "1 when in0 > in1, else 0"),
    nt("Nop", &[("in", 0.0)], &["out"], &[], "pass-through"),
    nt("Sine", &[("freq", 440.0), ("sync", 0.0)], &["out"], &[("min", "number (-1)"), ("max", "number (1)")], "sync resets the phase on a rising edge"),
    nt("Saw", &[("freq", 440.0)], &["out"], &[("min", "number"), ("max", "number")], "band-limited sawtooth"),
    nt("Tri", &[("freq", 440.0)], &["out"], &[("min", "number"), ("max", "number")], "band-limited triangle"),
    nt("Pulse", &[("freq", 440.0), ("pw", 0.5)], &["out"], &[("min", "number"), ("max", "number")], "band-limited pulse"),
    nt("Noise", &[], &["out"], &[("min", "number"), ("max", "number")], "white noise"),
    nt("ADSR", &[("gate", 0.0), ("att", 0.02), ("dec", 0.1), ("sus", 0.2), ("rel", 0.1)], &["out"], &[], "envelope 0..1, times in seconds"),
    nt("Filter", &[("in", 0.0), ("cutoff", 1.0), ("reso", 0.0)], &["out"], &[("mode", "lp|hp|bp (lp)")], "2-pole filter; cutoff 0..1 maps 20 Hz to 20 kHz exponentially"),
    nt("Delay", &[("in", 0.0), ("time", 0.25)], &["out"], &[("max", "seconds (2)")], "time in seconds; also breaks feedback cycles"),
    nt("Distort", &[("in", 0.0), ("amt", 0.0)], &["out"], &[], "tanh overdrive"),
    nt("Slide", &[("in", 0.0), ("rate", 1.0)], &["out"], &[], "slew limiter"),
    nt("Hold", &[("in", 0.0), ("trig", 0.0)], &["out"], &[], "sample and hold; breaks feedback cycles"),
    nt(
        "Clock",
        &[],
        &["out", "beat", "bar"],
        &[("source", "master|a|b|free (master)"), ("bpm", "number (free only)"), ("steps_per_beat", "number (4)"), ("swing", "0..0.75")],
        "out = gate high for the first half of each step, locked to the deck's beat grid; beat = 0..1 phase; bar = beat in bar",
    ),
    nt("ClockDiv", &[("clock", 0.0)], &["out"], &[("factor", "number (2)")], "divides gate pulses"),
    nt(
        "MonoSeq",
        &[("clock", 0.0), ("gateT", 0.5)],
        &["freq", "gate"],
        &[("scale", "name, e.g. A minor"), ("root", "MIDI note"), ("steps", "16"), ("rows", "number"), ("pattern", "0..7"), ("patterns", "[{steps, cells:[[step,row]]}] x8")],
        "one note per step; the row is a scale degree; advances on the clock's rising edge",
    ),
    nt(
        "GateSeq",
        &[("clock", 0.0), ("gateT", 0.5)],
        &["gate0", "gate1", "gate2", "gate3", "gate4", "gate5", "gate6", "gate7"],
        &[("steps", "16"), ("rows", "up to 8"), ("row_names", "[\"kick\",...]"), ("pattern", "0..7"), ("patterns", "[{steps, cells:[[step,row,velocity?]]}] x8")],
        "drum style: one gate output per row",
    ),
    nt("Kick", HIT, &["out"], &[], "synthesized one-shot, retriggers on a rising edge of trig"),
    nt("Snare", HIT, &["out"], &[], "synthesized one-shot"),
    nt("Clap", HIT, &["out"], &[], "synthesized one-shot"),
    nt("HatClosed", HIT, &["out"], &[], "synthesized one-shot"),
    nt("HatOpen", HIT, &["out"], &[], "synthesized one-shot"),
    nt("Rim", HIT, &["out"], &[], "synthesized one-shot"),
    nt("Tom", HIT, &["out"], &[], "synthesized one-shot"),
    nt("Pan", &[("in", 0.0), ("pos", 0.0)], &["left", "right"], &[], "equal-power pan, pos -1..1"),
    nt(
        "Mix4",
        &[("in0", 0.0), ("in1", 0.0), ("in2", 0.0), ("in3", 0.0), ("g0", 1.0), ("g1", 1.0), ("g2", 1.0), ("g3", 1.0)],
        &["out"],
        &[],
        "sums four inputs with a gain each",
    ),
    nt("AudioOut", &[("left", 0.0), ("right", 0.0)], &[], &[], "the slot's output; exactly one per patch"),
    nt("Notes", &[], &[], &[("text", "string")], "a comment, ignored"),
];

/// (used by the fake daemon and the tests; codjd is the real judge of a patch)
#[allow(dead_code)]
pub fn is_known_type(t: &str) -> bool {
    CATALOG.iter().any(|n| n.name == t)
}

/// The catalog as JSON for `patch_node_types`, with the file format rules agents need
pub fn catalog_json() -> Value {
    let nodes: Vec<Value> = CATALOG
        .iter()
        .map(|n| {
            let inputs: serde_json::Map<String, Value> = n.inputs.iter().map(|(k, d)| (k.to_string(), json!(d))).collect();
            let params: serde_json::Map<String, Value> = n.params.iter().map(|(k, d)| (k.to_string(), json!(d))).collect();
            json!({"type": n.name, "inputs": inputs, "outputs": n.outputs, "params": params, "note": n.note})
        })
        .collect();
    json!({
        "format": {
            "file": "{\"codj_patch\":1,\"title\":\"...\",\"nodes\":{\"<id>\":{\"type\":\"Saw\",\"name\":\"osc\",\"ins\":{\"freq\":[\"2\",\"freq\"]},\"params\":{}}}}",
            "ins": "an input name maps to [source node id, output name] or to a constant number; an unconnected input takes the default listed under `inputs`",
            "ids": "node ids are strings, unique per patch; `name` is optional except for Knob (unique per patch) and sequencers",
            "rules": [
                "exactly one AudioOut node",
                "signals are f32: frequency in Hz, gates 0/1, levels -1..1; there is no audio/control split",
                "a feedback cycle must pass through a Delay or Hold",
                "sequencer patterns: 8 per node (A-H = index 0-7); cells are [step,row] for MonoSeq and [step,row,velocity] for GateSeq"
            ],
        },
        "nodes": nodes,
    })
}

/// The starter patches of `:patch new <name> [drums|acid|pad]`
pub const TEMPLATE_KINDS: [&str; 3] = ["drums", "acid", "pad"];

/// A starter patch as JSON text with `title` filled in; None for an unknown kind
pub fn template(kind: &str, title: &str) -> Option<String> {
    let mut v: Value = match kind {
        "drums" => json!({
            "codj_patch": 1,
            "nodes": {
                "1": { "type": "Clock", "name": "clock", "params": { "source": "master", "steps_per_beat": 4 } },
                "2": { "type": "GateSeq", "name": "drums", "ins": { "clock": ["1", "out"] },
                       "params": { "steps": 16, "rows": 4, "row_names": ["kick", "snare", "hat", "clap"], "pattern": 0,
                                   "patterns": [ { "steps": 16, "cells": [
                                       [0, 0, 1.0], [4, 0, 1.0], [8, 0, 1.0], [12, 0, 1.0],
                                       [4, 1, 0.9], [12, 1, 0.9],
                                       [2, 2, 0.6], [6, 2, 0.6], [10, 2, 0.6], [14, 2, 0.6] ] } ] } },
                "3": { "type": "Kick", "name": "kick", "ins": { "trig": ["2", "gate0"] } },
                "4": { "type": "Snare", "name": "snare", "ins": { "trig": ["2", "gate1"] } },
                "5": { "type": "HatClosed", "name": "hat", "ins": { "trig": ["2", "gate2"] } },
                "6": { "type": "Clap", "name": "clap", "ins": { "trig": ["2", "gate3"] } },
                "7": { "type": "Knob", "name": "kick_level", "params": { "min": 0.0, "max": 1.0, "value": 0.9 } },
                "8": { "type": "Knob", "name": "hat_level", "params": { "min": 0.0, "max": 1.0, "value": 0.5 } },
                "9": { "type": "Mix4", "name": "mix",
                       "ins": { "in0": ["3", "out"], "in1": ["4", "out"], "in2": ["5", "out"], "in3": ["6", "out"],
                                "g0": ["7", "out"], "g1": 0.8, "g2": ["8", "out"], "g3": 0.6 } },
                "10": { "type": "AudioOut", "ins": { "left": ["9", "out"], "right": ["9", "out"] } }
            }
        }),
        "acid" => json!({
            "codj_patch": 1,
            "nodes": {
                "1": { "type": "Clock", "name": "clock", "params": { "source": "master", "steps_per_beat": 4 } },
                "2": { "type": "MonoSeq", "name": "seq", "ins": { "clock": ["1", "out"] },
                       "params": { "scale": "A minor", "root": 45, "steps": 16, "rows": 12, "pattern": 0,
                                   "patterns": [ { "steps": 16, "cells": [[0, 3], [2, 3], [4, 7], [6, 3], [8, 3], [10, 5], [12, 7], [14, 2]] } ] } },
                "3": { "type": "Saw", "name": "osc", "ins": { "freq": ["2", "freq"] } },
                "4": { "type": "ADSR", "name": "env", "ins": { "gate": ["2", "gate"] },
                       "params": { "att": 0.005, "dec": 0.15, "sus": 0.0, "rel": 0.05 } },
                "5": { "type": "Knob", "name": "cutoff", "params": { "min": 0.0, "max": 1.0, "value": 0.3 } },
                "6": { "type": "Knob", "name": "reso", "params": { "min": 0.0, "max": 0.95, "value": 0.6 } },
                "7": { "type": "Filter", "name": "lp", "ins": { "in": ["3", "out"], "cutoff": ["5", "out"], "reso": ["6", "out"] } },
                "8": { "type": "Mul", "ins": { "in0": ["7", "out"], "in1": ["4", "out"] } },
                "9": { "type": "AudioOut", "ins": { "left": ["8", "out"], "right": ["8", "out"] } }
            }
        }),
        "pad" => json!({
            "codj_patch": 1,
            "nodes": {
                "1": { "type": "Clock", "name": "clock", "params": { "source": "master", "steps_per_beat": 1 } },
                "2": { "type": "MonoSeq", "name": "chords", "ins": { "clock": ["1", "out"] },
                       "params": { "scale": "A minor", "root": 57, "steps": 8, "rows": 8, "pattern": 0,
                                   "patterns": [ { "steps": 8, "cells": [[0, 0], [2, 2], [4, 4], [6, 3]] } ] } },
                "3": { "type": "Saw", "name": "osc1", "ins": { "freq": ["2", "freq"] } },
                "4": { "type": "Mul", "name": "detune", "ins": { "in0": ["2", "freq"], "in1": 1.007 } },
                "5": { "type": "Saw", "name": "osc2", "ins": { "freq": ["4", "out"] } },
                "6": { "type": "Add", "ins": { "in0": ["3", "out"], "in1": ["5", "out"] } },
                "7": { "type": "ADSR", "name": "env", "ins": { "gate": ["2", "gate"] },
                       "params": { "att": 0.4, "dec": 0.5, "sus": 0.7, "rel": 1.2 } },
                "8": { "type": "Knob", "name": "cutoff", "params": { "min": 0.0, "max": 1.0, "value": 0.45 } },
                "9": { "type": "Filter", "name": "lp", "ins": { "in": ["6", "out"], "cutoff": ["8", "out"], "reso": 0.2 } },
                "10": { "type": "Mul", "name": "vca", "ins": { "in0": ["9", "out"], "in1": ["7", "out"] } },
                "11": { "type": "Mul", "name": "level", "ins": { "in0": ["10", "out"], "in1": 0.3 } },
                "12": { "type": "AudioOut", "ins": { "left": ["11", "out"], "right": ["11", "out"] } }
            }
        }),
        _ => return None,
    };
    v["title"] = json!(title);
    // keys in a stable, readable order: format version and title first
    let nodes = v["nodes"].take();
    let ordered = json!({"codj_patch": 1, "title": title, "nodes": nodes});
    serde_json::to_string_pretty(&ordered).ok().map(|mut s| {
        s.push('\n');
        s
    })
}

/// `name` as a patch file name: letters, digits, `-`, `_`; no path tricks
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_is_a_valid_patch() {
        for kind in TEMPLATE_KINDS {
            let text = template(kind, "t").unwrap_or_else(|| panic!("{kind}"));
            let v: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(v["codj_patch"], 1);
            assert_eq!(v["title"], "t");
            let nodes = v["nodes"].as_object().unwrap();
            let audio_out = nodes.values().filter(|n| n["type"] == "AudioOut").count();
            assert_eq!(audio_out, 1, "{kind}: exactly one AudioOut");
            let mut names = std::collections::HashSet::new();
            for (id, n) in nodes {
                let ty = n["type"].as_str().unwrap();
                let def = CATALOG.iter().find(|c| c.name == ty).unwrap_or_else(|| panic!("{kind}: type {ty}"));
                if ty == "Knob" {
                    assert!(names.insert(n["name"].as_str().unwrap().to_string()), "{kind}: knob names are unique");
                }
                for (input, src) in n["ins"].as_object().into_iter().flatten() {
                    assert!(def.inputs.iter().any(|(i, _)| i == input), "{kind}/{id}: {ty} has no input {input}");
                    if let Some(pair) = src.as_array() {
                        let (from, out) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
                        let from_ty = nodes.get(from).unwrap_or_else(|| panic!("{kind}/{id}: no node {from}"))["type"].as_str().unwrap();
                        let from_def = CATALOG.iter().find(|c| c.name == from_ty).unwrap();
                        assert!(from_def.outputs.contains(&out), "{kind}/{id}: {from_ty} has no output {out}");
                    } else {
                        assert!(src.is_number(), "{kind}/{id}.{input}");
                    }
                }
            }
        }
        assert!(template("polka", "t").is_none());
    }

    #[test]
    fn sequencer_cells_fit_their_patterns() {
        for kind in TEMPLATE_KINDS {
            let v: Value = serde_json::from_str(&template(kind, "t").unwrap()).unwrap();
            for n in v["nodes"].as_object().unwrap().values() {
                if !matches!(n["type"].as_str(), Some("MonoSeq" | "GateSeq")) {
                    continue;
                }
                let steps = n["params"]["steps"].as_u64().unwrap();
                let rows = n["params"]["rows"].as_u64().unwrap();
                for p in n["params"]["patterns"].as_array().unwrap() {
                    for c in p["cells"].as_array().unwrap() {
                        assert!(c[0].as_u64().unwrap() < steps && c[1].as_u64().unwrap() < rows, "{kind}: {c}");
                    }
                }
            }
        }
    }

    #[test]
    fn catalog_covers_the_contract_and_serializes() {
        for t in ["Const", "Knob", "Clock", "MonoSeq", "GateSeq", "Kick", "Mix4", "AudioOut", "Filter", "ADSR"] {
            assert!(is_known_type(t), "{t}");
        }
        let names: std::collections::HashSet<_> = CATALOG.iter().map(|n| n.name).collect();
        assert_eq!(names.len(), CATALOG.len(), "no type twice");
        let v = catalog_json();
        let nodes = v["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), CATALOG.len());
        let gate = nodes.iter().find(|n| n["type"] == "GateSeq").unwrap();
        assert_eq!(gate["outputs"].as_array().unwrap().len(), 8);
        let filter = nodes.iter().find(|n| n["type"] == "Filter").unwrap();
        assert_eq!(filter["inputs"]["cutoff"], 1.0);
        assert!(v["format"]["rules"].as_array().unwrap().iter().any(|r| r.as_str().unwrap().contains("AudioOut")));
    }

    #[test]
    fn names_are_file_safe() {
        assert!(valid_name("d1") && valid_name("acid-line_2"));
        assert!(!valid_name("") && !valid_name("../x") && !valid_name("a b") && !valid_name("a/b"));
    }
}
