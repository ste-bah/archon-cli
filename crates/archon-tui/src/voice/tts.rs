//! Speech synthesis — the output half of voice.
//!
//! Mirrors [`SttProvider`](super::stt::SttProvider): one trait, an HTTP
//! implementation, and a mock for tests. Nothing here decodes a container
//! format; providers are asked for raw PCM and hand back samples ready to play.
//!
//! # Why one HTTP provider covers Kokoro and OpenAI
//!
//! Kokoro-82M is served by `kokoro-fastapi`, which speaks the OpenAI
//! `/v1/audio/speech` protocol deliberately. So a single client reaches both: a
//! local Kokoro at `http://127.0.0.1:8880` with no key, or OpenAI with one. The
//! difference is a URL, a model name and a voice, which is configuration rather
//! than code.
//!
//! Kokoro is the default because the voices have to be worth listening to. It
//! is a neural model — 82M parameters, trained on real speech — not a formant
//! or concatenative synthesiser, and it does not sound like one.

use async_trait::async_trait;

/// Audio ready to play: mono `f32` samples and the rate they were made at.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Speech {
    /// How long this will take to say.
    #[must_use]
    pub fn duration(&self) -> std::time::Duration {
        if self.sample_rate == 0 {
            return std::time::Duration::ZERO;
        }
        std::time::Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.sample_rate))
    }
}

/// Trait implemented by all text-to-speech providers.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, text: &str) -> anyhow::Result<Speech>;
}

/// The sample rate `/v1/audio/speech` returns for `response_format: "pcm"`.
///
/// Fixed by the protocol on both implementations — OpenAI documents 24 kHz and
/// `kokoro-fastapi` matches it. It is not negotiable per request, so it is a
/// constant rather than something parsed out of a response that never says it.
pub const PCM_SAMPLE_RATE: u32 = 24_000;

/// Longest reply worth speaking, in characters.
///
/// Synthesis cost and the time spent listening both scale with length, and a
/// spoken wall of text cannot be skimmed the way the screen can. Past this the
/// reply is truncated at a sentence boundary rather than read out entire.
pub const MAX_SPOKEN_CHARS: usize = 1_000;

// ---------------------------------------------------------------------------
// OpenAI-compatible provider (Kokoro, OpenAI)
// ---------------------------------------------------------------------------

/// Speaks to any `/v1/audio/speech` endpoint.
pub struct OpenAiCompatibleTts {
    /// Base URL, without the path. `http://127.0.0.1:8880` for a local Kokoro.
    pub url: String,
    /// Empty for a local server that wants no authentication.
    pub api_key: String,
    /// `kokoro` locally, `tts-1` or `gpt-4o-mini-tts` at OpenAI.
    pub model: String,
    /// `af_heart` and friends for Kokoro, `alloy` and friends for OpenAI.
    pub voice: String,
}

#[async_trait]
impl TtsProvider for OpenAiCompatibleTts {
    async fn synthesize(&self, text: &str) -> anyhow::Result<Speech> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": self.voice,
            // Raw PCM rather than mp3 or wav: there is no decoder in this
            // crate, and asking for a container we would then have to parse is
            // a dependency bought for nothing.
            "response_format": "pcm",
        });

        let client = reqwest::Client::new();
        let mut request = client
            .post(format!(
                "{}/v1/audio/speech",
                self.url.trim_end_matches('/')
            ))
            .json(&body);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }

        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            // The body carries the reason — a bad voice name, an unloaded
            // model — and a bare status code sends the reader to the wrong
            // place.
            let detail = response.text().await.unwrap_or_default();
            anyhow::bail!("speech synthesis failed ({status}): {}", detail.trim());
        }

        let bytes = response.bytes().await?;
        Ok(Speech {
            samples: decode_pcm_s16le(&bytes),
            sample_rate: PCM_SAMPLE_RATE,
        })
    }
}

/// Decode signed 16-bit little-endian PCM into `f32` in [-1.0, 1.0].
///
/// A trailing odd byte is dropped: half a sample is not a sample, and keeping
/// it would shift every value after it by one byte.
#[must_use]
pub fn decode_pcm_s16le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / f32::from(i16::MAX))
        .collect()
}

/// Trim a reply to something worth listening to.
///
/// Cuts at the last sentence end inside the limit so the speech finishes a
/// thought rather than stopping mid-word; falls back to the hard limit on a
/// character boundary when there is no sentence end to find.
#[must_use]
pub fn spoken_excerpt(text: &str, limit: usize) -> String {
    let collapsed = text.trim();
    if collapsed.chars().count() <= limit {
        return collapsed.to_string();
    }
    let head: String = collapsed.chars().take(limit).collect();
    let cut = ['.', '!', '?', '\n']
        .iter()
        .filter_map(|end| head.rfind(*end))
        .max();
    match cut {
        // Only honour a sentence end that is not right at the start, or a reply
        // beginning "Yes. <long explanation>" would be spoken as "Yes."
        Some(index) if index > limit / 4 => head[..=index].trim().to_string(),
        _ => head.trim().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Mock provider (for tests)
// ---------------------------------------------------------------------------

/// Returns a fixed tone, so a test can assert on length without a server.
pub struct MockTts {
    pub sample_count: usize,
}

#[async_trait]
impl TtsProvider for MockTts {
    async fn synthesize(&self, _text: &str) -> anyhow::Result<Speech> {
        Ok(Speech {
            samples: vec![0.1; self.sample_count],
            sample_rate: PCM_SAMPLE_RATE,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_decodes_to_the_full_range() {
        let bytes = [0x00, 0x00, 0xff, 0x7f, 0x01, 0x80];
        let samples = decode_pcm_s16le(&bytes);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], 0.0);
        assert!((samples[1] - 1.0).abs() < 1e-6, "{samples:?}");
        assert!(samples[2] < -0.99, "{samples:?}");
    }

    /// Half a sample is not a sample, and keeping it would shift everything
    /// after it by a byte.
    #[test]
    fn a_trailing_odd_byte_is_dropped_rather_than_shifting_the_stream() {
        assert_eq!(decode_pcm_s16le(&[0x00, 0x00, 0x7f]).len(), 1);
        assert!(decode_pcm_s16le(&[0x7f]).is_empty());
        assert!(decode_pcm_s16le(&[]).is_empty());
    }

    #[test]
    fn duration_is_samples_over_rate() {
        let speech = Speech {
            samples: vec![0.0; 24_000],
            sample_rate: 24_000,
        };
        assert_eq!(speech.duration(), std::time::Duration::from_secs(1));
    }

    #[test]
    fn a_rateless_clip_has_no_duration_rather_than_dividing_by_zero() {
        let speech = Speech {
            samples: vec![0.0; 100],
            sample_rate: 0,
        };
        assert_eq!(speech.duration(), std::time::Duration::ZERO);
    }

    #[test]
    fn a_short_reply_is_spoken_whole() {
        assert_eq!(spoken_excerpt("  Done.  ", 100), "Done.");
    }

    /// Speech should finish a thought rather than stop mid-word.
    #[test]
    fn a_long_reply_is_cut_at_a_sentence_end() {
        // The sentence end sits at index 30, inside the 60-character window and
        // past the quarter-way guard, so it is the cut point.
        let text = format!("{}. {}", "a".repeat(30), "b".repeat(200));
        let spoken = spoken_excerpt(&text, 60);

        assert!(spoken.ends_with('.'), "cut mid-word: {spoken}");
        assert_eq!(spoken.chars().count(), 31, "{spoken}");
    }

    /// With no sentence end inside the window there is nothing to cut at, so
    /// the hard limit stands rather than the reply being dropped.
    #[test]
    fn a_long_reply_with_no_sentence_end_in_range_is_cut_at_the_limit() {
        let text = format!("{} and then more that will not fit", "a".repeat(50));
        let spoken = spoken_excerpt(&text, 60);

        // At most the limit, not exactly it: the cut can land on a space, and
        // the trailing space is trimmed.
        assert!(spoken.chars().count() <= 60, "{spoken}");
        assert!(spoken.chars().count() >= 55, "cut far too short: {spoken}");
        assert!(spoken.starts_with("aaaa"), "{spoken}");
    }

    /// "Yes. <two pages>" must not be spoken as "Yes."
    #[test]
    fn an_early_full_stop_does_not_swallow_the_answer() {
        let text = format!("Yes. {}", "b".repeat(400));
        let spoken = spoken_excerpt(&text, 100);
        assert!(
            spoken.chars().count() > 50,
            "the reply was cut to its first word: {spoken}"
        );
    }

    #[test]
    fn a_reply_with_no_sentence_end_is_cut_on_a_character_boundary() {
        let spoken = spoken_excerpt(&"é".repeat(500), 100);
        assert_eq!(spoken.chars().count(), 100);
    }
}
