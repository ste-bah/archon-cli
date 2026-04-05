use async_trait::async_trait;

/// Trait implemented by all speech-to-text providers.
#[async_trait]
pub trait SttProvider: Send + Sync {
    async fn transcribe(&self, wav_bytes: &[u8]) -> anyhow::Result<String>;
}

// ---------------------------------------------------------------------------
// OpenAI Whisper provider
// ---------------------------------------------------------------------------

pub struct OpenAiStt {
    pub api_key: String,
    pub url: String,
}

#[async_trait]
impl SttProvider for OpenAiStt {
    async fn transcribe(&self, wav_bytes: &[u8]) -> anyhow::Result<String> {
        use reqwest::multipart;

        let part = multipart::Part::bytes(wav_bytes.to_vec())
            .file_name("audio.wav")
            .mime_str("audio/wav")?;

        let form = multipart::Form::new()
            .text("model", "whisper-1")
            .part("file", part);

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/v1/audio/transcriptions", self.url))
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;
        let text = json["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'text' field in transcription response"))?
            .to_owned();

        Ok(text)
    }
}

// ---------------------------------------------------------------------------
// Local STT provider (generic HTTP endpoint)
// ---------------------------------------------------------------------------

pub struct LocalStt {
    pub url: String,
}

#[async_trait]
impl SttProvider for LocalStt {
    async fn transcribe(&self, wav_bytes: &[u8]) -> anyhow::Result<String> {
        let client = reqwest::Client::new();
        let response = client
            .post(&self.url)
            .body(wav_bytes.to_vec())
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;
        let text = json["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'text' field in transcription response"))?
            .to_owned();

        Ok(text)
    }
}

// ---------------------------------------------------------------------------
// Mock STT provider (for tests)
// ---------------------------------------------------------------------------

pub struct MockStt {
    pub response: String,
}

#[async_trait]
impl SttProvider for MockStt {
    async fn transcribe(&self, _wav_bytes: &[u8]) -> anyhow::Result<String> {
        Ok(self.response.clone())
    }
}
