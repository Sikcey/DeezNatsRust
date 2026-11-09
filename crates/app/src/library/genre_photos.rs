//! Shared landscape photography for related genre families. No provider cover searches.
use super::Bitmap;
use std::{cell::RefCell, collections::VecDeque, sync::Arc};

pub(crate) const PREFIX: &str = "genre-photo:v2:";
pub(crate) const DECODE_EDGE: u32 = 768;

const PHOTOS: &[(&str, &[u8])] = &[
    (
        "acoustic",
        include_bytes!("../../../../assets/genre-photos/acoustic.jpg"),
    ),
    (
        "ambient",
        include_bytes!("../../../../assets/genre-photos/ambient.jpg"),
    ),
    (
        "blues",
        include_bytes!("../../../../assets/genre-photos/blues.jpg"),
    ),
    (
        "classical",
        include_bytes!("../../../../assets/genre-photos/classical.jpg"),
    ),
    (
        "electronic",
        include_bytes!("../../../../assets/genre-photos/electronic.jpg"),
    ),
    (
        "hiphop",
        include_bytes!("../../../../assets/genre-photos/hiphop.jpg"),
    ),
    (
        "jazz",
        include_bytes!("../../../../assets/genre-photos/jazz.jpg"),
    ),
    (
        "lofi",
        include_bytes!("../../../../assets/genre-photos/lofi.jpg"),
    ),
    (
        "percussion",
        include_bytes!("../../../../assets/genre-photos/percussion.jpg"),
    ),
    (
        "piano",
        include_bytes!("../../../../assets/genre-photos/piano.jpg"),
    ),
    (
        "pop",
        include_bytes!("../../../../assets/genre-photos/pop.jpg"),
    ),
    (
        "rock",
        include_bytes!("../../../../assets/genre-photos/rock.jpg"),
    ),
    (
        "soul",
        include_bytes!("../../../../assets/genre-photos/soul.jpg"),
    ),
    (
        "studio",
        include_bytes!("../../../../assets/genre-photos/studio.jpg"),
    ),
    (
        "tropical",
        include_bytes!("../../../../assets/genre-photos/tropical.jpg"),
    ),
];

pub(crate) fn key(genre: &str) -> String {
    let normalized = genre.to_lowercase().replace(['-', '_', '&'], " ");
    let has = |terms: &[&str]| terms.iter().any(|term| normalized.contains(term));
    let family = if has(&["lo fi", "lofi", "chillhop", "trip hop", "downtempo"]) {
        "lofi"
    } else if has(&["metal", "punk", "hardcore", "grind", "screamo"]) {
        "rock"
    } else if has(&["piano", "keyboard"]) {
        "piano"
    } else if has(&["jazz", "swing", "bebop", "bossa", "saxophone"]) {
        "jazz"
    } else if has(&["rhythm and blues"]) {
        "soul"
    } else if has(&["blues"]) {
        "blues"
    } else if has(&[
        "classical",
        "orchestr",
        "symphon",
        "opera",
        "baroque",
        "chamber",
        "violin",
        "cello",
        "choral",
    ]) {
        "classical"
    } else if has(&[
        "ambient",
        "chill",
        "meditat",
        "sleep",
        "new age",
        "soundscape",
        "drone",
        "nature",
    ]) {
        "ambient"
    } else if has(&[
        "reggae",
        "dancehall",
        "dub",
        "ska",
        "tropical",
        "caribbean",
        "reggaeton",
    ]) && !has(&["dubstep"])
    {
        "tropical"
    } else if has(&[
        "hip hop", "hiphop", "rap", "trap", "grime", "drill", "boom bap",
    ]) {
        "hiphop"
    } else if has(&["rnb", "r b", "rhythm", "soul", "funk", "gospel", "motown"]) {
        "soul"
    } else if has(&["rock", "grunge", "alternative", "shoegaze"]) {
        "rock"
    } else if has(&[
        "techno",
        "house",
        "electro",
        "edm",
        "trance",
        "dance",
        "disco",
        "synth",
        "garage",
        "drum and bass",
        "drum n bass",
        "jungle",
        "breakbeat",
        "dubstep",
        "new wave",
        "vaporwave",
        "retrowave",
        "chiptune",
        "eurobeat",
        "breakcore",
        "industrial",
    ]) {
        "electronic"
    } else if has(&["pop", "idol", "schlager", "eurovision"]) {
        "pop"
    } else if has(&[
        "folk",
        "country",
        "acoustic",
        "singer",
        "songwriter",
        "flamenco",
        "fado",
        "arabesk",
        "turk",
        "türk",
        "anadolu",
        "world",
        "oud",
        "baglama",
        "bağlama",
        "saz",
        "bluegrass",
        "americana",
        "celtic",
    ]) {
        "acoustic"
    } else if has(&[
        "latin",
        "salsa",
        "samba",
        "cumbia",
        "afro",
        "percussion",
        "drum",
    ]) {
        "percussion"
    } else if has(&["indie"]) {
        "rock"
    } else {
        "studio"
    };
    format!("{PREFIX}{family}")
}

pub(crate) fn bytes(key: &str) -> Option<&'static [u8]> {
    let family = key.strip_prefix(PREFIX)?;
    PHOTOS
        .iter()
        .find_map(|(name, bytes)| (*name == family).then_some(*bytes))
}

type CropKey = (u64, u32, u32);
thread_local! {
    static CROPS: RefCell<VecDeque<(CropKey, Arc<Bitmap>)>> = const { RefCell::new(VecDeque::new()) };
}

/// Cache a center crop at the actual button ratio; never stretch or crop to square.
pub(crate) fn cover(image: &Bitmap, width: u32, height: u32) -> Arc<Bitmap> {
    if image.width == 0 || image.height == 0 {
        return Arc::new(image.clone());
    }
    let (width, height) = (width.max(1), height.max(1));
    let key = (image.id, width, height);
    CROPS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(index) = cache.iter().position(|(candidate, _)| *candidate == key) {
            let entry = cache.remove(index).unwrap();
            let result = entry.1.clone();
            cache.push_back(entry);
            return result;
        }
        let (crop_width, crop_height) = if u64::from(image.width) * u64::from(height)
            > u64::from(image.height) * u64::from(width)
        {
            (
                ((u64::from(image.height) * u64::from(width) / u64::from(height)) as u32).max(1),
                image.height,
            )
        } else {
            (
                image.width,
                ((u64::from(image.width) * u64::from(height) / u64::from(width)) as u32).max(1),
            )
        };
        let left = (image.width - crop_width) / 2;
        let top = (image.height - crop_height) / 2;
        let mut pixels = Vec::with_capacity(crop_width as usize * crop_height as usize);
        for y in top..top + crop_height {
            let start = (y * image.width + left) as usize;
            pixels.extend_from_slice(&image.pixels[start..start + crop_width as usize]);
        }
        let crop = Arc::new(Bitmap::new(crop_width, crop_height, pixels));
        while cache.len() >= 64
            || cache
                .iter()
                .map(|(_, image)| image.pixels.len() * 4)
                .sum::<usize>()
                + crop.pixels.len() * 4
                > 32 * 1024 * 1024
        {
            if cache.pop_front().is_none() {
                break;
            }
        }
        cache.push_back((key, crop.clone()));
        crop
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn related_genres_share_photographs_across_providers() {
        assert_eq!(key("Deep House"), key("Techno"));
        assert_eq!(key("Jazz"), key("Bebop"));
        assert_eq!(key("Ambient"), key("Chillout"));
        assert_eq!(key("Dubstep"), key("Deep House"));
        assert_eq!(key("Türkçe Pop"), key("Pop"));
        assert_eq!(key("Turkish Rock"), key("Rock"));
        assert_eq!(key("Bluegrass"), key("Country"));
        assert_ne!(key("Jazz"), key("Hip Hop"));
        for genre in crate::spotify::flow::genres() {
            assert!(bytes(&key(genre)).is_some(), "Missing image for {genre}");
        }
    }

    #[test]
    fn bundled_photos_keep_high_resolution_landscape_dimensions_through_blur() {
        for (name, bytes) in PHOTOS {
            let source = image::load_from_memory(bytes).unwrap();
            assert_eq!((source.width(), source.height()), (960, 640), "{name}");
            let image = super::super::decode_background(bytes, DECODE_EDGE).unwrap();
            let image = crate::appearance::prepare_genre_photo(&image);
            assert_eq!((image.width, image.height), (768, 512), "{name}");
        }
    }

    #[test]
    fn cropping_preserves_button_ratio_and_reuses_pixels() {
        let pixels = (0..8).flat_map(|_| 0..32).map(|x| x as u32).collect();
        let image = Bitmap::new(32, 8, pixels);
        let crop = cover(&image, 180, 60);
        assert_eq!((crop.width, crop.height), (24, 8));
        assert_eq!(crop.pixels[0], 4);
        assert_eq!(crop.pixels[23], 27);
        assert!(Arc::ptr_eq(&crop, &cover(&image, 180, 60)));
        let wider = cover(&image, 400, 80);
        assert_eq!((wider.width, wider.height), (32, 6));
    }
}
