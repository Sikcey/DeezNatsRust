//! Premultiplied source-over blending for cached UI rasters.

#[inline]
fn scalar_pixel(destination: u32, source: u32) -> u32 {
    let inverse = 255 - (source >> 24);
    let mut result = 0;
    for shift in [0, 8, 16, 24] {
        let product = ((destination >> shift) & 255) * inverse + 128;
        let background = (product + (product >> 8)) >> 8;
        result |= ((((source >> shift) & 255) + background).min(255)) << shift;
    }
    result
}

pub(super) fn blend_row(destination: &mut [u32], source: &[u32]) {
    assert_eq!(destination.len(), source.len());
    let mut index = 0;
    // SSE2 is part of the x86-64 baseline, with no runtime CPU requirement.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::*;
        let zero = _mm_setzero_si128();
        let full = _mm_set1_epi16(255);
        let rounding = _mm_set1_epi16(128);
        while index + 4 <= destination.len() {
            let foreground = _mm_loadu_si128(source.as_ptr().add(index).cast());
            let alpha = _mm_srli_epi32::<24>(foreground);
            if _mm_movemask_epi8(_mm_cmpeq_epi32(alpha, zero)) == 0xffff {
                index += 4;
                continue;
            }
            if _mm_movemask_epi8(_mm_cmpeq_epi32(alpha, _mm_set1_epi32(255))) == 0xffff {
                _mm_storeu_si128(destination.as_mut_ptr().add(index).cast(), foreground);
                index += 4;
                continue;
            }
            let background = _mm_loadu_si128(destination.as_ptr().add(index).cast());
            let blend = |foreground, background| {
                let alpha = _mm_shufflehi_epi16::<255>(_mm_shufflelo_epi16::<255>(foreground));
                let inverse = _mm_sub_epi16(full, alpha);
                let product = _mm_add_epi16(_mm_mullo_epi16(background, inverse), rounding);
                let scaled =
                    _mm_srli_epi16::<8>(_mm_add_epi16(product, _mm_srli_epi16::<8>(product)));
                _mm_add_epi16(foreground, scaled)
            };
            let low = blend(
                _mm_unpacklo_epi8(foreground, zero),
                _mm_unpacklo_epi8(background, zero),
            );
            let high = blend(
                _mm_unpackhi_epi8(foreground, zero),
                _mm_unpackhi_epi8(background, zero),
            );
            _mm_storeu_si128(
                destination.as_mut_ptr().add(index).cast(),
                _mm_packus_epi16(low, high),
            );
            index += 4;
        }
    }
    for (destination, source) in destination[index..].iter_mut().zip(&source[index..]) {
        *destination = scalar_pixel(*destination, *source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_blocks_mixed_blocks_and_unaligned_tails_match_scalar() {
        let source = [
            0x12345678, 0, 0, 0, 0, 0xff123456, 0xffabcdef, 0xff000000, 0xffffffff, 0, 0x80402010,
            0xff345678, 0x01010101, 0, 0xffabcdef, 0x40201008, 0,
        ];
        let original: Vec<u32> = (0..source.len())
            .map(|i| 0x80402010u32.wrapping_add(i as u32 * 0x010101))
            .collect();
        for len in 1..source.len() {
            let mut actual = original.clone();
            blend_row(&mut actual[1..1 + len], &source[1..1 + len]);
            for i in 0..source.len() {
                let expected = if (1..1 + len).contains(&i) {
                    scalar_pixel(original[i], source[i])
                } else {
                    original[i]
                };
                assert_eq!(actual[i], expected, "length {len}, pixel {i}");
            }
        }
    }

    #[test]
    fn vector_blending_matches_source_over_for_every_alpha_and_unaligned_tail() {
        let mut source = vec![0; 1031];
        let mut original = vec![0; source.len()];
        for (index, (source, destination)) in source.iter_mut().zip(&mut original).enumerate() {
            let alpha = index as u32 % 256;
            let destination_alpha = (index as u32 * 71) % 256;
            *source = (alpha << 24) | ((alpha / 2) << 16) | ((alpha / 3) << 8) | alpha;
            *destination = (destination_alpha << 24)
                | (destination_alpha << 16)
                | ((destination_alpha / 5) << 8)
                | (destination_alpha / 2);
        }
        let expected: Vec<_> = original
            .iter()
            .zip(&source)
            .map(|(&destination, &source)| scalar_pixel(destination, source))
            .collect();
        let mut actual = original.clone();
        blend_row(&mut actual[1..], &source[1..]);
        assert_eq!(&actual[1..], &expected[1..]);
        assert_eq!(actual[0], original[0]);
        assert_eq!(scalar_pixel(0x80402010, 0), 0x80402010);
        assert_eq!(scalar_pixel(0x80402010, 0xffabcdef), 0xffabcdef);
    }
}
