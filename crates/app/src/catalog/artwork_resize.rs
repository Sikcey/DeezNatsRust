// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Sonora Contributors.
// Adapted from Sonora b525b2a, crates/ui/src/artwork.rs::artwork_frame.
// https://github.com/sonorahq/sonora
// Changes: return DynamicImage before channel conversion; crop small covers too.
// See assets/licenses/sonora-GPL-3.0.txt and NOTICE.md.
use image::{imageops, DynamicImage};

pub(super) fn cover(image: DynamicImage, edge: u32) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    let side = width.min(height);
    let square = match width == height {
        true => image,
        false => image.crop_imm((width - side) / 2, (height - side) / 2, side, side),
    };
    match edge == 0 || side <= edge {
        true => square,
        false => match side > edge.saturating_mul(2) {
            true => square.thumbnail_exact(edge, edge),
            false => square.resize_exact(edge, edge, imageops::FilterType::Triangle),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn covers_only_downscale_and_small_sources_still_crop_to_square() {
        assert_eq!(cover(DynamicImage::new_rgb8(64, 32), 192).width(), 32);
        assert_eq!(cover(DynamicImage::new_rgb8(1000, 800), 192).height(), 192);
        assert_eq!(cover(DynamicImage::new_rgb8(256, 256), 192).width(), 192);
        assert_eq!(cover(DynamicImage::new_rgb8(128, 64), 0).width(), 64);
    }
}
