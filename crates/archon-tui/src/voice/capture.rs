/// Handles audio capture configuration.
pub struct AudioCapture {
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
        }
    }

    /// Returns true if there is a default audio input device to record from.
    ///
    /// Requires the `audio-capture` feature (which links cpal/ALSA). When the
    /// feature is disabled (e.g. on WSL2 without libasound-dev), this returns
    /// false so the rest of the TUI still compiles and runs without audio.
    ///
    /// The check is `default_input_device().is_some()`, not
    /// `input_devices().is_ok()`: enumerating devices succeeds on a machine
    /// with no microphone at all, so the older check answered "is there an
    /// audio subsystem" while its callers were asking "can I record".
    pub fn is_supported(&self) -> bool {
        #[cfg(feature = "audio-capture")]
        {
            use cpal::traits::HostTrait;
            cpal::default_host().default_input_device().is_some()
        }
        #[cfg(not(feature = "audio-capture"))]
        {
            false
        }
    }

    /// Encode a slice of f32 PCM samples into WAV bytes (32-bit IEEE float, mono, 16 kHz).
    ///
    /// The output is a complete, valid WAV file that begins with the "RIFF" header.
    pub fn encode_to_wav(&self, samples: &[f32]) -> Vec<u8> {
        // WAV header constants for IEEE float format (format tag 3)
        const FORMAT_TAG_FLOAT: u16 = 3;
        let num_channels: u16 = 1;
        let sample_rate: u32 = self.sample_rate;
        let bits_per_sample: u16 = 32;
        let block_align: u16 = num_channels * (bits_per_sample / 8);
        let byte_rate: u32 = sample_rate * u32::from(block_align);
        let data_size: u32 = samples.len() as u32 * u32::from(bits_per_sample / 8);
        // Total RIFF chunk size = 4 (WAVE) + 8 (fmt chunk header) + 16 (fmt chunk body) + 8 (data chunk header) + data
        let riff_size: u32 = 4 + 8 + 16 + 8 + data_size;

        let mut buf: Vec<u8> = Vec::with_capacity(44 + data_size as usize);

        // RIFF chunk descriptor
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&riff_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        // fmt sub-chunk
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes()); // sub-chunk size for PCM
        buf.extend_from_slice(&FORMAT_TAG_FLOAT.to_le_bytes());
        buf.extend_from_slice(&num_channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data sub-chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for &sample in samples {
            buf.extend_from_slice(&sample.to_le_bytes());
        }

        buf
    }
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Signal shaping
//
// A microphone hands back interleaved frames at whatever rate and channel
// count the device likes; STT wants 16 kHz mono. These three functions are the
// whole conversion, kept here rather than in the cpal backend so they compile
// and are tested in a default build — a machine with no soundcard can still
// prove the arithmetic is right.
// ---------------------------------------------------------------------------

/// Average interleaved frames down to one channel.
///
/// `to_f32` converts one device sample to `f32`; the cpal backend passes
/// `f32::from_sample` so an `i16` or `u16` device needs no separate code path,
/// and the tests pass the identity so they need no soundcard.
///
/// A trailing partial frame is averaged over the samples that are actually
/// there, because dropping it would splice a discontinuity into the audio.
pub fn mix_frames_to_mono<T: Copy, F: Fn(T) -> f32>(
    interleaved: &[T],
    channels: usize,
    to_f32: F,
) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.iter().map(|s| to_f32(*s)).collect();
    }
    interleaved
        .chunks(channels)
        .map(|frame| {
            let sum: f32 = frame.iter().map(|s| to_f32(*s)).sum();
            sum / frame.len() as f32
        })
        .collect()
}

/// Resample by linear interpolation.
///
/// Speech recognition is unbothered by the aliasing a proper band-limited
/// resampler would remove, and a 48 kHz → 16 kHz integer decimation — the
/// overwhelmingly common case — reduces to picking every third sample, which
/// this does exactly.
pub fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || from_rate == 0 || to_rate == 0 || input.len() < 2 {
        return input.to_vec();
    }
    let ratio = f64::from(from_rate) / f64::from(to_rate);
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let position = index as f64 * ratio;
        let left = position.floor() as usize;
        let fraction = (position - left as f64) as f32;
        let a = input[left];
        let b = *input.get(left + 1).unwrap_or(&a);
        out.push(a + (b - a) * fraction);
    }
    out
}

/// Root-mean-square level of a block, as a 0.0–1.0-ish amplitude.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Voice activity detector based on RMS energy threshold.
pub struct VoiceActivityDetector {
    pub threshold: f32,
}

impl VoiceActivityDetector {
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }

    /// Returns true if the RMS energy of `samples` exceeds the threshold.
    pub fn is_speech(&self, samples: &[f32]) -> bool {
        if samples.is_empty() {
            return false;
        }
        rms(samples) > self.threshold
    }
}

#[cfg(test)]
mod signal_tests {
    use super::*;

    #[test]
    fn stereo_frames_average_to_one_channel() {
        let interleaved = [1.0_f32, 0.0, 0.5, -0.5];
        assert_eq!(
            mix_frames_to_mono(&interleaved, 2, |s| s),
            vec![0.5_f32, 0.0]
        );
    }

    /// A device that hands back a half-finished frame must not shift every
    /// later sample into the wrong channel, and must not lose the frame.
    #[test]
    fn a_trailing_partial_frame_is_averaged_over_what_arrived() {
        let mono = mix_frames_to_mono(&[1.0_f32, 0.0, 0.4], 2, |s| s);
        assert_eq!(mono, vec![0.5_f32, 0.4]);
    }

    #[test]
    fn mono_input_passes_through_unchanged() {
        let samples = [0.1_f32, -0.2, 0.3];
        assert_eq!(mix_frames_to_mono(&samples, 1, |s| s), samples.to_vec());
    }

    /// The conversion closure is the only thing that knows the device format,
    /// so an integer device shares the mixing code rather than copying it.
    #[test]
    fn an_integer_device_shares_the_same_mixing_code() {
        let interleaved = [i16::MAX, 0_i16];
        let mono = mix_frames_to_mono(&interleaved, 2, |s| f32::from(s) / f32::from(i16::MAX));
        assert_eq!(mono.len(), 1);
        assert!((mono[0] - 0.5).abs() < 1e-6, "{mono:?}");
    }

    #[test]
    fn forty_eight_kilohertz_down_to_sixteen_keeps_every_third_sample() {
        let input: Vec<f32> = (0..9).map(|i| i as f32).collect();
        assert_eq!(resample_linear(&input, 48_000, 16_000), vec![0.0, 3.0, 6.0]);
    }

    #[test]
    fn a_matching_rate_is_not_resampled_at_all() {
        let input = vec![0.1_f32, 0.2, 0.3];
        assert_eq!(resample_linear(&input, 16_000, 16_000), input);
    }

    /// Interpolation, not nearest-neighbour: 44.1 kHz is not an integer
    /// multiple of 16 kHz and is what most USB microphones actually report.
    #[test]
    fn a_non_integer_ratio_interpolates_between_neighbours() {
        let input: Vec<f32> = (0..441).map(|i| i as f32).collect();
        let out = resample_linear(&input, 44_100, 16_000);
        assert_eq!(out.len(), 160);
        assert_eq!(out[0], 0.0);
        // 44100/16000 = 2.75625, so output sample 1 sits between input 2 and 3.
        assert!((out[1] - 2.75625).abs() < 1e-3, "{}", out[1]);
    }

    #[test]
    fn a_constant_signal_survives_resampling_unchanged() {
        let input = vec![0.25_f32; 480];
        let out = resample_linear(&input, 48_000, 16_000);
        assert_eq!(out.len(), 160);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn silence_has_no_level_and_full_scale_has_all_of_it() {
        assert_eq!(rms(&[0.0_f32; 64]), 0.0);
        assert!((rms(&[1.0_f32, -1.0, 1.0, -1.0]) - 1.0).abs() < 1e-6);
        assert_eq!(rms(&[]), 0.0);
    }

    /// `is_speech` and the meter must agree, or the overlay would show a level
    /// while the pipeline discards the recording as silence.
    #[test]
    fn the_detector_and_the_meter_read_the_same_signal() {
        let detector = VoiceActivityDetector::new(0.2);
        let quiet = [0.1_f32; 32];
        let loud = [0.3_f32; 32];
        assert!(!detector.is_speech(&quiet));
        assert!(detector.is_speech(&loud));
        assert!(rms(&quiet) < detector.threshold);
        assert!(rms(&loud) > detector.threshold);
    }
}
