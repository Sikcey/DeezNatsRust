//! Bundled Style Mix flags and shared genre photographs. See assets/style-mix/sources.json.
const FILES: &[(&str, &[u8])] = &[
    (
        "flag-tr.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-tr.webp"
        )),
    ),
    (
        "flag-jp.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-jp.webp"
        )),
    ),
    (
        "flag-kr.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-kr.webp"
        )),
    ),
    (
        "flag-fr.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-fr.webp"
        )),
    ),
    (
        "flag-arab.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-arab.webp"
        )),
    ),
    (
        "flag-br.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-br.webp"
        )),
    ),
    (
        "flag-es.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-es.webp"
        )),
    ),
    (
        "flag-pt.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-pt.webp"
        )),
    ),
    (
        "flag-de.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-de.webp"
        )),
    ),
    (
        "flag-it.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-it.webp"
        )),
    ),
    (
        "flag-in.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-in.webp"
        )),
    ),
    (
        "flag-ir.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ir.webp"
        )),
    ),
    (
        "flag-gr.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-gr.webp"
        )),
    ),
    (
        "flag-ru.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ru.webp"
        )),
    ),
    (
        "flag-cn.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-cn.webp"
        )),
    ),
    (
        "flag-id.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-id.webp"
        )),
    ),
    (
        "flag-th.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-th.webp"
        )),
    ),
    (
        "flag-pl.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-pl.webp"
        )),
    ),
    (
        "flag-nl.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-nl.webp"
        )),
    ),
    (
        "flag-se.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-se.webp"
        )),
    ),
    (
        "flag-no.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-no.webp"
        )),
    ),
    (
        "flag-fi.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-fi.webp"
        )),
    ),
    (
        "flag-dk.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-dk.webp"
        )),
    ),
    (
        "flag-ua.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ua.webp"
        )),
    ),
    (
        "flag-ro.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ro.webp"
        )),
    ),
    (
        "flag-hu.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-hu.webp"
        )),
    ),
    (
        "flag-cz.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-cz.webp"
        )),
    ),
    (
        "flag-rs.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-rs.webp"
        )),
    ),
    (
        "flag-hr.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-hr.webp"
        )),
    ),
    (
        "flag-al.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-al.webp"
        )),
    ),
    (
        "flag-bd.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-bd.webp"
        )),
    ),
    (
        "flag-pk.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-pk.webp"
        )),
    ),
    (
        "flag-vn.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-vn.webp"
        )),
    ),
    (
        "flag-ph.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ph.webp"
        )),
    ),
    (
        "flag-my.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-my.webp"
        )),
    ),
    (
        "flag-np.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-np.webp"
        )),
    ),
    (
        "flag-il.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-il.webp"
        )),
    ),
    (
        "flag-az.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-az.webp"
        )),
    ),
    (
        "flag-ge.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ge.webp"
        )),
    ),
    (
        "flag-am.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-am.webp"
        )),
    ),
    (
        "flag-kurd.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-kurd.webp"
        )),
    ),
    (
        "flag-tz.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-tz.webp"
        )),
    ),
    (
        "flag-et.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-et.webp"
        )),
    ),
    (
        "flag-mx.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-mx.webp"
        )),
    ),
    (
        "flag-ar.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ar.webp"
        )),
    ),
    (
        "flag-co.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-co.webp"
        )),
    ),
    (
        "flag-cl.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-cl.webp"
        )),
    ),
    (
        "flag-pe.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-pe.webp"
        )),
    ),
    (
        "flag-cu.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-cu.webp"
        )),
    ),
    (
        "flag-do.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-do.webp"
        )),
    ),
    (
        "flag-pr.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-pr.webp"
        )),
    ),
    (
        "flag-jm.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-jm.webp"
        )),
    ),
    (
        "flag-ng.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ng.webp"
        )),
    ),
    (
        "flag-gh.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-gh.webp"
        )),
    ),
    (
        "flag-za.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-za.webp"
        )),
    ),
    (
        "flag-ma.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ma.webp"
        )),
    ),
    (
        "flag-dz.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-dz.webp"
        )),
    ),
    (
        "flag-tn.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-tn.webp"
        )),
    ),
    (
        "flag-eg.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-eg.webp"
        )),
    ),
    (
        "flag-lb.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-lb.webp"
        )),
    ),
    (
        "flag-is.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-is.webp"
        )),
    ),
    (
        "flag-sk.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-sk.webp"
        )),
    ),
    (
        "flag-si.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-si.webp"
        )),
    ),
    (
        "flag-bg.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-bg.webp"
        )),
    ),
    (
        "flag-lt.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-lt.webp"
        )),
    ),
    (
        "flag-lv.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-lv.webp"
        )),
    ),
    (
        "flag-ee.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ee.webp"
        )),
    ),
    (
        "flag-ie.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ie.webp"
        )),
    ),
    (
        "flag-gb-sct.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-gb-sct.webp"
        )),
    ),
    (
        "flag-gb-wls.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-gb-wls.webp"
        )),
    ),
    (
        "flag-basque.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-basque.webp"
        )),
    ),
    (
        "flag-catalan.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-catalan.webp"
        )),
    ),
    (
        "flag-gb-eng.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-gb-eng.webp"
        )),
    ),
    (
        "flag-us.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-us.webp"
        )),
    ),
    (
        "flag-gb.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-gb.webp"
        )),
    ),
    (
        "flag-ca.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ca.webp"
        )),
    ),
    (
        "flag-au.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-au.webp"
        )),
    ),
    (
        "flag-nz.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-nz.webp"
        )),
    ),
    (
        "flag-at.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-at.webp"
        )),
    ),
    (
        "flag-ch.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-ch.webp"
        )),
    ),
    (
        "flag-be.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/flag-be.webp"
        )),
    ),
    (
        "concert.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/concert.webp"
        )),
    ),
    (
        "studio.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/studio.webp"
        )),
    ),
    (
        "guitar.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/guitar.webp"
        )),
    ),
    (
        "piano.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/piano.webp"
        )),
    ),
    (
        "jazz.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/jazz.webp"
        )),
    ),
    (
        "vinyl.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/vinyl.webp"
        )),
    ),
    (
        "electronic.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/style-mix/electronic.webp"
        )),
    ),
];
const REGIONS: &[(&str, &str)] = &[
    ("Turkish", "flag-tr.webp"),
    ("Japanese", "flag-jp.webp"),
    ("Korean", "flag-kr.webp"),
    ("French", "flag-fr.webp"),
    ("Arabic", "flag-arab.webp"),
    ("Brazilian", "flag-br.webp"),
    ("Spanish", "flag-es.webp"),
    ("Portuguese", "flag-pt.webp"),
    ("German", "flag-de.webp"),
    ("Italian", "flag-it.webp"),
    ("Hindi", "flag-in.webp"),
    ("Punjabi", "flag-in.webp"),
    ("Persian", "flag-ir.webp"),
    ("Greek", "flag-gr.webp"),
    ("Russian", "flag-ru.webp"),
    ("Chinese", "flag-cn.webp"),
    ("Indonesian", "flag-id.webp"),
    ("Thai", "flag-th.webp"),
    ("Polish", "flag-pl.webp"),
    ("Dutch", "flag-nl.webp"),
    ("Swedish", "flag-se.webp"),
    ("Norwegian", "flag-no.webp"),
    ("Finnish", "flag-fi.webp"),
    ("Danish", "flag-dk.webp"),
    ("Ukrainian", "flag-ua.webp"),
    ("Romanian", "flag-ro.webp"),
    ("Hungarian", "flag-hu.webp"),
    ("Czech", "flag-cz.webp"),
    ("Serbian", "flag-rs.webp"),
    ("Croatian", "flag-hr.webp"),
    ("Albanian", "flag-al.webp"),
    ("Bengali", "flag-bd.webp"),
    ("Tamil", "flag-in.webp"),
    ("Telugu", "flag-in.webp"),
    ("Urdu", "flag-pk.webp"),
    ("Vietnamese", "flag-vn.webp"),
    ("Filipino / Tagalog", "flag-ph.webp"),
    ("Malay", "flag-my.webp"),
    ("Nepali", "flag-np.webp"),
    ("Hebrew", "flag-il.webp"),
    ("Azerbaijani", "flag-az.webp"),
    ("Georgian", "flag-ge.webp"),
    ("Armenian", "flag-am.webp"),
    ("Kurdish", "flag-kurd.webp"),
    ("Swahili", "flag-tz.webp"),
    ("Amharic", "flag-et.webp"),
    ("Mexican", "flag-mx.webp"),
    ("Argentinian", "flag-ar.webp"),
    ("Colombian", "flag-co.webp"),
    ("Chilean", "flag-cl.webp"),
    ("Peruvian", "flag-pe.webp"),
    ("Cuban", "flag-cu.webp"),
    ("Dominican", "flag-do.webp"),
    ("Puerto Rican", "flag-pr.webp"),
    ("Jamaican", "flag-jm.webp"),
    ("Nigerian", "flag-ng.webp"),
    ("Ghanaian", "flag-gh.webp"),
    ("South African", "flag-za.webp"),
    ("Moroccan", "flag-ma.webp"),
    ("Algerian", "flag-dz.webp"),
    ("Tunisian", "flag-tn.webp"),
    ("Egyptian", "flag-eg.webp"),
    ("Lebanese", "flag-lb.webp"),
    ("Icelandic", "flag-is.webp"),
    ("Slovak", "flag-sk.webp"),
    ("Slovenian", "flag-si.webp"),
    ("Bulgarian", "flag-bg.webp"),
    ("Lithuanian", "flag-lt.webp"),
    ("Latvian", "flag-lv.webp"),
    ("Estonian", "flag-ee.webp"),
    ("Irish", "flag-ie.webp"),
    ("Scottish", "flag-gb-sct.webp"),
    ("Welsh", "flag-gb-wls.webp"),
    ("Basque", "flag-basque.webp"),
    ("Catalan", "flag-catalan.webp"),
    ("English", "flag-gb-eng.webp"),
    ("American", "flag-us.webp"),
    ("British", "flag-gb.webp"),
    ("Canadian", "flag-ca.webp"),
    ("Australian", "flag-au.webp"),
    ("New Zealand", "flag-nz.webp"),
    ("Austrian", "flag-at.webp"),
    ("Swiss", "flag-ch.webp"),
    ("Belgian", "flag-be.webp"),
];
pub(crate) fn key(region: bool, index: usize) -> Option<String> {
    if !region {
        let genre = super::GENRES.get(index)?.0;
        return Some(format!(
            "style-mix:{}",
            crate::catalog::genre_photos::key(genre)
        ));
    }
    let name = super::REGIONS.get(index)?.0;
    REGIONS
        .iter()
        .find(|(title, _)| *title == name)
        .map(|(_, file)| format!("style-mix:{file}"))
}

pub(crate) fn data(key: &str) -> Option<&'static [u8]> {
    let file = key.strip_prefix("style-mix:")?;
    if file.starts_with(crate::catalog::genre_photos::PREFIX) {
        return crate::catalog::genre_photos::bytes(file);
    }
    FILES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, bytes)| *bytes)
}

pub(crate) fn http_data(path: &str) -> Option<&'static [u8]> {
    let (kind, index) = path
        .strip_prefix("/style-mix/")?
        .strip_suffix(".webp")?
        .split_once('/')?;
    let region = match kind {
        "region" => true,
        "genre" => false,
        _ => return None,
    };
    data(&key(region, index.parse().ok()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn style_mix_artwork_is_bundled_decodable_and_complete() {
        for region in [true, false] {
            let total = if region {
                super::super::REGIONS.len()
            } else {
                super::super::GENRES.len()
            };
            for index in 0..total {
                let key = key(region, index).expect("every option has artwork");
                let bytes = data(&key).unwrap();
                let image = image::load_from_memory(bytes).unwrap();
                if region {
                    assert!(image.width() <= 320 && image.height() <= 240);
                } else {
                    let flow_key = crate::catalog::genre_photos::key(super::super::GENRES[index].0);
                    assert_eq!(Some(bytes), crate::catalog::genre_photos::bytes(&flow_key));
                }
                let kind = if region { "region" } else { "genre" };
                assert_eq!(
                    http_data(&format!("/style-mix/{kind}/{index}.webp")),
                    Some(bytes)
                );
            }
            assert!(key(region, total).is_none());
        }
        assert!(FILES.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() < 200_000);
        assert!(http_data("/style-mix/genre/../../sources.json").is_none());
        assert!(http_data("/style-mix/region/99999.webp").is_none());
        assert!(data("style-mix:../secret").is_none());
    }
    #[test]
    fn style_mix_related_genres_share_photos() {
        let key_for = |name| {
            key(
                false,
                super::super::GENRES
                    .iter()
                    .position(|g| g.0 == name)
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(key_for("Rock"), key_for("Indie Rock"));
        assert_eq!(key_for("House"), key_for("Deep House"));
        assert_ne!(key_for("Rock"), key_for("Classical"));
    }
}
