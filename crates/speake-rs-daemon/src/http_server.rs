use std::io::Cursor;
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::time::Instant;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hound::{SampleFormat, WavSpec, WavWriter};
use qwen3_tts::{hub::ModelPaths, AudioBuffer, Qwen3TTS};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

use speake_rs_core::config::Config;
use speake_rs_core::model::ModelVariant;
use speake_rs_core::profile;
use speake_rs_core::protocol::VoiceSelection;

use crate::server;

const PRESET_SPEAKERS: &[&str] = &[
    "ryan", "vivian", "serena", "aiden", "uncle_fu", "ono_anna", "sohee", "eric", "dylan",
];

pub struct HttpOptions {
    pub model: Option<ModelVariant>,
    pub listen: SocketAddr,
}

struct SynthesisJob {
    text: String,
    voice: VoiceSelection,
    language: String,
    reply: tokio::sync::oneshot::Sender<std::result::Result<AudioBuffer, server::SynthesisFailure>>,
}

#[derive(Clone)]
struct AppState {
    model_variant: ModelVariant,
    tx: SyncSender<SynthesisJob>,
    started: Instant,
}

#[derive(Debug, Deserialize)]
struct TtsRequest {
    text: String,
    #[serde(default = "default_voice")]
    voice: String,
    #[serde(default = "default_speaking_rate")]
    speaking_rate: f32,
    #[serde(default = "default_format")]
    format: AudioFormat,
    #[serde(default)]
    #[allow(dead_code)]
    max_length: Option<usize>,
    #[serde(default = "default_language")]
    language: String,
}

fn default_voice() -> String {
    "ryan".to_string()
}

fn default_speaking_rate() -> f32 {
    1.0
}

fn default_format() -> AudioFormat {
    AudioFormat::Ogg
}

fn default_language() -> String {
    "en".to_string()
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AudioFormat {
    Ogg,
    Mp3,
}

impl AudioFormat {
    fn content_type(self) -> &'static str {
        match self {
            AudioFormat::Ogg => "audio/ogg",
            AudioFormat::Mp3 => "audio/mpeg",
        }
    }

    fn ffmpeg_format(self) -> &'static str {
        match self {
            AudioFormat::Ogg => "ogg",
            AudioFormat::Mp3 => "mp3",
        }
    }
}

struct HttpError {
    status: StatusCode,
    code: String,
    message: String,
}

impl HttpError {
    fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: code.into(),
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal".to_string(),
            message: message.into(),
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "ok": false,
            "error_code": self.code,
            "error_message": self.message,
        });
        (self.status, Json(body)).into_response()
    }
}

pub fn run(options: HttpOptions) -> Result<()> {
    let config = Config::load_or_create()?;
    server::enforce_global_voice_policy(&config)?;

    let model_variant = options.model.unwrap_or(config.daemon.model);

    eprintln!("loading model: {}", model_variant.model_id());
    let (device, fallback_reason) = server::choose_device();
    if let Some(message) = fallback_reason {
        eprintln!("warning: {message}");
    }

    let model_paths = ModelPaths::download(Some(model_variant.model_id()))
        .context("failed to download model from HuggingFace Hub")?;
    let model = Qwen3TTS::from_paths(&model_paths, device.clone())
        .context("failed to initialize Qwen3 model")?;

    eprintln!(
        "http server ready: model={}, device={:?}, listen={}",
        model_variant, device, options.listen
    );

    let started = Instant::now();
    let (tx, rx) = mpsc::sync_channel::<SynthesisJob>(16);

    // Synthesis worker thread — owns the model, processes jobs sequentially
    let worker_config = config.clone();
    let worker_variant = model_variant;
    std::thread::spawn(move || {
        while let Ok(job) = rx.recv() {
            let result = server::synthesize_to_buffer(
                &model,
                &device,
                worker_variant,
                &worker_config,
                &job.text,
                &job.language,
                &job.voice,
            );
            let _ = job.reply.send(result);
        }
    });

    let state = AppState {
        model_variant,
        tx,
        started,
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/voices", get(voices_handler))
        .route("/tts", post(tts_handler))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
                .allow_headers([axum::http::header::CONTENT_TYPE]),
        )
        .with_state(state);

    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(options.listen)
            .await
            .with_context(|| format!("failed to bind {}", options.listen))?;
        axum::serve(listener, app)
            .await
            .context("http server error")
    })
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    model: String,
    uptime_secs: u64,
}

async fn health_handler(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        model: state.model_variant.to_string(),
        uptime_secs: state.started.elapsed().as_secs(),
    })
}

async fn voices_handler(State(state): State<AppState>) -> Json<Vec<String>> {
    let mut voices: Vec<String> = PRESET_SPEAKERS.iter().map(|s| s.to_string()).collect();

    if state.model_variant == ModelVariant::Base {
        if let Ok(profiles) = profile::list_profiles() {
            for name in profiles {
                voices.push(format!("profile:{name}"));
            }
        }
    }

    Json(voices)
}

async fn tts_handler(
    State(state): State<AppState>,
    Json(req): Json<TtsRequest>,
) -> std::result::Result<Response, HttpError> {
    if req.text.trim().is_empty() {
        return Err(HttpError::bad_request("bad_request", "text is empty"));
    }

    if req.speaking_rate <= 0.0 || req.speaking_rate > 5.0 {
        return Err(HttpError::bad_request(
            "bad_request",
            "speaking_rate must be between 0.0 (exclusive) and 5.0 (inclusive)",
        ));
    }

    let voice = parse_voice_id(&req.voice)?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let job = SynthesisJob {
        text: req.text,
        voice,
        language: req.language,
        reply: reply_tx,
    };

    state.tx.send(job).map_err(|_| {
        HttpError::internal("synthesis worker is unavailable")
    })?;

    let audio = reply_rx.await.map_err(|_| {
        HttpError::internal("synthesis worker dropped the request")
    })?.map_err(|err| HttpError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: err.code.to_string(),
        message: err.message,
    })?;

    let (encoded, content_type) =
        encode_audio(&audio, req.format, req.speaking_rate).map_err(|err| {
            HttpError::internal(format!("audio encoding failed: {err}"))
        })?;

    Ok((
        StatusCode::OK,
        [
            ("content-type", content_type),
            ("content-disposition", "inline"),
        ],
        encoded,
    )
        .into_response())
}

fn parse_voice_id(voice: &str) -> std::result::Result<VoiceSelection, HttpError> {
    if let Some(name) = voice.strip_prefix("profile:") {
        if name.is_empty() {
            return Err(HttpError::bad_request(
                "bad_request",
                "profile name is empty",
            ));
        }
        Ok(VoiceSelection::Profile {
            name: name.to_string(),
        })
    } else {
        // Validate it's a known preset speaker name
        speake_rs_core::lang::parse_speaker(voice).map_err(|err| {
            HttpError::bad_request("bad_request", format!("unknown voice: {err}"))
        })?;
        Ok(VoiceSelection::Preset {
            name: voice.to_string(),
        })
    }
}

fn encode_audio(
    audio: &AudioBuffer,
    format: AudioFormat,
    speaking_rate: f32,
) -> Result<(Vec<u8>, &'static str)> {
    // Write WAV to memory
    let wav_bytes = audio_to_wav_bytes(audio)?;

    // Build ffmpeg command
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-f", "wav", "-i", "pipe:0"]);

    if (speaking_rate - 1.0).abs() > 0.01 {
        let filter = build_atempo_filter(speaking_rate);
        cmd.args(["-af", &filter]);
    }

    cmd.args(["-f", format.ffmpeg_format(), "pipe:1"]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("failed to spawn ffmpeg")?;

    // Write WAV to stdin
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().context("failed to open ffmpeg stdin")?;
        stdin
            .write_all(&wav_bytes)
            .context("failed to write to ffmpeg stdin")?;
    }

    let output = child
        .wait_with_output()
        .context("failed to wait for ffmpeg")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg exited with {}: {}", output.status, stderr.trim());
    }

    Ok((output.stdout, format.content_type()))
}

fn audio_to_wav_bytes(audio: &AudioBuffer) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: audio.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer =
            WavWriter::new(&mut cursor, spec).context("failed to create WAV writer")?;
        for &sample in &audio.samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let scaled = (clamped * 32767.0) as i16;
            writer.write_sample(scaled)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}

fn build_atempo_filter(rate: f32) -> String {
    // ffmpeg atempo filter only supports values in [0.5, 100.0].
    // For rates below 0.5, chain multiple atempo stages.
    if rate >= 0.5 {
        return format!("atempo={rate}");
    }

    let mut remaining = rate;
    let mut stages = Vec::new();
    while remaining < 0.5 {
        stages.push("atempo=0.5".to_string());
        remaining /= 0.5;
    }
    stages.push(format!("atempo={remaining}"));
    stages.join(",")
}
