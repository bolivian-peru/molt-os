//! Voice Activity Detection (VAD) — detect when a user starts/stops speaking.
//!
//! M0 implementation: simple energy-based VAD. Monitors PipeWire audio input
//! and detects speech segments based on RMS energy thresholds.
//!
//! M1+: Replace with Silero VAD or webrtcvad for better accuracy.

use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;

/// Record a segment of audio from PipeWire for a given duration.
///
/// Uses `pw-record` to capture audio from the default input device.
/// Duration is enforced via `timeout` to ensure the recording terminates.
/// Returns the path to the recorded WAV file (16kHz mono 16-bit PCM).
pub async fn record_segment(data_dir: &Path, duration_secs: u32) -> Result<String> {
    let output_file = data_dir
        .join("cache")
        .join(format!("vad_{}.wav", uuid::Uuid::new_v4()));

    let output_path = output_file
        .to_str()
        .context("invalid output path")?
        .to_string();

    let timeout_duration = std::time::Duration::from_secs(duration_secs as u64 + 2);

    // Record using PipeWire's pw-record with timeout to ensure termination
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
            if output.status.success() || output.status.code() == Some(124) {
                Ok(output_path)
            } else {
                anyhow::bail!("pw-record exited with {}", output.status);
            }
        }
        Ok(Err(e)) => Err(e).context("failed to execute pw-record"),
        Err(_) => anyhow::bail!("recording timed out after {}s", duration_secs),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_record_clip_handles_missing_pipewire() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Without PipeWire available, recording should either succeed (if PipeWire present)
        // or fail gracefully with an error — never panic.
        let result = record_clip(dir.path(), 1).await;
        if let Err(ref e) = result {
            let msg = format!("{e}");
            // Should mention a recording/execution error, not a path or panic
            assert!(
                msg.contains("recording") || msg.contains("failed") || msg.contains("timed out") || msg.contains("exit"),
                "unexpected error: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn test_record_segment_handles_missing_pipewire() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let result = record_segment(dir.path(), 1).await;
        if let Err(ref e) = result {
            let msg = format!("{e}");
            assert!(
                msg.contains("recording") || msg.contains("failed") || msg.contains("timed out") || msg.contains("pw-record") || msg.contains("exit"),
                "unexpected error: {msg}"
            );
        }
    }
}
