mod tts;
mod stt;
#[allow(dead_code)]
mod vad;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

/// AgentOS voice daemon — STT (whisper.cpp) + TTS (piper) over PipeWire.
#[derive(Parser, Debug)]
#[command(name = "agentos-voice", version, about)]
struct Args {
    /// Unix socket to listen on.
    #[arg(long, default_value = "/run/agentos/voice.sock")]
    socket: String,

    /// Directory for voice models and cache.
    #[arg(long, default_value = "/var/lib/agentos/voice")]
    data_dir: String,

    /// Path to whisper.cpp binary (whisper-cpp).
    #[arg(long, default_value = "whisper-cpp")]
    whisper_bin: String,

    /// Path to whisper model file.
    #[arg(long, default_value = "/var/lib/agentos/voice/models/ggml-base.en.bin")]
    whisper_model: String,

    /// Path to piper-tts binary.
    #[arg(long, default_value = "piper")]
    piper_bin: String,

    /// Path to piper voice model.
    #[arg(long, default_value = "/var/lib/agentos/voice/models/en_US-lessac-medium.onnx")]
    piper_model: String,

    /// Path to agentd socket for forwarding transcriptions.
    #[arg(long, default_value = "/run/agentos/agentd.sock")]
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
    /// Will be used in M1 for forwarding transcriptions to agentd.
    #[allow(dead_code)]
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
    match stt::transcribe(&s.whisper_bin, &s.whisper_model, &req.audio_path).await {
        Ok(result) => Ok(Json(result)),
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

    tracing::info!(socket = %args.socket, data_dir = %args.data_dir, "starting agentos-voice");

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
    tracing::info!(socket = %args.socket, "agentos-voice listening");

    axum::serve(listener, app)
        .await
        .expect("voice daemon server error");
}
