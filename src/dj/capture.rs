//! Capturing cues, loops and grids into the active set, and recording a mix: the command
//! parsing, the deck choice (the focused deck, unless one is named; `pick_deck`, the deck on
//! air, is for `:mix new`) and the texts. The calls go through `SetCall` like the other set
//! commands; codjd writes the file (see "Editing from codj" in `docs/set-language.md`).

use super::Command;
use crate::dj_types::{DeckInfo, MixerInfo};

/// `a` / `b` as a deck index
fn deck_word(w: &str) -> Option<u8> {
    match w {
        "a" | "A" => Some(0),
        "b" | "B" => Some(1),
        _ => None,
    }
}

/// Names are set-language identifiers: `[A-Za-z_][A-Za-z0-9_-]*`
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `<name> [a|b]` of `:cue` and `:loopsave`
fn name_and_deck(words: &[&str], usage: &str) -> Result<(String, Option<u8>), Command> {
    let (name, deck) = match words {
        [name] => (*name, None),
        [name, d] => match deck_word(d) {
            Some(d) => (*name, Some(d)),
            None => return Err(Command::Unknown(usage.into())),
        },
        _ => return Err(Command::Unknown(usage.into())),
    };
    if !valid_name(name) {
        return Err(Command::Unknown(format!(
            "bad name {name:?}: letters, digits, _ and - only, starting with a letter ({usage})"
        )));
    }
    Ok((name.to_string(), deck))
}

/// The capture and record commands, None for any other word
pub fn parse(word: &str, args: &[&str]) -> Option<Command> {
    const CUE: &str = "usage: cue <name> [a|b]";
    const LOOP: &str = "usage: loopsave <name> [a|b]";
    Some(match word {
        "cue" => name_and_deck(args, CUE).map_or_else(|c| c, |(n, d)| Command::Cue(n, d)),
        "loopsave" => name_and_deck(args, LOOP).map_or_else(|c| c, |(n, d)| Command::LoopSave(n, d)),
        "gridsave" => match args {
            [] => Command::GridSave(None),
            [d] => deck_word(d).map_or_else(|| Command::Unknown("usage: gridsave [a|b]".into()), |d| Command::GridSave(Some(d))),
            _ => Command::Unknown("usage: gridsave [a|b]".into()),
        },
        "rec" | "record" => match args {
            [] | ["toggle"] => Command::Record(None),
            ["on" | "start" | "1"] => Command::Record(Some(true)),
            ["off" | "stop" | "0"] => Command::Record(Some(false)),
            _ => Command::Unknown("usage: rec [on|off]".into()),
        },
        _ => return None,
    })
}

/// How loud a deck is in the room, roughly: its fader times the crossfader's share
/// (-1 = all deck A, +1 = all deck B, linear)
fn audible(deck: usize, d: &DeckInfo, crossfader: f64) -> f64 {
    let x = crossfader.clamp(-1.0, 1.0);
    let share = if deck == 0 { 1.0 - x.max(0.0) } else { 1.0 + x.min(0.0) };
    d.fader * share
}

/// The deck that is on the air (`:mix new` mixes out of it): the one that is playing; when both play, the one
/// that is louder (fader times crossfader); on a tie, the one on the crossfader's side (A when
/// centered). Neither playing: the only loaded deck, else the caller must say a or b.
pub fn pick_deck(decks: &[DeckInfo], mixer: &MixerInfo) -> Result<u32, String> {
    let get = |i: usize| decks.get(i).filter(|d| d.loaded);
    let playing: Vec<usize> = (0..2).filter(|&i| get(i).is_some_and(|d| d.playing)).collect();
    match playing.as_slice() {
        [one] => Ok(*one as u32),
        [_, _] => {
            let (a, b) = (audible(0, &decks[0], mixer.crossfader), audible(1, &decks[1], mixer.crossfader));
            if (a - b).abs() > 1e-6 {
                Ok(u32::from(b > a))
            } else {
                Ok(u32::from(mixer.crossfader > 0.0))
            }
        }
        _ => match (get(0), get(1)) {
            (Some(_), None) => Ok(0),
            (None, Some(_)) => Ok(1),
            (None, None) => Err("no track on a deck: load one first".into()),
            _ => Err("no deck is playing: say which one (a or b)".into()),
        },
    }
}

/// The reply of codjd after `ok `, shown to the user: arrows as arrows
pub fn pretty_reply(detail: &str) -> String {
    detail.trim().replace(" -> ", " → ")
}

/// Recording state for the strip: `● REC 12.5 bars · 34 events`
pub fn rec_text(beats: f64, events: u32, short: bool) -> String {
    let bars = beats.max(0.0) / 4.0;
    if short {
        format!("● REC {bars:.1} · {events} ev")
    } else {
        format!("● REC {bars:.1} bars · {events} event{}", if events == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck(loaded: bool, playing: bool, fader: f64) -> DeckInfo {
        DeckInfo { loaded, playing, fader, ..DeckInfo::default() }
    }

    fn xf(x: f64) -> MixerInfo {
        MixerInfo { crossfader: x, ..MixerInfo::default() }
    }

    #[test]
    fn parses_capture_commands() {
        let p = |s: &str| {
            let mut w = s.split_whitespace();
            let first = w.next().unwrap();
            parse(first, &w.collect::<Vec<_>>())
        };
        assert_eq!(p("cue drop"), Some(Command::Cue("drop".into(), None)));
        assert_eq!(p("cue drop b"), Some(Command::Cue("drop".into(), Some(1))));
        assert_eq!(p("cue outro-2 A"), Some(Command::Cue("outro-2".into(), Some(0))));
        assert_eq!(p("loopsave break a"), Some(Command::LoopSave("break".into(), Some(0))));
        assert_eq!(p("gridsave"), Some(Command::GridSave(None)));
        assert_eq!(p("gridsave b"), Some(Command::GridSave(Some(1))));
        assert_eq!(p("rec"), Some(Command::Record(None)));
        assert_eq!(p("rec on"), Some(Command::Record(Some(true))));
        assert_eq!(p("rec off"), Some(Command::Record(Some(false))));
        assert_eq!(p("load a"), None);
        for bad in ["cue", "cue drop c", "cue 1drop", "cue a b c", "loopsave", "gridsave x", "rec maybe", "cue \"x\""] {
            assert!(matches!(p(bad), Some(Command::Unknown(_))), "{bad}");
        }
        // a lone a or b is the name
        assert_eq!(p("cue a"), Some(Command::Cue("a".into(), None)));
    }

    #[test]
    fn deck_choice_rule() {
        let d = |a: DeckInfo, b: DeckInfo| vec![a, b];
        // one playing
        assert_eq!(pick_deck(&d(deck(true, false, 1.), deck(true, true, 1.)), &xf(0.)), Ok(1));
        assert_eq!(pick_deck(&d(deck(true, true, 1.), deck(true, false, 1.)), &xf(1.)), Ok(0));
        // both playing: the louder one, whatever the crossfader side would say on a tie
        assert_eq!(pick_deck(&d(deck(true, true, 0.2), deck(true, true, 1.)), &xf(0.)), Ok(1));
        assert_eq!(pick_deck(&d(deck(true, true, 1.), deck(true, true, 1.)), &xf(0.6)), Ok(1));
        assert_eq!(pick_deck(&d(deck(true, true, 1.), deck(true, true, 1.)), &xf(-0.6)), Ok(0));
        // crossfader cuts A away although its fader is up
        assert_eq!(pick_deck(&d(deck(true, true, 1.), deck(true, true, 0.5)), &xf(1.)), Ok(1));
        // exact tie, centered: A
        assert_eq!(pick_deck(&d(deck(true, true, 1.), deck(true, true, 1.)), &xf(0.)), Ok(0));
        // none playing
        assert_eq!(pick_deck(&d(deck(true, false, 1.), deck(false, false, 1.)), &xf(0.)), Ok(0));
        assert_eq!(pick_deck(&d(deck(false, false, 1.), deck(true, false, 1.)), &xf(0.)), Ok(1));
        assert!(pick_deck(&d(deck(true, false, 1.), deck(true, false, 1.)), &xf(0.)).unwrap_err().contains("a or b"));
        assert!(pick_deck(&d(deck(false, false, 1.), deck(false, false, 1.)), &xf(0.)).is_err());
        // a playing deck without a track doesn't count
        assert_eq!(pick_deck(&d(deck(false, true, 1.), deck(true, true, 1.)), &xf(0.)), Ok(1));
    }

    #[test]
    fn texts() {
        assert_eq!(pretty_reply("mix 2: aft -> smk, 16 bars, 9 items"), "mix 2: aft → smk, 16 bars, 9 items");
        assert_eq!(rec_text(50.0, 34, false), "● REC 12.5 bars · 34 events");
        assert_eq!(rec_text(0.0, 1, false), "● REC 0.0 bars · 1 event");
        assert_eq!(rec_text(50.0, 34, true), "● REC 12.5 · 34 ev");
    }
}
