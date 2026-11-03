//! Per-listener MP3 encoding. Capture and host playback never wait for this work.
use mp3lame_encoder::{Bitrate, Builder, InterleavedPcm, Mode, Quality};

pub(super) struct Encoder {
    inner: mp3lame_encoder::Encoder,
    samples: Vec<i16>,
    output: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use symphonia::core::{audio::SampleBuffer, errors::Error, io::MediaSourceStream, probe::Hint};

    #[test]
    fn live_mp3_decodes_incremental_stereo_pcm_at_the_expected_rate() {
        let mut encoder = Encoder::new().unwrap();
        let frames = 44100 * 5;
        let pcm: Vec<u8> = (0..frames)
            .flat_map(|n| {
                [440., 1200.].into_iter().flat_map(move |hz| {
                    let sample =
                        ((n as f64 * hz * std::f64::consts::TAU / 44100.).sin() * 12000.) as i16;
                    sample.to_le_bytes()
                })
            })
            .collect();
        let mut encoded = Vec::new();
        // Vary packet sizes, including single stereo frames. Never flush:
        // a browser must decode the ongoing response before its connection ends.
        let mut offset = 0;
        for chunk_frames in [1, 137, 441, 882, 4096].into_iter().cycle() {
            let end = (offset + chunk_frames * 4).min(pcm.len());
            encoded.extend_from_slice(encoder.encode(&pcm[offset..end]).unwrap());
            offset = end;
            if offset == pcm.len() {
                break;
            }
        }
        assert!(
            (110_000..130_000).contains(&encoded.len()),
            "unexpected MP3 bandwidth: {}",
            encoded.len()
        );
        let source = MediaSourceStream::new(Box::new(Cursor::new(encoded)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("mp3");
        let mut format = symphonia::default::get_probe()
            .format(&hint, source, &Default::default(), &Default::default())
            .unwrap()
            .format;
        let track = format.default_track().unwrap();
        assert_eq!(track.codec_params.sample_rate, Some(44100));
        assert_eq!(track.codec_params.channels.unwrap().count(), 2);
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &Default::default())
            .unwrap();
        let mut samples = Vec::new();
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => panic!("invalid MP3 response: {e}"),
            };
            let decoded = decoder.decode(&packet).unwrap();
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            buffer.copy_interleaved_ref(decoded);
            samples.extend_from_slice(buffer.samples());
        }
        assert!(
            (44100 * 48 / 10..44100 * 51 / 10).contains(&(samples.len() / 2)),
            "stream lost audio duration"
        );
        let power = |channel: usize, hz: f64| {
            let (mut re, mut im) = (0., 0.);
            for (n, frame) in samples.chunks_exact(2).enumerate().skip(4410) {
                let phase = n as f64 * hz * std::f64::consts::TAU / 44100.;
                re += frame[channel] as f64 * phase.cos();
                im += frame[channel] as f64 * phase.sin();
            }
            re * re + im * im
        };
        assert!(
            power(0, 440.) > 100. * power(0, 1200.),
            "left channel corrupted"
        );
        assert!(
            power(1, 1200.) > 100. * power(1, 440.),
            "right channel corrupted"
        );
    }

    #[test]
    fn malformed_pcm_is_rejected_before_encoding() {
        let mut encoder = Encoder::new().unwrap();
        assert!(encoder.encode(&[0; 3]).is_err());
        assert!(encoder.encode(&[0; 4096]).is_ok());
    }
}

impl Encoder {
    pub(super) fn new() -> Result<Self, String> {
        let mut builder = Builder::new().ok_or("Could not allocate Connect MP3 encoder")?;
        let configured = (|| {
            builder.set_num_channels(2)?;
            builder.set_sample_rate(44100)?;
            builder.set_brate(Bitrate::Kbps192)?;
            builder.set_mode(Mode::JointStereo)?;
            builder.set_quality(Quality::NearBest)?;
            // Live responses cannot seek back to rewrite a duration/Xing tag.
            builder.set_to_write_vbr_tag(false)?;
            builder.build()
        })();
        let inner =
            configured.map_err(|e| format!("Could not initialize Connect MP3 encoder: {e:?}"))?;
        Ok(Self {
            inner,
            samples: Vec::new(),
            output: Vec::new(),
        })
    }

    pub(super) fn encode(&mut self, pcm: &[u8]) -> Result<&[u8], String> {
        if pcm.len() % 4 != 0 {
            return Err("Connect PCM is not aligned to stereo frames".into());
        }
        self.samples.clear();
        self.samples.extend(
            pcm.chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]])),
        );
        self.output.clear();
        self.output
            .reserve(mp3lame_encoder::max_required_buffer_size(
                self.samples.len() / 2,
            ));
        self.inner
            .encode_to_vec(InterleavedPcm(&self.samples), &mut self.output)
            .map_err(|e| format!("Connect MP3 encoding failed: {e:?}"))?;
        // Do not flush between callbacks: preserve the continuous codec timeline.
        Ok(&self.output)
    }
}
