use std::path::Path;

use regex::{Regex, RegexBuilder};
use zbus::zvariant::FilePath;

use crate::types::Music;

/// Filters the music list with a case-insensitive regex.
///
/// A plain query matches title, artist or file name. `title:`, `artist:` and `genre:` restrict it
/// to one field. A pattern that isn't valid regex yet (e.g. a half-typed `(`) is matched literally,
/// so typing never empties the list just because the regex is incomplete.
pub fn search_musics(query: &str, list: &[Music]) -> Vec<Music> {
    let (field, pattern) = match query.split_once(':') {
        Some(("title", p)) => (Field::Title, p.trim()),
        Some(("artist", p)) => (Field::Artist, p.trim()),
        Some(("genre", p)) => (Field::Genre, p.trim()),
        _ => (Field::Any, query.trim()),
    };
    if pattern.is_empty() {
        return list.to_vec();
    }
    let re = build_regex(pattern);
    list.iter()
        .filter(|m| match field {
            Field::Any => {
                re.is_match(&m.title) || re.is_match(&m.artist) || re.is_match(&file_name(&m.path))
            }
            Field::Title => re.is_match(&m.title),
            Field::Artist => re.is_match(&m.artist),
            Field::Genre => re.is_match(&m.genre),
        })
        .cloned()
        .collect()
}

enum Field {
    Any,
    Title,
    Artist,
    Genre,
}

/// Case-insensitive regex, or a literal match when `pattern` isn't valid regex (yet)
pub fn build_regex(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .unwrap_or_else(|_| {
            RegexBuilder::new(&regex::escape(pattern))
                .case_insensitive(true)
                .build()
                .expect("escaped pattern is always valid")
        })
}

/// File name of a music path, for untagged files whose title is empty
pub fn file_name<'f>(path: &'f FilePath<'f>) -> String {
    let path: &Path = path.into();
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// File name without its extension, to show untagged files
pub fn file_stem<'f>(path: &'f FilePath<'f>) -> String {
    let path: &Path = path.into();
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn music(title: &str, artist: &str, genre: &str, path: &str) -> Music {
        Music {
            title: title.to_string(),
            artist: artist.to_string(),
            genre: genre.to_string(),
            path: PathBuf::from(path).into(),
            ..Default::default()
        }
    }

    fn list() -> Vec<Music> {
        vec![
            music("Aftertekno", "Pneumatix", "Techno", "/m/Pneumatix - Aftertekno.flac"),
            music("Attic Attack", "Pneumatix", "Techno", "/m/attic.flac"),
            music("", "", "", "/m/05-aasi_and_swooh-cadillac.mp3"),
            music("Back Again", "DJ Software", "House", "/m/back.mp3"),
        ]
    }

    fn titles(v: Vec<Music>) -> Vec<String> {
        v.into_iter().map(|m| m.title).collect()
    }

    #[test]
    fn empty_query_returns_everything() {
        assert_eq!(search_musics("", &list()).len(), 4);
        assert_eq!(search_musics("   ", &list()).len(), 4);
    }

    #[test]
    fn regex_is_case_insensitive() {
        assert_eq!(titles(search_musics("^att", &list())), vec!["Attic Attack"]);
        assert_eq!(titles(search_musics("tek|again", &list())), vec!["Aftertekno", "Back Again"]);
    }

    #[test]
    fn matches_artist_and_file_name() {
        assert_eq!(search_musics("pneumatix", &list()).len(), 2);
        // untagged file is found by its file name
        assert_eq!(search_musics("cadillac", &list()).len(), 1);
    }

    #[test]
    fn field_prefix_restricts_search() {
        assert_eq!(titles(search_musics("artist: dj", &list())), vec!["Back Again"]);
        assert_eq!(search_musics("genre:techno", &list()).len(), 2);
        // "attic" is only in a title/file name, not in any artist
        assert!(search_musics("artist:attic", &list()).is_empty());
    }

    #[test]
    fn invalid_regex_is_matched_literally() {
        assert!(search_musics("(", &list()).is_empty());
        let mut l = list();
        l.push(music("Remix (Edit)", "", "", "/m/x.mp3"));
        assert_eq!(titles(search_musics("(edit", &l)), vec!["Remix (Edit)"]);
    }

    #[test]
    fn no_match_is_empty() {
        assert!(search_musics("zzzz", &list()).is_empty());
    }
}
