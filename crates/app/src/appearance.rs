//! Bounded background preparation, independent of UI and audio threads.
use crate::catalog::Bitmap;
use std::sync::{mpsc, Arc};

pub const PREPARE_LIMIT: u32 = 512;

pub struct Request {
    pub key: String,
    pub source: Arc<Bitmap>,
    pub blur: u32,
}
pub struct Worker {
    pub requests: mpsc::SyncSender<Request>,
    pub updates: mpsc::Receiver<(String, Arc<Bitmap>)>,
}
impl Worker {
    pub fn start() -> Self {
        let (requests, rx) = mpsc::sync_channel::<Request>(1);
        let (tx, updates) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(mut request) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                let texture = prepare(&request.source, request.blur);
                if tx.send((request.key, Arc::new(texture))).is_err() {
                    break;
                }
            }
        });
        Self { requests, updates }
    }
}
pub fn prepare(source: &Bitmap, blur: u32) -> Bitmap {
    if blur == 0 {
        return source.clone();
    }
    if source.width == 0 || source.height == 0 {
        return source.clone();
    }
    // Keep packed DIB pixels throughout. Three running-sum box passes approximate
    // a Gaussian in linear time, independent of the requested blur radius.
    let (w, h, mut pixels) = downscale(source, PREPARE_LIMIT);
    let scale = w.max(h) as f64 / source.width.max(source.height) as f64;
    let sigma = (blur.min(100) as f64 * 0.35 * scale).max(0.5);
    let ideal = (4.0 * sigma * sigma + 1.0).sqrt();
    let lower = (ideal.floor() as usize).max(1) | 1;
    let lower = if lower as f64 > ideal {
        lower.saturating_sub(2).max(1)
    } else {
        lower
    };
    let upper = lower + 2;
    let l = lower as f64;
    let lower_passes = ((12.0 * sigma * sigma - 3.0 * l * l - 12.0 * l - 9.0) / (-4.0 * l - 4.0))
        .round()
        .clamp(0.0, 3.0) as usize;
    // At the minimum sigma, rounding must not turn a requested blur into a no-op.
    let lower_passes = if lower == 1 {
        lower_passes.min(2)
    } else {
        lower_passes
    };
    let mut scratch = vec![0; pixels.len()];
    let mut columns = vec![0u64; w as usize];
    for pass in 0..3 {
        let radius = (if pass < lower_passes { lower } else { upper }) / 2;
        if radius != 0 {
            box_blur_horizontal(&pixels, &mut scratch, w as usize, radius);
            box_blur_vertical(
                &scratch,
                &mut pixels,
                w as usize,
                h as usize,
                radius,
                &mut columns,
            );
        }
    }
    Bitmap::new(w, h, pixels)
}

#[cfg(test)]
fn channels(pixel: u32) -> [u32; 3] {
    [pixel & 255, (pixel >> 8) & 255, (pixel >> 16) & 255]
}

// Three independent 21-bit lanes allow RGB running sums with one integer add.
#[inline]
fn packed_channels(pixel: u32) -> u64 {
    (pixel & 255) as u64
        | ((((pixel >> 8) & 255) as u64) << 21)
        | ((((pixel >> 16) & 255) as u64) << 42)
}

#[inline]
fn unpack_sum(sum: u64) -> [u32; 3] {
    [
        (sum & 0x1fffff) as u32,
        ((sum >> 21) & 0x1fffff) as u32,
        (sum >> 42) as u32,
    ]
}

struct Average {
    rounding: u32,
    reciprocal: u64,
}
impl Average {
    fn new(count: u32) -> Self {
        // 40 fractional bits make the rounded reciprocal exact for all channel
        // sums here (at most 8192 samples), without per-channel integer division.
        debug_assert!(count > 0 && count <= 8192);
        Self {
            rounding: count / 2,
            reciprocal: (1u64 << 40).div_ceil(count as u64),
        }
    }
    #[inline]
    fn channel(&self, sum: u32) -> u32 {
        (((sum + self.rounding) as u64 * self.reciprocal) >> 40) as u32
    }
    #[inline]
    fn pixel(&self, sum: [u32; 3]) -> u32 {
        0xff000000
            | self.channel(sum[0])
            | (self.channel(sum[1]) << 8)
            | (self.channel(sum[2]) << 16)
    }
}

fn downscale(source: &Bitmap, limit: u32) -> (u32, u32, Vec<u32>) {
    let max_dim = source.width.max(source.height);
    if max_dim <= limit {
        return (
            source.width,
            source.height,
            source.pixels.iter().map(|p| p | 0xff000000).collect(),
        );
    }
    let dimension =
        |n: u32| ((n as u64 * limit as u64 + max_dim as u64 / 2) / max_dim as u64).max(1) as u32;
    let (w, h) = (dimension(source.width), dimension(source.height));
    let mut pixels = Vec::with_capacity((w * h) as usize);
    // Retain the existing sampling grid, with integer coordinates computed once
    // per column. Preparation stays bounded even for very large source images.
    let x_lut: Vec<_> = (0..w)
        .map(|x| (x as u64 * max_dim as u64 / limit as u64).min(source.width as u64 - 1) as usize)
        .collect();
    for y in 0..h {
        let sy = (y as u64 * max_dim as u64 / limit as u64).min(source.height as u64 - 1) as usize;
        let row = sy * source.width as usize;
        for &sx in &x_lut {
            pixels.push(source.pixels[row + sx] | 0xff000000);
        }
    }
    (w, h, pixels)
}

fn box_blur_horizontal(source: &[u32], dest: &mut [u32], width: usize, radius: usize) {
    let average = Average::new((radius * 2 + 1) as u32);
    for (row, out) in source.chunks_exact(width).zip(dest.chunks_exact_mut(width)) {
        let mut sum = packed_channels(row[0]) * (radius + 1) as u64;
        for x in 1..=radius {
            sum += packed_channels(row[x.min(width - 1)]);
        }
        for (x, pixel) in out.iter_mut().enumerate() {
            *pixel = average.pixel(unpack_sum(sum));
            let remove = packed_channels(row[x.saturating_sub(radius)]);
            let add = packed_channels(row[(x + radius + 1).min(width - 1)]);
            sum = sum - remove + add;
        }
    }
}

fn box_blur_vertical(
    source: &[u32],
    dest: &mut [u32],
    width: usize,
    height: usize,
    radius: usize,
    columns: &mut [u64],
) {
    let average = Average::new((radius * 2 + 1) as u32);
    for (sum, &pixel) in columns.iter_mut().zip(&source[..width]) {
        *sum = packed_channels(pixel) * (radius + 1) as u64;
    }
    for y in 1..=radius {
        let start = y.min(height - 1) * width;
        for (sum, &pixel) in columns.iter_mut().zip(&source[start..start + width]) {
            *sum += packed_channels(pixel);
        }
    }
    // Traverse rows contiguously instead of walking strided columns.
    for (y, out) in dest.chunks_exact_mut(width).enumerate() {
        let remove = y.saturating_sub(radius) * width;
        let add = (y + radius + 1).min(height - 1) * width;
        for (x, (pixel, sum)) in out.iter_mut().zip(columns.iter_mut()).enumerate() {
            *pixel = average.pixel(unpack_sum(*sum));
            *sum = *sum - packed_channels(source[remove + x]) + packed_channels(source[add + x]);
        }
    }
}
pub fn cover_sample(image: &Bitmap, x: u32, y: u32, width: u32, height: u32) -> u32 {
    let scale = (width as f64 / image.width as f64).max(height as f64 / image.height as f64);
    let sx = ((x as f64 + (image.width as f64 * scale - width as f64) / 2.0) / scale) as u32;
    let sy = ((y as f64 + (image.height as f64 * scale - height as f64) / 2.0) / scale) as u32;
    image.pixels[(sy.min(image.height - 1) * image.width + sx.min(image.width - 1)) as usize]
}
// Quantized dominant color from at most 4096 pixels; never scans artwork per frame.
pub fn artwork_color(image: &Bitmap) -> u32 {
    let mut bins = [[0u64; 4]; 512];
    let step = image.pixels.len().div_ceil(4096).max(1);
    for &pixel in image.pixels.iter().step_by(step) {
        if pixel >> 24 < 128 {
            continue;
        }
        let r = (pixel >> 16) & 255;
        let g = (pixel >> 8) & 255;
        let b = pixel & 255;
        let hi = r.max(g).max(b);
        let lo = r.min(g).min(b);
        if hi < 35 || lo > 235 {
            continue;
        }
        let weight = (hi - lo + 16) as u64;
        let bin = &mut bins[((r >> 5) * 64 + (g >> 5) * 8 + (b >> 5)) as usize];
        bin[0] += weight;
        bin[1] += r as u64 * weight;
        bin[2] += g as u64 * weight;
        bin[3] += b as u64 * weight;
    }
    let bin = bins.iter().max_by_key(|bin| bin[0]).unwrap();
    if bin[0] == 0 {
        return 0x00a08060;
    }
    ((bin[1] / bin[0]) | ((bin[2] / bin[0]) << 8) | ((bin[3] / bin[0]) << 16)) as u32
}

pub fn artwork_luminance(image: &Bitmap) -> f32 {
    if image.pixels.is_empty() {
        return 0.15;
    }
    let step = image.pixels.len().div_ceil(1024).max(1);
    let mut total_lum = 0.0f32;
    let mut count = 0usize;
    for &pixel in image.pixels.iter().step_by(step) {
        if pixel >> 24 < 128 {
            continue;
        }
        let r = ((pixel >> 16) & 255) as f32 / 255.0;
        let g = ((pixel >> 8) & 255) as f32 / 255.0;
        let b = (pixel & 255) as f32 / 255.0;
        total_lum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
        count += 1;
    }
    if count == 0 {
        0.15
    } else {
        total_lum / count as f32
    }
}

pub fn colorref_luminance(color: u32) -> f32 {
    let r = (color & 255) as f32 / 255.0;
    let g = ((color >> 8) & 255) as f32 / 255.0;
    let b = ((color >> 16) & 255) as f32 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Shared desktop/browser text protection for the actual backdrop lightness.
pub fn readable_palette(mut palette: [u32; 6], light: bool, brightness: u32) -> [u32; 6] {
    let rgb = |r:u32,g:u32,b:u32| r | (g<<8) | (b<<16);
        if light {
            for color in &mut palette[..4] {
                let mut adjusted = 0;
                for shift in [0, 8, 16] {
                    adjusted |=
                        (((*color >> shift) & 255) * brightness / 100).min(255) << shift;
                }
                *color = adjusted;
            }
            let text_lum = crate::appearance::colorref_luminance(palette[4]);
            if text_lum > 0.30 {
                palette[4] = rgb(20, 20, 24);
            }
            let muted_lum = crate::appearance::colorref_luminance(palette[5]);
            if muted_lum > 0.52 || muted_lum < 0.15 {
                palette[5] = rgb(94, 102, 118);
            }
        } else {
            let text_lum = crate::appearance::colorref_luminance(palette[4]);
            if text_lum < 0.70 {
                palette[4] = rgb(248, 248, 248);
            }
            let muted_lum = crate::appearance::colorref_luminance(palette[5]);
            if muted_lum < 0.38 || muted_lum > 0.85 {
                palette[5] = rgb(190, 190, 190);
            }
        }
    palette
}

pub fn adaptive_color(seed: u32, light: bool, key: &str) -> Option<u32> {
    let mix = |base: u32, percent: u32| {
        let mut color = 0;
        for shift in [0, 8, 16] {
            color |= ((((seed >> shift) & 255) * percent
                + ((base >> shift) & 255) * (100 - percent))
                / 100)
                << shift;
        }
        color
    };
    Some(match key {
        "Background" => mix(
            if light { 0x00ffffff } else { 0x000c0c0c },
            if light { 10 } else { 12 },
        ),
        "Panel" => mix(
            if light { 0x00ffffff } else { 0x00191919 },
            if light { 7 } else { 16 },
        ),
        "Raised" => mix(
            if light { 0x00ffffff } else { 0x002c2c2c },
            if light { 17 } else { 22 },
        ),
        "Border" => mix(if light { 0x00aaaaaa } else { 0x00505050 }, 24),
        "Text" | "Icon" => {
            if light {
                0x00141418
            } else {
                0x00f5f5f5
            }
        }
        "Muted" => {
            if light {
                0x00555555
            } else {
                0x00bcbcbc
            }
        }
        "Accent" | "GradientStart" => mix(0, 40),
        "GradientEnd" => mix(0, 28),
        "AccentText" => {
            let accent = mix(0, 40);
            if colorref_luminance(accent) > 0.45 {
                0x00141418
            } else {
                0x00ffffff
            }
        }
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_palette_guards_repair_unreadable_text_without_changing_surfaces() {
        let dark=[0x00101010,0x00202020,0x00303030,0x00404040,0x00000000,0x00ffffff];
        let guarded=readable_palette(dark,false,100);
        assert_eq!(&guarded[..4],&dark[..4]);
        assert_eq!(guarded[4],0x00f8f8f8);
        assert_eq!(guarded[5],0x00bebebe);
        let light=[0x00ffffff,0x00eeeeee,0x00dddddd,0x00cccccc,0x00ffffff,0x00000000];
        let guarded=readable_palette(light,true,100);
        assert_eq!(&guarded[..4],&light[..4]);
        assert_eq!(guarded[4],0x00181414);
        assert_eq!(guarded[5],0x0076665e);
    }

    #[test]
    fn downscale_keeps_bounds_and_extreme_aspect_ratios() {
        let reduced = prepare(&Bitmap::new(1024, 1024, vec![0xff808080; 1024 * 1024]), 40);
        assert_eq!((reduced.width, reduced.height), (PREPARE_LIMIT, PREPARE_LIMIT));
        assert!(reduced.pixels.iter().all(|&p| p == 0xff808080));
        for (w, h) in [(1, 6000), (6000, 1), (1, 1), (PREPARE_LIMIT, PREPARE_LIMIT), (PREPARE_LIMIT + 1, 96)] {
            let source = Bitmap::new(w, h, vec![0xff2878c8; (w * h) as usize]);
            let result = prepare(&source, 100);
            assert!(result.width > 0 && result.height > 0);
            assert!(result.width.max(result.height) <= PREPARE_LIMIT);
            assert_eq!(result.pixels.len(), (result.width * result.height) as usize);
            assert!(result.pixels.iter().all(|&p| p == 0xff2878c8));
        }
    }

    #[test]
    fn running_blur_matches_clamped_spatial_average_on_thin_images() {
        let average = |sum: [u32; 3], count: u32| {
            0xff000000
                | ((sum[0] + count / 2) / count)
                | (((sum[1] + count / 2) / count) << 8)
                | (((sum[2] + count / 2) / count) << 16)
        };
        for count in 1..=71 {
            let reciprocal = Average::new(count);
            for sum in 0..=255 * count {
                assert_eq!(reciprocal.channel(sum), (sum + count / 2) / count);
            }
        }
        for (w, h) in [(1, 1), (1, 7), (9, 1), (7, 5)] {
            let source: Vec<u32> = (0..w * h)
                .map(|i| 0xff000000 | (i as u32 * 0x030507))
                .collect();
            for radius in [1, 3, 12] {
                let mut horizontal = vec![0; source.len()];
                let mut actual = vec![0; source.len()];
                box_blur_horizontal(&source, &mut horizontal, w, radius);
                box_blur_vertical(&horizontal, &mut actual, w, h, radius, &mut vec![0u64; w]);
                for y in 0..h {
                    for x in 0..w {
                        let mut horizontal_sum = [0u32; 3];
                        for k in -(radius as isize)..=radius as isize {
                            let sample_x = (x as isize + k).clamp(0, w as isize - 1) as usize;
                            let c = channels(source[y * w + sample_x]);
                            for i in 0..3 {
                                horizontal_sum[i] += c[i];
                            }
                        }
                        let mut vertical_sum = [0u32; 3];
                        for k in -(radius as isize)..=radius as isize {
                            let sample_y = (y as isize + k).clamp(0, h as isize - 1) as usize;
                            let mut horizontal_sum = [0u32; 3];
                            for j in -(radius as isize)..=radius as isize {
                                let sample_x = (x as isize + j).clamp(0, w as isize - 1) as usize;
                                let c = channels(source[sample_y * w + sample_x]);
                                for i in 0..3 {
                                    horizontal_sum[i] += c[i];
                                }
                            }
                            let c = channels(average(horizontal_sum, (radius * 2 + 1) as u32));
                            for i in 0..3 {
                                vertical_sum[i] += c[i];
                            }
                        }
                        assert_eq!(
                            actual[y * w + x],
                            average(vertical_sum, (radius * 2 + 1) as u32)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn gaussian_approximation_preserves_a_smooth_color_gradient() {
        let detail = Bitmap::new(2, 1, vec![0xff000000, 0xffffffff]);
        assert_ne!(prepare(&detail, 1).pixels, detail.pixels);
        let source = Bitmap::new(
            64,
            64,
            (0..64 * 64)
                .map(|i| 0xff000000 | ((i % 64 * 4) << 16) | ((i / 64 * 4) << 8) | 120)
                .collect(),
        );
        let rgba = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba([(x * 4) as u8, (y * 4) as u8, 120, 255])
        });
        for blur in [1, 40, 70, 100] {
            let actual = prepare(&source, blur);
            let gaussian = image::imageops::blur(&rgba, (blur as f32 * 0.35).max(0.5));
            let error: f64 = actual
                .pixels
                .iter()
                .zip(gaussian.pixels())
                .map(|(&p, q)| {
                    let dr = ((p >> 16) & 255) as f64 - q[0] as f64;
                    let dg = ((p >> 8) & 255) as f64 - q[1] as f64;
                    (dr * dr + dg * dg) / 2.0
                })
                .sum::<f64>()
                / actual.pixels.len() as f64;
            assert!(error.sqrt() < 16.0, "blur={blur}, RMSE={}", error.sqrt());
        }
        assert_eq!(
            prepare(&source, 100).pixels,
            prepare(&source, u32::MAX).pixels
        );
    }
    #[test]
    fn artwork_palettes_preserve_hue_and_readable_text_in_both_modes() {
        let image = Bitmap::new(64, 64, vec![0xffd04020; 4096]);
        assert_eq!(artwork_color(&image), 0x002040d0);
        let luminance = |color: u32| {
            [0, 8, 16]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(shift, w)| {
                    let c = ((color >> shift) & 255) as f64 / 255.0;
                    w * if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
                })
                .sum::<f64>()
        };
        for seed in [0, 0xffffff, 0x0000ff, 0x00ff00, 0xff0000, 0x2030d0] {
            for light in [false, true] {
                for surface in ["Background", "Panel", "Raised"] {
                    let a = luminance(adaptive_color(seed, light, surface).unwrap());
                    let b = luminance(adaptive_color(seed, light, "Text").unwrap());
                    assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
                }
                let accent = luminance(adaptive_color(seed, light, "Accent").unwrap());
                assert!((1.0 + 0.05) / (accent + 0.05) >= 4.5);
            }
        }
        assert_eq!(artwork_color(&Bitmap::new(2, 2, vec![0; 4])), 0x00a08060);
    }
    #[test]
    fn artwork_and_colorref_luminance_accurately_detect_brightness() {
        let dark_image = Bitmap::new(4, 4, vec![0xff101010; 16]);
        assert!(artwork_luminance(&dark_image) < 0.15);
        let light_image = Bitmap::new(4, 4, vec![0xfff5f5f5; 16]);
        assert!(artwork_luminance(&light_image) > 0.85);

        assert!(colorref_luminance(0x00ffffff) > 0.95);
        assert!(colorref_luminance(0x00000000) < 0.05);
        assert!(colorref_luminance(0x00141418) < 0.15);
    }
    #[test]
    fn cover_crops_the_center_instead_of_stretching() {
        let image = Bitmap {
            id: 0,
            width: 4,
            height: 2,
            pixels: vec![0, 1, 2, 3, 4, 5, 6, 7],
        };
        assert_eq!(cover_sample(&image, 0, 0, 2, 2), 1);
        assert_eq!(cover_sample(&image, 1, 1, 2, 2), 6);
    }
    #[test]
    fn blur_preserves_uniform_color_and_zero_preserves_detail() {
        let image = Bitmap {
            id: 1,
            width: 8,
            height: 8,
            pixels: vec![0xff2878c8; 64],
        };
        assert!(prepare(&image, 100).pixels.iter().all(|p| *p == 0xff2878c8));
        let detail = Bitmap {
            id: 2,
            width: 2,
            height: 1,
            pixels: vec![0xff000000, 0xffffffff],
        };
        assert_eq!(prepare(&detail, 0).pixels, detail.pixels);
    }
}
