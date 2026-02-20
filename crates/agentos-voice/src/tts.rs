//! Text-to-Speech via piper-tts subprocess.
//!
//! Piper reads text from stdin and outputs raw 16-bit PCM audio. We write
//! the output to a WAV file in the cache directory, which can then be played
//! via PipeWire/pw-play.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::SpeakResponse;

/// Synthesize speech from text using piper-tts.
///
/// Returns the path to the generated WAV file and processing duration.
pub async fn speak(
    piper_bin: &str,
    model_path: &str,
    text: &str,
    data_dir: &Path,
) -> Result<SpeakResponse> {
    let start = Instant::now();

    // Generate a unique output filename
    let output_file = data_dir
        .join("cache")
        .join(format!("tts_{}.wav", uuid::Uuid::new_v4()));

    let mut child = Command::new(piper_bin)
        .args([
            "--model", model_path,
            "--output_file", output_file.to_str().unwrap(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to start piper-tts")?;

    // Write text to piper's stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(text.as_bytes()).await?;
        // Drop stdin to signal EOF
    }

    let output = child.wait_with_output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("piper-tts exited with {}: {}", output.status, stderr);
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let audio_path = output_file.to_string_lossy().to_string();

    tracing::info!(duration_ms, audio_path = %audio_path, "TTS synthesis complete");

    // Play the audio via PipeWire (non-blocking)
    tokio::spawn(async move {
        let play_result = Command::new("pw-play")
            .arg(&audio_path)
            .output()
            .await;
        match play_result {
            Ok(out) if out.status.success() => {
                tracing::debug!(audio_path = %audio_path, "audio playback complete");
                // Clean up the cache file after playback
                let _ = tokio::fs::remove_file(&audio_path).await;
            }
            Ok(out) => {
                tracing::warn!(
                    status = %out.status,
                    "pw-play failed — audio output may not be configured"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to run pw-play — PipeWire may not be available");
            }
        }
    });

    Ok(SpeakResponse {
        audio_path: output_file.to_string_lossy().to_string(),
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_speak_missing_binary() {
        let result = speak(
            "/nonexistent/piper",
            "/nonexistent/model.onnx",
            "hello world",
            Path::new("/tmp"),
        )
        .await;
        assert!(result.is_err());
    }
}
