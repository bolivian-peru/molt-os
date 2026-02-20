//! Voice Activity Detection (VAD) — detect when a user starts/stops speaking.
//!
//! M0 implementation: simple energy-based VAD. Monitors PipeWire audio input
//! and detects speech segments based on RMS energy thresholds.
//!
//! M1+: Replace with Silero VAD or webrtcvad for better accuracy.

use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;

/// Record a segment of audio from PipeWire when speech is detected.
///
/// Uses `pw-record` to capture audio from the default input device.
/// Returns the path to the recorded WAV file.
pub async fn record_segment(data_dir: &Path, _duration_secs: u32) -> Result<String> {
    let output_file = data_dir
        .join("cache")
        .join(format!("vad_{}.wav", uuid::Uuid::new_v4()));

    let output_path = output_file
        .to_str()
        .context("invalid output path")?
        .to_string();

    // Record using PipeWire's pw-record
    // Format: 16kHz mono 16-bit PCM (what whisper.cpp expects)
    let status = Command::new("pw-record")
        .args([
            "--format", "s16",
            "--rate", "16000",
            "--channels", "1",
            "--target", "0",  // default input
            &output_path,
        ])
        .spawn()
        .context("failed to start pw-record")?
        .wait()
        .await
        .context("pw-record failed")?;

    if !status.success() {
        anyhow::bail!("pw-record exited with {}", status);
    }

    Ok(output_path)
}

/// Record a fixed-duration audio clip (for manual transcription triggers).
pub async fn record_clip(data_dir: &Path, duration_secs: u32) -> Result<String> {
    let output_file = data_dir
        .join("cache")
        .join(format!("clip_{}.wav", uuid::Uuid::new_v4()));

    let output_path = output_file
        .to_str()
        .context("invalid output path")?
        .to_string();

    let timeout_duration = std::time::Duration::from_secs(duration_secs as u64 + 1);

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("timeout")
            .args([
                &duration_secs.to_string(),
                "pw-record",
                "--format", "s16",
                "--rate", "16000",
                "--channels", "1",
                &output_path,
            ])
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            // timeout exits with 124 when it kills the process, which is expected
            if output.status.success() || output.status.code() == Some(124) {
                Ok(output_path)
            } else {
                anyhow::bail!("recording failed with status {}", output.status);
            }
        }
        Ok(Err(e)) => Err(e).context("failed to execute recording command"),
        Err(_) => {
            // Timeout from tokio — should not happen since we use `timeout` binary
            anyhow::bail!("recording timed out");
        }
    }
}
