// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Sonora Contributors.
// Adapted from Sonora b525b2a, crates/ui/src/artwork.rs::artwork_frame.
// https://github.com/sonorahq/sonora
// Changes: return DynamicImage; borrow large crops; crop small covers too.
// See assets/licenses/sonora-GPL-3.0.txt and NOTICE.md.
use image::{imageops, DynamicImage};

pub(super) fn cover(image: DynamicImage, edge: u32) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    let side = width.min(height);
    if edge != 0 && side > edge {
        if width == height {
            return if side > edge.saturating_mul(2) {
                image.thumbnail_exact(edge, edge)
            } else {
                image.resize_exact(edge, edge, imageops::FilterType::Triangle)
            };
        }
        // Borrow the source crop: copying it first can allocate tens of MiB
        // just to produce a small thumbnail. Only allocate the resized output.
        let square = imageops::crop_imm(&image, (width - side) / 2, (height - side) / 2, side, side);
        return DynamicImage::ImageRgba8(if side > edge.saturating_mul(2) {
            imageops::thumbnail(&*square, edge, edge)
        } else {
            imageops::resize(&*square, edge, edge, imageops::FilterType::Triangle)
        });
    }
    let square = match width == height {
        true => image,
        false => image.crop_imm((width - side) / 2, (height - side) / 2, side, side),
    };
    square
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

    #[test]
    fn downscaled_cover_keeps_the_center_crop() {
        let image = image::RgbImage::from_fn(80, 40, |x, _| {
            if (20..60).contains(&x) { image::Rgb([20, 100, 200]) }
            else { image::Rgb([255, 0, 0]) }
        });
        for edge in [8, 32] {
            let result = cover(DynamicImage::ImageRgb8(image.clone()), edge).into_rgba8();
            assert_eq!(result.dimensions(), (edge, edge));
            assert!(result.pixels().all(|p| p.0 == [20, 100, 200, 255]));
        }
    }
}
