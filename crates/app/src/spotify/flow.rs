//! Local Flow preferences; Spotify playlists supply the music.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const PREFIX: &str = "spotify-flow:";
pub(crate) const MOODS: &[(&str, &str)] = &[
    ("default", "All moods"),
    ("chill", "Chill"),
    ("focus", "Focus"),
    ("party", "Party"),
    ("workout", "Workout"),
    ("happy", "Happy"),
    ("sad", "Sad"),
    ("sleep", "Sleep"),
    ("romance", "Romance"),
];
pub(crate) fn mood_genres(mood: &str) -> Vec<String> {
    let genres: &[&str] = match mood {
        "chill" => &["Chillout", "Lo-fi", "Acoustic"],
        "focus" => &["Ambient", "Classical", "Instrumental"],
        "party" => &["Dance", "Pop", "House"],
        "workout" => &["EDM", "Hip Hop", "Rock"],
        "happy" => &["Pop", "Disco", "Funk"],
        "sad" => &["Acoustic", "Indie", "Soul"],
        "sleep" => &["Ambient", "Classical", "Instrumental"],
        "romance" => &["R&B", "Soul", "Jazz"],
        _ => &[],
    };
    genres.iter().map(|genre| (*genre).to_owned()).collect()
}

pub(crate) fn save(key: &str, edits: &str) -> Result<Value, String> {
    let mut selection = Selection::parse(key)?;
    let edits: Vec<Value> = serde_json::from_str(edits).map_err(|_| "Invalid Flow genre edits")?;
    if edits.len() > 8192 {
        return Err("Too many Flow genre edits".into());
    }
    for edit in edits {
        let genre = edit["clusterId"]
            .as_str()
            .filter(|g| valid_genre(g))
            .ok_or("Invalid Flow genre")?;
        let enabled = edit["isEnabled"]
            .as_bool()
            .ok_or("Invalid Flow genre state")?;
        selection.genres.retain(|g| !g.eq_ignore_ascii_case(genre));
        if enabled {
            selection.genres.push(genre.to_owned());
        }
    }
    let selection = Selection::parse(&selection.key())?;
    Ok(selection.tuner(&selection.key()))
}

pub(crate) const BROWSE_GENRES: &[&str] = &[
    "Pop",
    "Rock",
    "Hip Hop",
    "R&B",
    "Electronic",
    "Dance",
    "House",
    "Techno",
    "Jazz",
    "Blues",
    "Soul",
    "Funk",
    "Classical",
    "Metal",
    "Punk",
    "Indie",
    "Alternative",
    "Folk",
    "Country",
    "Reggae",
    "Latin",
    "Reggaeton",
    "Afrobeats",
    "Disco",
    "Ambient",
    "Lo-fi",
    "K-Pop",
    "J-Pop",
    "Soundtracks",
    "Acoustic",
    "Trance",
    "Drum & Bass",
    "Synthwave",
    "Indie Rock",
    "Indie Pop",
    "Hard Rock",
    "Heavy Metal",
    "Dance Pop",
    "Bluegrass",
    "Chillout",
    "Trip-hop",
    "Deep House",
    "Tech House",
    "Dubstep",
    "Hardstyle",
    "Synthpop",
    "Electropop",
    "Dream Pop",
    "Pop Punk",
    "Post-punk",
    "New Wave",
    "Progressive Rock",
    "Psychedelic Rock",
    "Shoegaze",
    "Metalcore",
    "Death Metal",
    "Black Metal",
    "Trap",
    "Drill",
    "Neo Soul",
];

const EXTRA_GENRES: &[&str] = &[
    "Latin Pop",
    "Salsa",
    "Bachata",
    "Amapiano",
    "Dancehall",
    "Dub",
    "Ska",
    "Bossa Nova",
    "Samba",
    "Tango",
    "Flamenco",
    "Fado",
    "Traditional",
    "Instrumental",
    "Gospel",
    "Opera",
    "Cumbia",
    "Merengue",
    "Bolero",
    "Afropop",
    "Highlife",
    "Zouk",
    "Rap",
    "EDM",
    "Singer-Songwriter",
    "World Music",
    "Turkish Pop",
    "Arabic Pop",
    "Turkish Rock",
    "Arabesk",
    "Progressive Metal",
    "Symphonic Metal",
    "Grunge",
    "Emo",
    "Garage Rock",
    "Classic Rock",
    "Progressive House",
    "Electro Swing",
    "Bhangra",
    "Bollywood",
];

/// Full Every Noise index, with existing display names retained for saved choices.
pub(crate) fn genres() -> &'static [&'static str] {
    static CATALOG: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut entries = BROWSE_GENRES.to_vec();
        entries.extend_from_slice(EXTRA_GENRES);
        let mut seen: std::collections::HashSet<_> =
            entries.iter().map(|g| g.to_lowercase()).collect();
        for line in include_str!("../../../../assets/spotify-flow-genres.tsv").lines() {
            let name = line.split('\t').next().unwrap_or("");
            if !name.is_empty() && seen.insert(name.to_lowercase()) {
                entries.push(name);
            }
        }
        entries
    })
}
pub(crate) fn genre_radio(genre: &str) -> Option<&'static str> {
    include_str!("../../../../assets/spotify-flow-genres.tsv")
        .lines()
        .find_map(|line| {
            let (name, uri) = line.split_once('\t')?;
            (name.eq_ignore_ascii_case(genre) && !uri.is_empty()).then_some(uri)
        })
}
pub(crate) fn valid_genre(genre: &str) -> bool {
    !genre.trim().is_empty()
        && genre == genre.trim()
        && genre.chars().count() <= 100
        && !genre.chars().any(char::is_control)
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Selection {
    pub mood: String,
    pub genres: Vec<String>,
}
impl Selection {
    pub fn parse(key: &str) -> Result<Self, String> {
        let mut selection: Self = if let Some(value) = key.strip_prefix(PREFIX) {
            if value.len() > 65_536 {
                return Err("Invalid Flow selection".into());
            }
            serde_json::from_str(value).map_err(|_| "Invalid Flow selection")?
        } else {
            Self {
                mood: key.into(),
                genres: mood_genres(key),
            }
        };
        // Migrate the old personal-playlist default to the genre/mood picker.
        if selection.mood.starts_with("spotify:playlist:") || selection.mood.is_empty() {
            selection.mood = "default".into();
        }
        if !MOODS.iter().any(|(id, _)| *id == selection.mood)
            || selection.genres.len() > 128
            || selection.genres.iter().any(|g| !valid_genre(g))
        {
            return Err("Invalid Flow mood or genre".into());
        }
        selection.genres.sort();
        selection.genres.dedup();
        Ok(selection)
    }
    pub fn key(&self) -> String {
        format!("{PREFIX}{}", serde_json::to_string(self).unwrap())
    }
    pub fn tuner(&self, key: &str) -> Value {
        json!({"id":key,"customizable":true,"genres":genres().iter().copied().chain(self.genres.iter().map(String::as_str).filter(|g| !genres().iter().any(|known| known.eq_ignore_ascii_case(g)))).map(|g|
            json!({"id":g,"title":g,"isEnabled":self.genres.iter().any(|s| s.eq_ignore_ascii_case(g))})
        ).collect::<Vec<_>>()})
    }
    pub fn queries(&self) -> Vec<String> {
        let mood = if self.mood == "default" {
            ""
        } else {
            &self.mood
        };
        if self.genres.is_empty() {
            vec![if mood.is_empty() {
                "popular hits".into()
            } else {
                mood.into()
            }]
        } else {
            self.genres
                .iter()
                .map(|g| format!("{mood} {g}").trim().to_owned())
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mood_presets_and_saved_edits_roundtrip() {
        for (mood, _) in MOODS.iter().filter(|(id, _)| *id != "default") {
            let selection = Selection::parse(mood).unwrap();
            assert!(!selection.genres.is_empty());
            assert!(selection
                .genres
                .iter()
                .all(|g| genres().contains(&g.as_str())));
        }
        let saved = save(
            "chill",
            r#"[{"clusterId":"Lo-fi","isEnabled":false},{"clusterId":"Jazz","isEnabled":true}]"#,
        )
        .unwrap();
        let restored = Selection::parse(saved["id"].as_str().unwrap()).unwrap();
        assert!(restored.genres.contains(&"Jazz".to_owned()));
        assert!(!restored.genres.contains(&"Lo-fi".to_owned()));
        let empty = Selection {
            mood: "chill".into(),
            genres: vec![],
        };
        assert!(Selection::parse(&empty.key()).unwrap().genres.is_empty());
        assert!(save("chill", r#"[{"clusterId":"Jazz"}]"#).is_err());
    }
    #[test]
    fn flow_includes_style_mix_genres_and_preserves_existing_choices() {
        for (genre, _) in crate::style_mix::GENRES {
            let genre = match *genre {
                "Hip-hop" => "Hip Hop",
                "Soundtrack" => "Soundtracks",
                genre => genre,
            };
            assert!(genres().contains(&genre), "Missing Flow genre: {genre}");
        }
        let selection = Selection {
            mood: "default".into(),
            genres: genres().iter().take(128).map(|g| (*g).to_owned()).collect(),
        };
        let restored = Selection::parse(&selection.key()).unwrap();
        assert_eq!(restored.genres.len(), 128);
        assert_eq!(
            genres()
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            genres().len()
        );
        for genre in ["Hip Hop", "Soundtracks", "K-Pop", "J-Pop", "Latin", "Dance"] {
            assert!(genres().contains(&genre));
        }
    }
    #[test]
    fn selection_roundtrip_and_validation() {
        let s = Selection {
            mood: "chill".into(),
            genres: vec!["Jazz".into(), "Soul".into()],
        };
        let restored = Selection::parse(&s.key()).unwrap();
        assert_eq!(restored.queries(), ["chill Jazz", "chill Soul"]);
        assert_eq!(
            restored.tuner(&s.key())["genres"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|g| g["isEnabled"] == true)
                .count(),
            2
        );
        assert!(Selection::parse("spotify-flow:{\"mood\":\"bogus\",\"genres\":[]}").is_err());
        assert_eq!(
            Selection::parse("spotify:playlist:old").unwrap().mood,
            "default"
        );
    }
    #[test]
    fn full_catalog_and_custom_genres_are_available() {
        assert_eq!(BROWSE_GENRES.len(), 60);
        assert!(genres().len() >= 6291);
        for genre in [
            "turkish pop",
            "arabesk",
            "turkish rock",
            "melodic death metal",
        ] {
            assert!(
                genres().iter().any(|name| name.eq_ignore_ascii_case(genre)),
                "Missing {genre}"
            );
        }
        assert!(genre_radio("turkish pop")
            .unwrap()
            .starts_with("spotify:playlist:"));
        let selection = Selection {
            mood: "default".into(),
            genres: vec!["My emerging subgenre".into()],
        };
        let restored = Selection::parse(&selection.key()).unwrap();
        assert_eq!(restored.queries(), ["My emerging subgenre"]);
        assert!(restored.tuner(&restored.key())["genres"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["title"] == "My emerging subgenre" && g["isEnabled"] == true));
        assert!(!valid_genre("bad\ngenre"));
        let saved = Selection {
            mood: "default".into(),
            genres: vec!["turkish pop".into()],
        };
        assert!(saved.tuner(&saved.key())["genres"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["title"] == "Turkish Pop" && g["isEnabled"] == true));
    }
}
