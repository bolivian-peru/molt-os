mod tts;
mod stt;
mod vad;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::net::UnixListener;
use tokio::signal;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

/// osModa voice daemon — STT (whisper.cpp) + TTS (piper) over PipeWire.
#[derive(Parser, Debug)]
#[command(name = "osmoda-voice", version, about)]
struct Args {
    /// Unix socket to listen on.
    #[arg(long, default_value = "/run/osmoda/voice.sock")]
    socket: String,

    /// Directory for voice models and cache.
    #[arg(long, default_value = "/var/lib/osmoda/voice")]
    data_dir: String,

    /// Path to whisper.cpp binary (whisper-cpp).
    #[arg(long, default_value = "whisper-cpp")]
    whisper_bin: String,

    /// Path to whisper model file.
    #[arg(long, default_value = "/var/lib/osmoda/voice/models/ggml-base.en.bin")]
    whisper_model: String,

    /// Path to piper-tts binary.
    #[arg(long, default_value = "piper")]
    piper_bin: String,

    /// Path to piper voice model.
    #[arg(long, default_value = "/var/lib/osmoda/voice/models/en_US-lessac-medium.onnx")]
    piper_model: String,

    /// Path to agentd socket for forwarding transcriptions.
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    agentd_socket: String,
}

/// Shared voice daemon state.
struct VoiceState {
    listening: bool,
    whisper_bin: String,
    whisper_model: String,
    piper_bin: String,
    piper_model: String,
    data_dir: PathBuf,
    agentd_socket: String,
}

type SharedVoiceState = Arc<Mutex<VoiceState>>;

// === API types ===

#[derive(Deserialize)]
struct TranscribeRequest {
    /// Path to a WAV audio file to transcribe.
    audio_path: String,
}

#[derive(Serialize)]
struct TranscribeResponse {
    text: String,
    duration_ms: u64,
}

#[derive(Deserialize)]
struct SpeakRequest {
    text: String,
}

#[derive(Serialize)]
struct SpeakResponse {
    audio_path: String,
    duration_ms: u64,
}

#[derive(Deserialize)]
struct RecordRequest {
    /// Duration in seconds to record (default 5, max 30).
    duration_secs: Option<u32>,
    /// If true, also transcribe the recording and return text.
    transcribe: Option<bool>,
}

#[derive(Serialize)]
struct RecordResponse {
    audio_path: String,
    duration_secs: u32,
    /// Transcribed text (only if transcribe=true).
    text: Option<String>,
    /// Transcription processing time in ms (only if transcribe=true).
    transcribe_duration_ms: Option<u64>,
}

#[derive(Deserialize)]
struct ListenRequest {
    /// Start or stop listening.
    enabled: bool,
}

#[derive(Serialize)]
struct VoiceStatus {
    listening: bool,
    whisper_model_loaded: bool,
    piper_model_loaded: bool,
    whisper_model: String,
    piper_model: String,
}

// === Agentd logging helper ===

/// POST to agentd /memory/ingest over Unix socket. Best-effort — never blocks caller.
async fn agentd_post(socket_path: &str, body: &str) -> anyhow::Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!(error = %e, "agentd connection closed");
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/memory/ingest")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))?;

    let _resp = sender.send_request(req).await?;
    Ok(())
}

// === Handlers ===

async fn voice_status(State(state): State<SharedVoiceState>) -> Json<VoiceStatus> {
    let s = state.lock().await;
    let whisper_ok = Path::new(&s.whisper_model).exists();
    let piper_ok = Path::new(&s.piper_model).exists();
    Json(VoiceStatus {
        listening: s.listening,
        whisper_model_loaded: whisper_ok,
        piper_model_loaded: piper_ok,
        whisper_model: s.whisper_model.clone(),
        piper_model: s.piper_model.clone(),
    })
}

async fn voice_transcribe(
    State(state): State<SharedVoiceState>,
    Json(req): Json<TranscribeRequest>,
) -> Result<Json<TranscribeResponse>, (axum::http::StatusCode, String)> {
    let s = state.lock().await;
    let agentd_socket = s.agentd_socket.clone();
    match stt::transcribe(&s.whisper_bin, &s.whisper_model, &req.audio_path).await {
        Ok(result) => {
            // Log transcription to agentd — best-effort, non-blocking
            let body = serde_json::json!({
                "source": "osmoda-voice",
                "content": result.text,
                "category": "voice.transcription",
                "tags": ["voice", "transcription"],
            });
            if let Ok(body_str) = serde_json::to_string(&body) {
                tokio::spawn(async move {
                    if let Err(e) = agentd_post(&agentd_socket, &body_str).await {
                        tracing::debug!(error = %e, "failed to log transcription to agentd (non-fatal)");
                    }
                });
            }
            Ok(Json(result))
        }
        Err(e) => Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("transcription failed: {e}"),
        )),
    }
}

async fn voice_speak(
    State(state): State<SharedVoiceState>,
    Json(req): Json<SpeakRequest>,
) -> Result<Json<SpeakResponse>, (axum::http::StatusCode, String)> {
    let s = state.lock().await;
    match tts::speak(&s.piper_bin, &s.piper_model, &req.text, &s.data_dir).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("TTS failed: {e}"),
        )),
    }
}

async fn voice_record(
    State(state): State<SharedVoiceState>,
    Json(req): Json<RecordRequest>,
) -> Result<Json<RecordResponse>, (axum::http::StatusCode, String)> {
    let duration = req.duration_secs.unwrap_or(5).min(30).max(1);
    let should_transcribe = req.transcribe.unwrap_or(true);

    let s = state.lock().await;
    let data_dir = s.data_dir.clone();
    let whisper_bin = s.whisper_bin.clone();
    let whisper_model = s.whisper_model.clone();
    let agentd_socket = s.agentd_socket.clone();
    drop(s); // Release lock during recording

    // Record audio clip via PipeWire
    let audio_path = vad::record_clip(&data_dir, duration).await.map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("recording failed: {e}"),
        )
    })?;

    let mut response = RecordResponse {
        audio_path: audio_path.clone(),
        duration_secs: duration,
        text: None,
        transcribe_duration_ms: None,
    };

    // Optionally transcribe the recording
    if should_transcribe {
        match stt::transcribe(&whisper_bin, &whisper_model, &audio_path).await {
            Ok(result) => {
                // Log transcription to agentd — best-effort
                let body = serde_json::json!({
                    "source": "osmoda-voice",
                    "content": result.text,
                    "category": "voice.transcription",
                    "tags": ["voice", "transcription", "record"],
                });
                if let Ok(body_str) = serde_json::to_string(&body) {
                    let sock = agentd_socket.clone();
                    tokio::spawn(async move {
                        if let Err(e) = agentd_post(&sock, &body_str).await {
                            tracing::debug!(error = %e, "failed to log record transcription to agentd (non-fatal)");
                        }
                    });
                }
                response.text = Some(result.text);
                response.transcribe_duration_ms = Some(result.duration_ms);
            }
            Err(e) => {
                tracing::warn!(error = %e, "transcription failed after recording");
                // Return the recording even if transcription fails
                response.text = Some(format!("[transcription error: {e}]"));
            }
        }
    }

    Ok(Json(response))
}

async fn voice_listen(
    State(state): State<SharedVoiceState>,
    Json(req): Json<ListenRequest>,
) -> Json<serde_json::Value> {
    let mut s = state.lock().await;
    let prev = s.listening;
    s.listening = req.enabled;
    tracing::info!(prev = prev, now = req.enabled, "listening state changed");
    Json(serde_json::json!({
        "listening": req.enabled,
        "previous": prev,
    }))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    tracing::info!(socket = %args.socket, data_dir = %args.data_dir, "starting osmoda-voice");

    // Ensure data directory exists
    std::fs::create_dir_all(&args.data_dir).expect("failed to create voice data directory");
    std::fs::create_dir_all(format!("{}/models", args.data_dir))
        .expect("failed to create models directory");
    std::fs::create_dir_all(format!("{}/cache", args.data_dir))
        .expect("failed to create cache directory");

    // Check for models
    if !Path::new(&args.whisper_model).exists() {
        tracing::warn!(path = %args.whisper_model, "whisper model not found — STT will be unavailable until model is downloaded");
    }
    if !Path::new(&args.piper_model).exists() {
        tracing::warn!(path = %args.piper_model, "piper model not found — TTS will be unavailable until model is downloaded");
    }

    let state: SharedVoiceState = Arc::new(Mutex::new(VoiceState {
        listening: false,
        whisper_bin: args.whisper_bin,
        whisper_model: args.whisper_model,
        piper_bin: args.piper_bin,
        piper_model: args.piper_model,
        data_dir: PathBuf::from(&args.data_dir),
        agentd_socket: args.agentd_socket,
    }));

    let app = Router::new()
        .route("/voice/status", get(voice_status))
        .route("/voice/transcribe", post(voice_transcribe))
        .route("/voice/speak", post(voice_speak))
        .route("/voice/record", post(voice_record))
        .route("/voice/listen", post(voice_listen))
        .with_state(state);

    // Remove existing socket
    if Path::new(&args.socket).exists() {
        std::fs::remove_file(&args.socket).expect("failed to remove existing socket");
    }
    if let Some(parent) = Path::new(&args.socket).parent() {
        std::fs::create_dir_all(parent).expect("failed to create socket parent directory");
    }

    let listener = UnixListener::bind(&args.socket).expect("failed to bind voice socket");
    tracing::info!(socket = %args.socket, "osmoda-voice listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("voice daemon server error");

    tracing::info!("osmoda-voice shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
