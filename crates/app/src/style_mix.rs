//! Country/genre mixes use explicitly matching playlists, never additive Flow filters.
use crate::{
    catalog::{self, Entry},
    engine::Engine,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const REGIONS: &[(&str, &[&str])] = &[
    ("Turkish", &["turkish", "turkce", "turk"]),
    (
        "Japanese",
        &["japanese", "japan", "j rock", "j pop", "j metal"],
    ),
    ("Korean", &["korean", "korea", "k pop", "k rock"]),
    ("French", &["french", "francais", "francaise"]),
    ("Arabic", &["arabic", "arab", "arabe"]),
    (
        "Brazilian",
        &["brazilian", "brasil", "brasileiro", "brasileira"],
    ),
    ("Spanish", &["spanish", "espanol", "espanola", "espana"]),
    (
        "Portuguese",
        &["portuguese", "portugues", "portuguesa", "portugal"],
    ),
    ("German", &["german", "deutsch", "deutsche", "deutscher"]),
    ("Italian", &["italian", "italiano", "italiana", "italiane"]),
    ("Hindi", &["hindi", "bollywood"]),
    ("Punjabi", &["punjabi", "panjabi"]),
    ("Persian", &["persian", "farsi", "iranian"]),
    ("Greek", &["greek", "ellinika", "elliniko"]),
    ("Russian", &["russian", "russkiy", "russkaya"]),
    ("Chinese", &["chinese", "mandarin", "cantopop", "c pop"]),
    ("Indonesian", &["indonesian", "indonesia", "indo"]),
    ("Thai", &["thai", "thailand"]),
    ("Polish", &["polish", "polski", "polska", "polskie"]),
    ("Dutch", &["dutch", "nederlands", "nederlandse"]),
    ("Swedish", &["swedish", "svensk", "svenska"]),
    ("Norwegian", &["norwegian", "norsk", "norske"]),
    ("Finnish", &["finnish", "suomi", "suomen", "suomalainen"]),
    ("Danish", &["danish", "dansk", "danske"]),
    (
        "Ukrainian",
        &[
            "ukrainian",
            "ukrainska",
            "ukrainski",
            "українська",
            "український",
        ],
    ),
    (
        "Romanian",
        &["romanian", "romanesc", "romaneasca", "romanesti"],
    ),
    ("Hungarian", &["hungarian", "magyar", "magyarul"]),
    ("Czech", &["czech", "cesky", "ceska", "ceske"]),
    ("Serbian", &["serbian", "srpski", "srpska", "srpsko"]),
    (
        "Croatian",
        &["croatian", "hrvatski", "hrvatska", "hrvatske"],
    ),
    ("Albanian", &["albanian", "shqip", "shqiptare", "shqiptar"]),
    ("Bengali", &["bengali", "bangla"]),
    ("Tamil", &["tamil"]),
    ("Telugu", &["telugu"]),
    ("Urdu", &["urdu"]),
    ("Vietnamese", &["vietnamese", "vietnam", "viet"]),
    (
        "Filipino / Tagalog",
        &["filipino", "tagalog", "pinoy", "opm"],
    ),
    ("Malay", &["malay", "malaysia", "melayu"]),
    ("Nepali", &["nepali", "nepalese", "nepal"]),
    ("Hebrew", &["hebrew", "israeli", "ivrit"]),
    (
        "Azerbaijani",
        &["azerbaijani", "azerbaijan", "azeri", "azerbaycan"],
    ),
    ("Georgian", &["georgian", "kartuli"]),
    ("Armenian", &["armenian", "armenia", "haykakan"]),
    ("Kurdish", &["kurdish", "kurdi", "kurd"]),
    ("Swahili", &["swahili", "kiswahili"]),
    ("Amharic", &["amharic", "amharic music"]),
    ("Mexican", &["mexican", "mexico", "mexicano", "mexicana"]),
    (
        "Argentinian",
        &["argentinian", "argentine", "argentina", "argentino"],
    ),
    (
        "Colombian",
        &["colombian", "colombia", "colombiano", "colombiana"],
    ),
    ("Chilean", &["chilean", "chile", "chileno", "chilena"]),
    ("Peruvian", &["peruvian", "peru", "peruano", "peruana"]),
    ("Cuban", &["cuban", "cuba", "cubano", "cubana"]),
    ("Dominican", &["dominican", "dominicana", "dominicano"]),
    ("Puerto Rican", &["puerto rican", "puerto rico", "boricua"]),
    ("Jamaican", &["jamaican", "jamaica"]),
    ("Nigerian", &["nigerian", "nigeria", "naija"]),
    ("Ghanaian", &["ghanaian", "ghana"]),
    (
        "South African",
        &["south african", "south africa", "mzansi"],
    ),
    ("Moroccan", &["moroccan", "morocco", "maroc", "marocain"]),
    ("Algerian", &["algerian", "algeria", "algerie", "algerien"]),
    ("Tunisian", &["tunisian", "tunisia", "tunisie", "tunisien"]),
    ("Egyptian", &["egyptian", "egypt", "masri"]),
    ("Lebanese", &["lebanese", "lebanon", "libanais"]),
    ("Icelandic", &["icelandic", "iceland", "islensk"]),
    ("Slovak", &["slovak", "slovensky", "slovenska"]),
    ("Slovenian", &["slovenian", "slovene", "slovenski"]),
    ("Bulgarian", &["bulgarian", "bulgaria", "balgarski"]),
    (
        "Lithuanian",
        &["lithuanian", "lithuania", "lietuviska", "lietuviskas"],
    ),
    ("Latvian", &["latvian", "latvia", "latviesu"]),
    ("Estonian", &["estonian", "estonia", "eesti"]),
    ("Irish", &["irish", "ireland", "gaeilge"]),
    ("Scottish", &["scottish", "scotland", "scots"]),
    ("Welsh", &["welsh", "wales", "cymraeg"]),
    ("Basque", &["basque", "euskara", "euskal"]),
    ("Catalan", &["catalan", "catala", "catalana"]),
    ("English", &["english", "anglophone"]),
    ("American", &["american", "usa", "united states"]),
    ("British", &["british", "uk", "united kingdom"]),
    (
        "Canadian",
        &["canadian", "canada", "canadien", "canadienne"],
    ),
    ("Australian", &["australian", "australia", "aussie"]),
    ("New Zealand", &["new zealand", "kiwi"]),
    ("Austrian", &["austrian", "austria", "osterreich"]),
    ("Swiss", &["swiss", "switzerland", "suisse", "schweiz"]),
    ("Belgian", &["belgian", "belgium", "belge"]),
];
pub const GENRES: &[(&str, &[&str])] = &[
    ("Pop", &["pop"]),
    ("Rock", &["rock"]),
    ("Hip-hop", &["hip hop", "hiphop", "rap"]),
    ("Metal", &["metal"]),
    ("Jazz", &["jazz", "caz"]),
    (
        "Electronic",
        &["electronic", "electronica", "elektronik", "edm"],
    ),
    ("R&B", &["r b", "rnb", "rhythm and blues"]),
    ("Indie", &["indie", "independent"]),
    ("Folk", &["folk", "halk"]),
    ("Classical", &["classical", "klasik", "classique"]),
    ("Blues", &["blues"]),
    ("Soul", &["soul"]),
    ("Funk", &["funk"]),
    ("Disco", &["disco", "disko"]),
    ("Punk", &["punk"]),
    ("Alternative", &["alternative", "alternatif", "alternativo"]),
    ("Reggae", &["reggae"]),
    ("Reggaeton", &["reggaeton", "regueton"]),
    ("House", &["house"]),
    ("Techno", &["techno", "tekno"]),
    ("Trance", &["trance"]),
    (
        "Drum & Bass",
        &["drum bass", "drum and bass", "drum n bass", "dnb", "d n b"],
    ),
    (
        "Acoustic",
        &["acoustic", "acustico", "acustica", "akustik", "acoustique"],
    ),
    ("Synthwave", &["synthwave", "synth wave"]),
    ("Indie Rock", &["indie rock", "indierock"]),
    ("Indie Pop", &["indie pop", "indiepop"]),
    ("Hard Rock", &["hard rock", "hardrock"]),
    ("Heavy Metal", &["heavy metal", "heavymetal"]),
    ("Dance Pop", &["dance pop", "dancepop"]),
    ("Country", &["country"]),
    ("Bluegrass", &["bluegrass", "blue grass"]),
    ("Ambient", &["ambient"]),
    ("Chillout", &["chillout", "chill out"]),
    ("Lo-fi", &["lo fi", "lofi"]),
    ("Trip-hop", &["trip hop", "triphop"]),
    ("Deep House", &["deep house", "deephouse"]),
    ("Tech House", &["tech house", "techhouse"]),
    ("Dubstep", &["dubstep"]),
    ("Hardstyle", &["hardstyle", "hard style"]),
    ("Synthpop", &["synthpop", "synth pop"]),
    ("Electropop", &["electropop", "electro pop"]),
    ("Dream Pop", &["dream pop", "dreampop"]),
    ("Pop Punk", &["pop punk", "poppunk"]),
    ("Post-punk", &["post punk", "postpunk"]),
    ("New Wave", &["new wave", "newwave"]),
    (
        "Progressive Rock",
        &["progressive rock", "prog rock", "progrock"],
    ),
    ("Psychedelic Rock", &["psychedelic rock", "psych rock"]),
    ("Shoegaze", &["shoegaze", "shoegazing"]),
    ("Metalcore", &["metalcore", "metal core"]),
    ("Death Metal", &["death metal", "deathmetal"]),
    ("Black Metal", &["black metal", "blackmetal"]),
    ("Trap", &["trap"]),
    ("Drill", &["drill"]),
    ("Neo Soul", &["neo soul", "neosoul"]),
    ("Latin Pop", &["latin pop", "latinpop", "pop latino"]),
    ("Salsa", &["salsa"]),
    ("Bachata", &["bachata"]),
    ("Afrobeats", &["afrobeats", "afro beats"]),
    ("Amapiano", &["amapiano"]),
    ("Dancehall", &["dancehall", "dance hall"]),
    ("Dub", &["dub"]),
    ("Ska", &["ska"]),
    ("Bossa Nova", &["bossa nova", "bossanova"]),
    ("Samba", &["samba"]),
    ("Tango", &["tango"]),
    ("Flamenco", &["flamenco"]),
    ("Fado", &["fado"]),
    (
        "Traditional",
        &["traditional", "traditionnel", "tradicional", "geleneksel"],
    ),
    (
        "Soundtrack",
        &["soundtrack", "soundtracks", "ost", "film music"],
    ),
    (
        "Instrumental",
        &["instrumental", "instrumentals", "enstrumantal"],
    ),
    ("Gospel", &["gospel"]),
    ("Opera", &["opera"]),
    ("Cumbia", &["cumbia"]),
    ("Merengue", &["merengue"]),
    ("Bolero", &["bolero"]),
    ("Afropop", &["afropop", "afro pop"]),
    ("Highlife", &["highlife", "high life"]),
    ("Zouk", &["zouk"]),
];
pub const MARKER: &str = "style_mix";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub region: usize,
    pub genre: usize,
}
impl Selection {
    pub fn new(region: usize, genre: usize) -> Result<Self, String> {
        if region >= REGIONS.len() || genre >= GENRES.len() {
            return Err("Choose a valid country and genre".into());
        }
        Ok(Self { region, genre })
    }
    pub fn parse(key: &str) -> Result<Self, String> {
        let (region, genre) = key.split_once(':').ok_or("Choose a country and genre")?;
        Self::new(
            region.parse().map_err(|_| "Invalid country")?,
            genre.parse().map_err(|_| "Invalid genre")?,
        )
    }
    pub fn key(self) -> String {
        format!("{}:{}", self.region, self.genre)
    }
    pub fn title(self) -> String {
        format!("{} {}", REGIONS[self.region].0, GENRES[self.genre].0)
    }
    fn matches(self, title: &str) -> bool {
        let title = normalize(title);
        let contains = |aliases: &[&str]| {
            aliases
                .iter()
                .any(|alias| title.contains(&format!(" {} ", alias)))
        };
        contains(REGIONS[self.region].1) && contains(GENRES[self.genre].1)
    }
}

fn normalize(text: &str) -> String {
    let mut normalized = String::from(" ");
    // Match whole words; punctuation also handles J-Rock, K-Pop and R&B.
    for ch in text.to_lowercase().chars() {
        normalized.push(match ch {
            'ü' | 'ú' | 'ù' | 'ű' => 'u',
            'ö' | 'ó' | 'ô' | 'ő' => 'o',
            'ç' | 'č' | 'ć' => 'c',
            'ñ' | 'ń' => 'n',
            'ä' | 'å' | 'ă' | 'ą' => 'a',
            'ş' | 'ș' | 'š' => 's',
            'ž' | 'ź' | 'ż' => 'z',
            'ř' => 'r',
            'ł' => 'l',
            'ț' | 'ţ' => 't',
            'đ' => 'd',
            'ý' => 'y',
            'ğ' => 'g',
            'ı' | 'ï' => 'i',
            'é' | 'è' | 'ê' | 'ě' | 'ę' => 'e',
            'ã' | 'á' | 'à' | 'â' => 'a',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        });
    }
    format!(
        " {} ",
        normalized.split_whitespace().collect::<Vec<_>>().join(" ")
    )
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

pub fn options() -> Value {
    json!({"regions":REGIONS.iter().map(|r|r.0).collect::<Vec<_>>(),
        "genres":GENRES.iter().map(|g|g.0).collect::<Vec<_>>()})
}

fn public(engine: &Engine, path: &str) -> Result<Value, String> {
    let url = format!("https://api.deezer.com/{path}");
    let result = retry_quota(
        || {
            catalog::public_json(&url, || {
                wait_for_catalog_slot();
                engine.fetch(&url)
            })
            .map(|value| (*value).clone())
        },
        || {
            let mut next = next_catalog_request().lock().unwrap();
            *next = (*next).max(Instant::now() + Duration::from_secs(6));
        },
    );
    #[cfg(test)]
    if let Err(error) = &result {
        eprintln!("Style Mix catalog lookup {path}: {error}");
    }
    result
}

fn next_catalog_request() -> &'static Mutex<Instant> {
    static NEXT: OnceLock<Mutex<Instant>> = OnceLock::new();
    NEXT.get_or_init(|| Mutex::new(Instant::now()))
}

fn wait_for_catalog_slot() {
    let wait = {
        let now = Instant::now();
        let mut next = next_catalog_request().lock().unwrap();
        let slot = (*next).max(now);
        *next = slot + Duration::from_millis(75);
        slot.saturating_duration_since(now)
    };
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
}

fn retry_quota<T>(
    mut fetch: impl FnMut() -> Result<T, String>,
    mut defer: impl FnMut(),
) -> Result<T, String> {
    for attempt in 0..3 {
        match fetch() {
            Err(error) if attempt < 2 && error.to_lowercase().contains("quota limit") => defer(),
            result => return result,
        }
    }
    unreachable!()
}

fn search_playlists(engine: &Engine, selection: Selection) -> Result<Value, String> {
    search_playlists_with(selection, |query| {
        public(
            engine,
            &format!("search/playlist?q={}&limit=50", encode(query)),
        )
    })
}

fn search_playlists_with(
    selection: Selection,
    mut fetch: impl FnMut(&str) -> Result<Value, String>,
) -> Result<Value, String> {
    let aliases = REGIONS[selection.region].1;
    // Search one language name at a time (particularly Filipino / Tagalog).
    // Try the local name when the English query has no matching playlist titles.
    let mut value = Value::Null;
    for region in aliases.iter().take(2) {
        let query = format!("{} {}", region, GENRES[selection.genre].0);
        value = fetch(&query)?;
        if !matching_playlists(&value, selection).is_empty() {
            break;
        }
    }
    Ok(value)
}

pub fn genre_options(region: usize) -> Result<Value, String> {
    Selection::new(region, 0)?;
    Ok(json!({"region": region, "genres": GENRES.iter().enumerate()
        .map(|(id, genre)| json!({"id": id, "title": genre.0}))
        .collect::<Vec<_>>() }))
}
pub fn available_genres(engine: &Engine, region: usize) -> Result<Value, String> {
    available_genres_while(engine, region, || true)
}

pub fn available_genres_while(
    engine: &Engine,
    region: usize,
    current: impl Fn() -> bool + Sync,
) -> Result<Value, String> {
    Selection::new(region, 0)?;
    if !current() {
        return Err("Superseded genre check".into());
    }
    let mut broad = Vec::new();
    for alias in REGIONS[region].1.iter().take(2) {
        if !current() {
            return Err("Superseded genre check".into());
        }
        if let Ok(value) = public(
            engine,
            &format!("search/playlist?q={}&limit=100", encode(alias)),
        ) {
            broad.push(value);
        }
    }
    discover_genres(
        region,
        |selection| {
            if !current() {
                return Err("Superseded genre check".into());
            }
            if broad
                .iter()
                .any(|search| !matching_playlists(search, selection).is_empty())
            {
                return Ok(true);
            }
            let search = search_playlists(engine, selection)?;
            Ok(!matching_playlists(&search, selection).is_empty())
        },
        &current,
    )
}

pub(crate) fn discover_genres(
    region: usize,
    probe: impl Fn(Selection) -> Result<bool, String> + Sync,
    current: impl Fn() -> bool + Sync,
) -> Result<Value, String> {
    Selection::new(region, 0)?;
    let mut genres = Vec::new();
    let mut failed = 0;
    catalog::parallel::load_with(
        8,
        GENRES.len(),
        |genre| probe(Selection { region, genre }),
        |genre, result| {
            if !current() {
                return Err("Superseded genre check".into());
            }
            match result {
                Ok(true) => genres.push(json!({"id":genre,"title":GENRES[genre].0})),
                Ok(false) => {}
                Err(_) => failed += 1,
            }
            Ok(())
        },
    )?;
    if failed > 0 && genres.is_empty() {
        return Err("Could not check genres. Please try again.".into());
    }
    genres.sort_by_key(|genre| genre["id"].as_u64().unwrap_or_default());
    Ok(json!({"region":region,"genres":genres,"partial":failed > 0}))
}

fn playable_tracks(value: &Value) -> Vec<Entry> {
    value["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|track| track["readable"].as_bool() != Some(false))
        .filter_map(|track| catalog::entry(track, "track"))
        .filter(|track| {
            track.kind == "track" && track.id != "0" && track.id.bytes().all(|b| b.is_ascii_digit())
        })
        .collect()
}

fn matching_playlists(value: &Value, selection: Selection) -> Vec<&Value> {
    let mut seen = HashSet::new();
    let mut playlists: Vec<_> = value["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            let id = catalog::string(p, &["id"]);
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_digit())
                && p["nb_tracks"].as_u64() != Some(0)
                && selection.matches(&catalog::string(p, &["title"]))
                && seen.insert(id)
        })
        .collect();
    playlists.sort_by_key(|p| {
        (
            p["user"]["name"].as_str().unwrap_or("").to_lowercase() != "deezer",
            std::cmp::Reverse(p["nb_tracks"].as_u64().unwrap_or(0)),
        )
    });
    playlists.truncate(3);
    playlists
}

pub fn tracks(engine: &Engine, selection: Selection) -> Result<Vec<Entry>, String> {
    let search = search_playlists(engine, selection)?;
    let playlists = matching_playlists(&search, selection);
    let mut pools = Vec::new();
    let mut failure = None;
    for playlist in playlists {
        let id = catalog::string(playlist, &["id"]);
        let result = public(engine, &format!("playlist/{id}/tracks?limit=100"));
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                failure = Some(error);
                continue;
            }
        };
        let mut pool = playable_tracks(&result);
        // Start at a different place on each build, keeping playlist membership intact.
        if !pool.is_empty() {
            let offset = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() as usize
                % pool.len();
            pool.rotate_left(offset);
        }
        pools.push(pool);
    }
    let tracks = interleave(pools, selection);
    if tracks.is_empty() {
        return Err(failure.unwrap_or_else(|| {
            "No playable tracks found in matching playlists. Try another language or genre.".into()
        }));
    }
    Ok(tracks)
}

fn interleave(pools: Vec<Vec<Entry>>, selection: Selection) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    for index in 0..pools.iter().map(Vec::len).max().unwrap_or(0) {
        for pool in &pools {
            if let Some(track) = pool.get(index).filter(|t| seen.insert(t.id.clone())) {
                let mut track = track.clone();
                track.raw[MARKER] = json!(selection.key());
                tracks.push(track);
            }
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_responses_are_retried_but_other_errors_are_not() {
        let mut calls = 0;
        let mut deferrals = 0;
        let result = retry_quota(
            || {
                calls += 1;
                if calls == 1 {
                    Err("Deezer: Quota limit exceeded".into())
                } else {
                    Ok(42)
                }
            },
            || deferrals += 1,
        );
        assert_eq!(result, Ok(42));
        assert_eq!((calls, deferrals), (2, 1));
        let mut deferrals = 0;
        let result: Result<(), String> = retry_quota(
            || Err("Deezer: Quota limit exceeded".into()),
            || deferrals += 1,
        );
        assert!(result.is_err());
        assert_eq!(deferrals, 2);
        let result: Result<(), String> = retry_quota(
            || Err("Not found".into()),
            || panic!("permanent errors must not retry"),
        );
        assert_eq!(result, Err("Not found".into()));
    }
    #[test]
    fn local_name_fallback_keeps_both_country_and_genre_constraints() {
        let selection = Selection::new(
            REGIONS
                .iter()
                .position(|r| r.0 == "Filipino / Tagalog")
                .unwrap(),
            0,
        )
        .unwrap();
        let mut calls = Vec::new();
        let result = search_playlists_with(selection, |query| {
            calls.push(query.to_string());
            Ok(if query.starts_with("tagalog ") {
                json!({"data":[{"id":1,"title":"Tagalog Pop"}]})
            } else {
                json!({"data":[{"id":2,"title":"Pop Hits"}]})
            })
        })
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert!(!calls.iter().any(|query| query.contains('/')));
        assert_eq!(matching_playlists(&result, selection)[0]["id"], 1);
        let mut calls = 0;
        search_playlists_with(selection, |_| {
            calls += 1;
            Ok(json!({"data":[{"id":1,"title":"Filipino Pop"}]}))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(search_playlists_with(selection, |_| Err("offline".into())).is_err());
    }
    #[test]
    fn availability_only_lists_confirmed_genres_and_preserves_their_ids() {
        let result = discover_genres(
            1,
            |selection| match selection.genre {
                1 | 17 => Ok(true),
                2 => Err("offline".into()),
                _ => Ok(false),
            },
            || true,
        )
        .unwrap();
        assert_eq!(result["region"], 1);
        assert_eq!(
            result["genres"],
            json!([{"id":1,"title":"Rock"},{"id":17,"title":"Reggaeton"}])
        );
        assert_eq!(result["partial"], true);
        assert!(discover_genres(0, |_| Err("offline".into()), || true).is_err());
        assert!(
            discover_genres(0, |_| Ok(false), || true).unwrap()["genres"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(discover_genres(
            usize::MAX,
            |_| panic!("invalid region must not fetch"),
            || true
        )
        .is_err());
        assert!(discover_genres(0, |_| Ok(true), || false).is_err());
    }
    #[test]
    fn unreadable_and_invalid_tracks_do_not_make_a_genre_available() {
        let tracks = playable_tracks(&json!({"data":[
            {"id":1,"title":"Unavailable", "readable":false},
            {"id":0,"title":"Invalid"}, {"id":"bad","title":"Invalid"},
            {"id":2,"title":"Playable", "readable":true}
        ]}));
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, "2");
    }
    #[test]
    fn requires_country_and_genre_in_the_same_playlist() {
        let turkish_pop = Selection::new(0, 0).unwrap();
        for title in ["Turkish Pop Hits", "Türkçe Pop", "Türk Pop 2026"] {
            assert!(turkish_pop.matches(title), "{title}");
        }
        for title in [
            "Turkish Music",
            "Pop Hits",
            "Turkish Rock",
            "Japanese Pop",
            "Turkish Popular Classics",
        ] {
            assert!(!turkish_pop.matches(title), "{title}");
        }
        let japanese_rock = Selection::new(1, 1).unwrap();
        for title in ["Japanese Rock", "J-Rock Hits", "Japan Rock"] {
            assert!(japanese_rock.matches(title), "{title}");
        }
        for title in ["J-Pop", "Japanese Music", "Rock Classics", "Turkish Rock"] {
            assert!(!japanese_rock.matches(title), "{title}");
        }
    }
    #[test]
    fn rejects_invalid_selections_and_filters_unrelated_search_results() {
        for key in ["", "0", "-1:0", "0:999", "999:0", "0:0:1"] {
            assert!(Selection::parse(key).is_err());
        }
        let selection = Selection::parse("0:0").unwrap();
        let results = json!({"data":[{"id":1,"title":"Pop"},{"id":2,"title":"Turkish Music"},
            {"id":3,"title":"Turkish Pop"},{"id":3,"title":"Turkish Pop"}, {"id":"../bad","title":"Turkish Pop"},
            {"id":4,"title":"Turkish Pop","nb_tracks":0}]});
        let matches = matching_playlists(&results, selection);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["id"], 3);
    }
    #[test]
    fn expanded_styles_match_accents_and_require_the_full_subgenre() {
        let selection = |region: &str, genre: &str| {
            Selection::new(
                REGIONS.iter().position(|r| r.0 == region).unwrap(),
                GENRES.iter().position(|g| g.0 == genre).unwrap(),
            )
            .unwrap()
        };
        for (region, genre, title) in [
            ("Spanish", "Acoustic", "Español Acústico"),
            ("Portuguese", "Indie Rock", "Português Indie-Rock"),
            ("German", "Drum & Bass", "Deutsch Drum & Bass"),
            ("Italian", "Dance Pop", "Italian Dance-Pop"),
            ("Persian", "Heavy Metal", "Farsi Heavy Metal"),
            ("Greek", "Reggae", "Greek Reggae"),
            ("Hindi", "House", "Hindi House"),
            ("Punjabi", "Funk", "Punjabi Funk"),
            ("Russian", "Synthwave", "Russian Synthwave"),
            ("Chinese", "Soul", "Chinese Soul"),
            ("Indonesian", "Punk", "Indonesian Punk"),
            ("Thai", "Blues", "Thai Blues"),
        ] {
            assert!(selection(region, genre).matches(title), "{title}");
        }
        assert!(!selection("Portuguese", "Indie Rock").matches("Portuguese Indie Pop"));
        assert!(!selection("Greek", "Reggae").matches("Greek Reggaeton"));
        assert!(!selection("German", "Heavy Metal").matches("German Metal"));
        assert!(!selection("Spanish", "House").matches("Portuguese House"));
    }
    #[test]
    fn deduplicates_and_preserves_mix_for_queue_and_autoplay() {
        let track = |id: &str| Entry {
            id: id.into(),
            raw: json!({}),
            ..Default::default()
        };
        let tracks = interleave(
            vec![vec![track("1"), track("2")], vec![track("1"), track("3")]],
            Selection::new(1, 1).unwrap(),
        );
        assert_eq!(
            tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert!(tracks.iter().all(|t| t.raw[MARKER] == "1:1"));
        assert_eq!(tracks[0].wire()[MARKER], "1:1");
    }
    #[test]
    #[ignore = "requires a saved Deezer session and engine DLL; read-only"]
    fn live_available_genres() {
        let engine = Engine::load().unwrap();
        let secret = std::ffi::CString::new(crate::session::saved().unwrap()).unwrap();
        assert_ne!(unsafe { (engine.init)(secret.as_ptr()) }, 0);
        for (region, expected) in [(0, 0), (1, 1)] {
            let start = std::time::Instant::now();
            let data = available_genres(&engine, region).unwrap();
            let genres = data["genres"].as_array().unwrap();
            println!(
                "{}: {} verified genres in {:.1}s (partial: {})",
                REGIONS[region].0,
                genres.len(),
                start.elapsed().as_secs_f32(),
                data["partial"]
            );
            assert!(genres.iter().any(|genre| genre["id"] == expected));
            println!(
                "New styles: {}",
                genres
                    .iter()
                    .filter(|g| g["id"].as_u64().unwrap_or(0) >= 29)
                    .filter_map(|g| g["title"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    #[test]
    #[ignore = "requires a saved Deezer session and engine DLL; read-only"]
    fn live_expanded_style_samples() {
        let engine = Engine::load().unwrap();
        let secret = std::ffi::CString::new(crate::session::saved().unwrap()).unwrap();
        assert_ne!(unsafe { (engine.init)(secret.as_ptr()) }, 0);
        let mut found = 0;
        for (region, genre) in [
            ("South African", "Amapiano"),
            ("Mexican", "Cumbia"),
            ("Filipino / Tagalog", "Pop"),
            ("Azerbaijani", "Pop"),
            ("English", "Ambient"),
            ("Brazilian", "Bossa Nova"),
            ("Japanese", "Lo-fi"),
            ("Turkish", "Deep House"),
        ] {
            let selection = Selection::new(
                REGIONS.iter().position(|r| r.0 == region).unwrap(),
                GENRES.iter().position(|g| g.0 == genre).unwrap(),
            )
            .unwrap();
            match tracks(&engine, selection) {
                Ok(tracks) => {
                    found += 1;
                    println!("{}: {} playable tracks", selection.title(), tracks.len());
                }
                Err(error) => println!("{}: {error}", selection.title()),
            }
        }
        assert!(found > 0, "No sample returned playable tracks");
    }
    #[test]
    #[ignore = "requires a saved Deezer session and engine DLL; read-only"]
    fn live_country_genre_mixes() {
        let engine = Engine::load().unwrap();
        let secret = std::ffi::CString::new(crate::session::saved().unwrap()).unwrap();
        assert_ne!(unsafe { (engine.init)(secret.as_ptr()) }, 0);
        for selection in [Selection::new(0, 0).unwrap(), Selection::new(1, 1).unwrap()] {
            let tracks = tracks(&engine, selection).unwrap();
            assert!(!tracks.is_empty());
            assert!(tracks.iter().all(|t| t.raw[MARKER] == selection.key()));
            println!("{}: {} playable tracks", selection.title(), tracks.len());
        }
    }
}
