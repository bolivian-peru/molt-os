//! Speech-to-Text via whisper.cpp subprocess.
//!
//! We invoke the whisper.cpp CLI binary rather than using FFI. This is simpler,
//! avoids complex native build dependencies, and whisper.cpp's CLI is fast enough
//! for real-time on modern CPUs.

use std::time::Instant;

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::TranscribeResponse;

/// Transcribe an audio file using whisper.cpp.
///
/// The audio file should be 16kHz mono WAV format (whisper.cpp's expected input).
/// Returns the transcribed text and processing duration.
pub async fn transcribe(
    whisper_bin: &str,
    model_path: &str,
    audio_path: &str,
) -> Result<TranscribeResponse> {
    let start = Instant::now();

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        Command::new(whisper_bin)
            .args([
                "--model", model_path,
                "--file", audio_path,
                "--language", "en",
                "--output-txt",
                "--no-timestamps",
                "--threads", "4",
            ])
            .output()
    ).await
    .map_err(|_| anyhow::anyhow!("whisper.cpp transcription timed out after 120s"))?
    .context("failed to execute whisper.cpp")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("whisper.cpp exited with {}: {}", output.status, stderr);
    }

    let text = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();

    let duration_ms = start.elapsed().as_millis() as u64;
    tracing::info!(duration_ms, text_len = text.len(), "transcription complete");

    Ok(TranscribeResponse { text, duration_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_transcribe_missing_binary() {
        let result = transcribe(
            "/nonexistent/whisper",
            "/nonexistent/model.bin",
            "/nonexistent/audio.wav",
        )
        .await;
        assert!(result.is_err());
    }
}
