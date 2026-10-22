//! `:mix new <recipe> [bars]`: writes a recipe-expanded mix block for the two tracks on the decks
//! into the active set. The recipes and their text come from codjd (`SetRecipes`,
//! `SetRecipeText`); this file is the command parsing, the choice of tracks and the text surgery,
//! all pure.

use super::Command;
use crate::dj_types::{DeckInfo, MixerInfo, SetState};

/// Longest mix a recipe may be asked for
pub const MAX_BARS: u32 = 512;

pub const USAGE: &str = "usage: mix new [<recipe> [bars]] (without a recipe: lists them)";

/// `:mix new [recipe [bars]]` after the word `mix`
pub fn parse(args: &[&str]) -> Command {
    match args {
        ["new"] | ["list"] | ["recipes"] => Command::MixNew { recipe: None, bars: None },
        ["new", recipe] => Command::MixNew { recipe: Some(recipe.to_string()), bars: None },
        ["new", recipe, bars] => match bars.parse::<u32>() {
            Ok(b) if (1..=MAX_BARS).contains(&b) => Command::MixNew { recipe: Some(recipe.to_string()), bars: Some(b) },
            _ => Command::Unknown(format!("bars must be a whole number from 1 to {MAX_BARS} ({USAGE})")),
        },
        _ => Command::Unknown(USAGE.into()),
    }
}

/// What a `:mix new` works on
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    /// the deck that is on air
    pub out_deck: u32,
    pub from_id: String,
    pub to_id: String,
    pub from_title: String,
    pub to_title: String,
}

pub const NOT_IN_SET: &str = "add the track to the set first: :cue on it or edit";

/// The outgoing track is the playing deck on air (`capture::pick_deck`), the incoming one is the
/// other deck; both must be tracks of the active set
pub fn pick_pair(decks: &[DeckInfo], mixer: &MixerInfo, set: &SetState) -> Result<Pair, String> {
    if !set.is_active() {
        return Err("no active set: :set new <name> makes one, :set <name> uses one".into());
    }
    let loaded = |i: usize| decks.get(i).filter(|d| d.loaded);
    let (Some(_), Some(_)) = (loaded(0), loaded(1)) else {
        return Err("load a track on both decks first: the one on air and the one coming in".into());
    };
    let out = super::capture::pick_deck(decks, mixer)? as usize;
    let inc = 1 - out;
    let track_of = |d: &DeckInfo| set.tracks.iter().find(|t| t.2 == d.path);
    let (Some(from), Some(to)) = (track_of(&decks[out]), track_of(&decks[inc])) else {
        let missing: Vec<&str> = [out, inc].into_iter().filter(|i| track_of(&decks[*i]).is_none()).map(|i| decks[i].title.as_str()).collect();
        return Err(format!("{NOT_IN_SET} ({})", missing.join(", ")));
    };
    if from.0 == to.0 {
        return Err("both decks have the same track: a mix needs two".into());
    }
    Ok(Pair { out_deck: out as u32, from_id: from.0.clone(), to_id: to.0.clone(), from_title: from.1.clone(), to_title: to.1.clone() })
}

/// First words of a line, comment removed, `->` always a word of its own
fn words(line: &str) -> Vec<String> {
    let code = line.split('#').next().unwrap_or("");
    code.replace("->", " -> ").split_whitespace().map(str::to_string).collect()
}

fn is_header(w: &[String]) -> bool {
    matches!(w.first().map(String::as_str), Some("track" | "mix" | "set"))
}

/// The header line (0-based) of `mix <from> -> <to>`, and one past the last line of its block
pub fn find_mix(text: &str, from: &str, to: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| {
        let w = words(l);
        w.len() >= 4 && w[0] == "mix" && w[1] == from && w[2] == "->" && w[3] == to
    })?;
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate().skip(start + 1) {
        if is_header(&words(l)) {
            end = i;
            break;
        }
    }
    // the blank lines between this block and the next header stay where they are
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    Some((start, end))
}

/// The outcome of putting a block into a set's text
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub text: String,
    /// line (1-based) of an older `mix from -> to` block that was there already
    pub existing: Option<usize>,
    /// the older block was replaced (else the new one was appended after it)
    pub replaced: bool,
    /// the number (from 1) of the new mix among the set's mix blocks
    pub mix_number: usize,
}

fn mix_count(lines: &[&str]) -> usize {
    lines.iter().filter(|l| words(l).first().is_some_and(|w| w == "mix")).count()
}

/// Appends `block` to `source` (a blank line between), or replaces the older block of the same
/// two tracks when `replace` is set
pub fn merge_block(source: &str, block: &str, from: &str, to: &str, replace: bool) -> Merged {
    let block = block.trim_matches('\n');
    let existing = find_mix(source, from, to);
    let lines: Vec<&str> = source.lines().collect();
    match existing {
        Some((start, end)) if replace => {
            let mut out: Vec<&str> = lines[..start].to_vec();
            out.extend(block.lines());
            out.extend(&lines[end..]);
            let mix_number = mix_count(&lines[..start]) + 1;
            Merged { text: out.join("\n") + "\n", existing: Some(start + 1), replaced: true, mix_number }
        }
        _ => {
            let mut text = source.trim_end_matches('\n').to_string();
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(block);
            text.push('\n');
            Merged { text, existing: existing.map(|(s, _)| s + 1), replaced: false, mix_number: mix_count(&lines) + 1 }
        }
    }
}

/// A recipe reply is a block when it has a `mix` header line
pub fn looks_like_block(text: &str) -> bool {
    text.lines().any(|l| words(l).first().is_some_and(|w| w == "mix"))
}

/// The recipes as popup paragraphs
pub fn recipes_popup(recipes: &[(String, String, u32)]) -> (String, Vec<String>) {
    let mut lines = vec![":mix new <recipe> [bars] writes one for the two tracks on your decks (outgoing = the playing one on air).".to_string(), String::new()];
    if recipes.is_empty() {
        lines.push("codjd has no recipes".to_string());
    }
    for (name, why, bars) in recipes {
        lines.push(format!("{name}  ({bars} bars): {why}"));
    }
    (" mix recipes · any key closes ".to_string(), lines)
}

/// Text after a successful write
pub fn done_text(recipe: &str, pair: &Pair, bars: u32, merged: &Merged) -> String {
    let tracks = format!("{} → {}", pair.from_title, pair.to_title);
    match (merged.existing, merged.replaced) {
        (Some(line), true) => format!("replaced the {tracks} mix at line {line} with {recipe} ({bars} bars): :arm {}", merged.mix_number),
        (Some(line), false) => format!(
            "added {recipe} {tracks} ({bars} bars) as mix {}; an older {tracks} block exists at line {line}: delete one with :e",
            merged.mix_number
        ),
        (None, _) => format!("added {recipe} {tracks} ({bars} bars) as mix {}: :arm {}", merged.mix_number, merged.mix_number),
    }
}

/// Old codjd without the recipe methods
pub fn describe_error(what: &str, e: &zbus::Error) -> String {
    let text = e.to_string();
    if text.contains("UnknownMethod") || text.contains("Unknown method") || text.contains("No such method") {
        "codjd has no recipes: update codjd".into()
    } else {
        super::describe_error(what, e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SET: &str = "# demo\ntrack warm \"Aftertekno\"\n  cue drop bar 33\n\ntrack mid \"Smokin\"\n\nmix warm -> mid at bar 3 over 16 bars\n  bar 0 mid play\n\n  bar 8 swap low\n\ntrack peak \"Kaiku\"\n";
    const BLOCK: &str = "# blend\nmix mid -> peak at bar 3 over 8 bars\n  bar 0 peak play\n";

    #[test]
    fn parsing() {
        assert_eq!(parse(&["new"]), Command::MixNew { recipe: None, bars: None });
        assert_eq!(parse(&["new", "blend"]), Command::MixNew { recipe: Some("blend".into()), bars: None });
        assert_eq!(parse(&["new", "blend", "32"]), Command::MixNew { recipe: Some("blend".into()), bars: Some(32) });
        for bad in [&[][..], &["old"], &["new", "blend", "0"], &["new", "blend", "x"], &["new", "blend", "9999"], &["new", "a", "4", "5"]] {
            assert!(matches!(parse(bad), Command::Unknown(_)), "{bad:?}");
        }
    }

    #[test]
    fn append_with_a_blank_line_between() {
        let m = merge_block(SET, BLOCK, "mid", "peak", false);
        assert!(m.text.starts_with(SET.trim_end()) && m.text.ends_with("  bar 0 peak play\n"), "{}", m.text);
        assert!(m.text.contains("\"Kaiku\"\n\n# blend\nmix mid -> peak"), "{}", m.text);
        assert_eq!((m.existing, m.replaced, m.mix_number), (None, false, 2));
        // an empty set gets just the block
        assert_eq!(merge_block("", BLOCK, "mid", "peak", false).text, format!("{BLOCK}"));
        // a source without a final newline is not glued to the block
        assert!(merge_block("track a \"x\"", BLOCK, "a", "b", false).text.starts_with("track a \"x\"\n\n# blend"));
    }

    #[test]
    fn an_older_block_is_found_by_its_header_words() {
        assert_eq!(find_mix(SET, "warm", "mid"), Some((6, 10)));
        assert_eq!(find_mix(SET, "mid", "warm"), None);
        assert_eq!(find_mix(SET, "war", "mid"), None);
        // comments and spacing do not matter, a commented-out header does not count
        assert_eq!(find_mix("mix  a->b  at bar 1 over 4 bars # x\n bar 0 b play\n", "a", "b"), Some((0, 2)));
        assert_eq!(find_mix("# mix a -> b\n", "a", "b"), None);
        // the block ends at the next header of any kind
        assert_eq!(find_mix("mix a -> b over 4 bars\n  bar 1 x\nset \"n\"\n", "a", "b"), Some((0, 2)));
    }

    #[test]
    fn appending_next_to_an_older_block_tells_about_it() {
        let block = "mix warm -> mid at bar 5 over 8 bars\n  bar 0 mid play\n";
        let m = merge_block(SET, block, "warm", "mid", false);
        assert_eq!((m.existing, m.replaced, m.mix_number), (Some(7), false, 2));
        assert_eq!(m.text.matches("mix warm -> mid").count(), 2);
    }

    #[test]
    fn replacing_keeps_everything_around_the_block() {
        let block = "mix warm -> mid at bar 5 over 8 bars\n  bar 0 mid play\n";
        let m = merge_block(SET, block, "warm", "mid", true);
        assert!(m.replaced && m.existing == Some(7) && m.mix_number == 1);
        assert_eq!(m.text.matches("mix warm -> mid").count(), 1);
        assert!(m.text.contains("over 8 bars") && !m.text.contains("over 16 bars") && !m.text.contains("swap low"), "{}", m.text);
        // the text before and the next track stay, with the blank line between
        assert!(m.text.starts_with("# demo\ntrack warm") && m.text.ends_with("  bar 0 mid play\n\ntrack peak \"Kaiku\"\n"), "{}", m.text);
        // nothing to replace: it is an append
        let m = merge_block(SET, BLOCK, "mid", "peak", true);
        assert!(!m.replaced && m.existing.is_none());
    }

    #[test]
    fn what_a_recipe_reply_must_look_like() {
        assert!(looks_like_block(BLOCK));
        assert!(!looks_like_block("unknown recipe 'x'"));
        assert!(!looks_like_block("# mix a -> b only in a comment"));
    }

    fn deck(path: &str, title: &str, playing: bool, fader: f64) -> DeckInfo {
        DeckInfo { loaded: !path.is_empty(), path: path.into(), title: title.into(), playing, fader, ..Default::default() }
    }

    fn set() -> SetState {
        SetState {
            name: "demo".into(),
            tracks: vec![
                ("warm".into(), "Aftertekno".into(), "/m/a.flac".into()),
                ("mid".into(), "Smokin".into(), "/m/b.flac".into()),
            ],
            armed: -1,
            out_deck: -1,
            ..Default::default()
        }
    }

    #[test]
    fn the_playing_deck_is_outgoing_the_other_is_incoming() {
        let m = MixerInfo::default();
        let p = pick_pair(&[deck("/m/a.flac", "A", true, 1.0), deck("/m/b.flac", "B", false, 1.0)], &m, &set()).unwrap();
        assert_eq!((p.out_deck, p.from_id.as_str(), p.to_id.as_str()), (0, "warm", "mid"));
        let p = pick_pair(&[deck("/m/a.flac", "A", false, 1.0), deck("/m/b.flac", "B", true, 1.0)], &m, &set()).unwrap();
        assert_eq!((p.out_deck, p.from_id.as_str(), p.to_id.as_str()), (1, "mid", "warm"));
        assert_eq!((p.from_title.as_str(), p.to_title.as_str()), ("Smokin", "Aftertekno"));
        // both playing: the louder one is on air
        let p = pick_pair(&[deck("/m/a.flac", "A", true, 0.2), deck("/m/b.flac", "B", true, 1.0)], &m, &set()).unwrap();
        assert_eq!(p.out_deck, 1);
    }

    #[test]
    fn refuses_with_a_reason() {
        let m = MixerInfo::default();
        let a = deck("/m/a.flac", "A", true, 1.0);
        assert!(pick_pair(&[a.clone(), deck("", "", false, 1.0)], &m, &set()).unwrap_err().contains("both decks"));
        assert!(pick_pair(&[a.clone(), deck("/m/b.flac", "B", false, 1.0)], &m, &SetState::default()).unwrap_err().contains("no active set"));
        // a track that is not in the set
        let e = pick_pair(&[a.clone(), deck("/m/other.flac", "Other", false, 1.0)], &m, &set()).unwrap_err();
        assert!(e.starts_with(NOT_IN_SET) && e.contains("Other") && !e.contains("(A"), "{e}");
        // neither playing with both loaded: the deck rule asks
        let e = pick_pair(&[deck("/m/a.flac", "A", false, 1.0), deck("/m/b.flac", "B", false, 1.0)], &m, &set()).unwrap_err();
        assert!(e.contains("no deck is playing"), "{e}");
        // the same track twice
        let e = pick_pair(&[a.clone(), deck("/m/a.flac", "A", false, 1.0)], &m, &set()).unwrap_err();
        assert!(e.contains("same track"), "{e}");
    }

    #[test]
    fn texts() {
        let p = Pair { out_deck: 0, from_id: "a".into(), to_id: "b".into(), from_title: "Aft".into(), to_title: "Smk".into() };
        let appended = Merged { text: String::new(), existing: None, replaced: false, mix_number: 3 };
        assert_eq!(done_text("blend", &p, 16, &appended), "added blend Aft → Smk (16 bars) as mix 3: :arm 3");
        let older = Merged { existing: Some(9), ..appended.clone() };
        assert!(done_text("blend", &p, 16, &older).contains("older Aft → Smk block exists at line 9"));
        let replaced = Merged { existing: Some(9), replaced: true, ..appended };
        assert!(done_text("blend", &p, 16, &replaced).starts_with("replaced the Aft → Smk mix at line 9"));
        let (title, lines) = recipes_popup(&[("blend".into(), "fade B in".into(), 16)]);
        assert!(title.contains("mix recipes") && lines.iter().any(|l| l == "blend  (16 bars): fade B in"));
    }

    #[test]
    fn old_codjd_says_update() {
        let e = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.UnknownMethod").unwrap(),
            Some("Unknown method 'SetRecipes'".into()),
            zbus::message::Message::method_call("/", "x").unwrap().build(&()).unwrap(),
        );
        assert_eq!(describe_error("SetRecipes", &e), "codjd has no recipes: update codjd");
    }
}
