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

    /// Returns true if the default audio input device is accessible.
    ///
    /// Requires the `audio-capture` feature (which links cpal/ALSA). When the
    /// feature is disabled (e.g. on WSL2 without libasound-dev), this returns
    /// false so the rest of the TUI still compiles and runs without audio.
    pub fn is_supported(&self) -> bool {
        #[cfg(feature = "audio-capture")]
        {
            cpal::default_host().input_devices().is_ok()
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
        let n = samples.len();
        if n == 0 {
            return false;
        }
        let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
        let rms = (sum_sq / n as f32).sqrt();
        rms > self.threshold
    }
}
