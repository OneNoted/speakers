use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use candle_core::Device;
use qwen3_tts::{auto_device, hub::ModelPaths, Qwen3TTS, SynthesisOptions};
use speake_rs_core::config::Config;
use speake_rs_core::lang;
use speake_rs_core::model::ModelVariant;
use speake_rs_core::paths;
use speake_rs_core::profile::{self, ProfileMode};
use speake_rs_core::protocol::{
    DaemonRequest, DaemonResponse, HealthData, ResponseData, SynthesisData, VoiceSelection,
};

const MAX_ICL_REF_TEXT_TOKENS: usize = 240;
const MAX_ICL_REF_CODE_FRAMES: usize = 450;
const ICL_MAX_GENERATION_FRAMES: usize = 240;

pub struct DaemonOptions {
    pub model: Option<ModelVariant>,
    pub socket_path: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct SynthesisFailure {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) details: Vec<String>,
}

impl SynthesisFailure {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Vec::new(),
        }
    }

    pub(crate) fn from_anyhow(code: &'static str, err: anyhow::Error) -> Self {
        let message = err.to_string();
        let details = collect_error_chain(&err);
        Self {
            code,
            message,
            details,
        }
    }
}

pub fn run(options: DaemonOptions) -> Result<()> {
    let config = Config::load_or_create()?;
    paths::ensure_runtime_dir()?;
    enforce_global_voice_policy(&config)?;

    let model_variant = options.model.unwrap_or(config.daemon.model);
    let socket_path = options
        .socket_path
        .unwrap_or_else(|| config.daemon.resolved_socket_path());

    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!(
                "failed to remove stale socket file: {}",
                socket_path.display()
            )
        })?;
    }

    paths::ensure_parent(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind daemon socket: {}", socket_path.display()))?;
    listener
        .set_nonblocking(true)
        .context("failed to set listener nonblocking")?;

    let pid_path = paths::pid_path();
    paths::ensure_parent(&pid_path)?;
    std::fs::write(&pid_path, std::process::id().to_string())
        .with_context(|| format!("failed to write pid file: {}", pid_path.display()))?;

    let started = Instant::now();

    eprintln!("loading model: {}", model_variant.model_id());
    let (device, fallback_reason) = choose_device();
    if let Some(message) = fallback_reason {
        eprintln!("warning: {message}");
    }

    let model_paths = ModelPaths::download(Some(model_variant.model_id()))
        .context("failed to download model from HuggingFace Hub")?;
    let model = Qwen3TTS::from_paths(&model_paths, device.clone())
        .context("failed to initialize Qwen3 model")?;

    eprintln!(
        "daemon ready: model={}, device={:?}, socket={}",
        model_variant,
        device,
        socket_path.display()
    );

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let ctrlc_flag = Arc::clone(&shutdown_flag);
    ctrlc::set_handler(move || {
        ctrlc_flag.store(true, Ordering::SeqCst);
    })?;

    while !shutdown_flag.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let should_shutdown = handle_connection(
                    &mut stream,
                    &model,
                    &device,
                    model_variant,
                    &socket_path,
                    started,
                    &config,
                )
                .unwrap_or_else(|err| {
                    let response = DaemonResponse::error_with_details(
                        "internal",
                        err.to_string(),
                        collect_error_chain(&err),
                    );
                    let _ = send_response(&mut stream, &response);
                    false
                });

                let _ = stream.shutdown(Shutdown::Both);

                if should_shutdown {
                    shutdown_flag.store(true, Ordering::SeqCst);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                eprintln!("accept error: {err}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    std::fs::remove_file(&socket_path).ok();
    std::fs::remove_file(&pid_path).ok();

    eprintln!("daemon stopped");
    Ok(())
}

pub(crate) fn choose_device() -> (Device, Option<String>) {
    match auto_device() {
        Ok(device) => {
            if matches!(device, Device::Cpu) {
                (
                    device,
                    Some("CUDA was not selected; running on CPU (slower synthesis).".to_string()),
                )
            } else {
                (device, None)
            }
        }
        Err(err) => (
            Device::Cpu,
            Some(format!(
                "failed to initialize accelerated device ({err}); falling back to CPU"
            )),
        ),
    }
}

fn handle_connection(
    stream: &mut UnixStream,
    model: &Qwen3TTS,
    device: &Device,
    model_variant: ModelVariant,
    socket_path: &PathBuf,
    started: Instant,
    config: &Config,
) -> Result<bool> {
    let mut line = String::new();
    BufReader::new(&mut *stream)
        .read_line(&mut line)
        .context("failed to read request")?;

    if line.trim().is_empty() {
        let response = DaemonResponse::error("bad_request", "empty request");
        send_response(stream, &response)?;
        return Ok(false);
    }

    let request = match serde_json::from_str::<DaemonRequest>(&line) {
        Ok(req) => req,
        Err(err) => {
            let response =
                DaemonResponse::error("bad_request", format!("invalid JSON request: {err}"));
            send_response(stream, &response)?;
            return Ok(false);
        }
    };

    let mut should_shutdown = false;
    let response = match request {
        DaemonRequest::Health => {
            let data = ResponseData::Health(HealthData {
                pid: std::process::id(),
                model: model_variant,
                device: format!("{device:?}"),
                socket: socket_path.clone(),
                uptime_secs: started.elapsed().as_secs(),
            });
            DaemonResponse::ok(Some(data))
        }
        DaemonRequest::Shutdown => {
            should_shutdown = true;
            DaemonResponse::ok(Some(ResponseData::Ack))
        }
        DaemonRequest::Synthesize {
            text,
            language,
            output,
            voice,
            rate: _,
            pitch: _,
        } => match synthesize_request(
            model,
            device,
            model_variant,
            config,
            &text,
            &language,
            &output,
            &voice,
        ) {
            Ok(data) => DaemonResponse::ok(Some(ResponseData::Synthesis(data))),
            Err(err) => {
                eprintln!("synthesis error [{}]: {}", err.code, err.message);
                DaemonResponse::error_with_details(err.code, err.message, err.details)
            }
        },
    };

    send_response(stream, &response)?;
    Ok(should_shutdown)
}

fn send_response(stream: &mut UnixStream, response: &DaemonResponse) -> Result<()> {
    let body = serde_json::to_string(response).context("failed to encode response")?;
    stream
        .write_all(body.as_bytes())
        .context("failed to write response")?;
    stream
        .write_all(b"\n")
        .context("failed to write response newline")?;
    Ok(())
}

pub(crate) fn synthesize_to_buffer(
    model: &Qwen3TTS,
    device: &Device,
    model_variant: ModelVariant,
    config: &Config,
    text: &str,
    language: &str,
    voice: &VoiceSelection,
) -> std::result::Result<qwen3_tts::AudioBuffer, SynthesisFailure> {
    let text = text.trim();
    if text.is_empty() {
        return Err(SynthesisFailure::new("bad_request", "text is empty"));
    }

    let language = lang::parse_language(language)
        .map_err(|err| SynthesisFailure::from_anyhow("bad_request", err))?;

    let synth_timeout = Duration::from_millis(config.daemon.synthesis_timeout_ms.max(1));

    let (audio, is_icl) = match voice {
        VoiceSelection::Preset { name } => {
            if model_variant != ModelVariant::CustomVoice {
                return Err(SynthesisFailure::new(
                    "device_incompatible",
                    "preset voices require daemon model custom-voice",
                ));
            }

            let speaker = lang::parse_speaker(name)
                .map_err(|err| SynthesisFailure::from_anyhow("bad_request", err))?;
            let started = Instant::now();
            let audio = model
                .synthesize_with_voice(text, speaker, language, None)
                .map_err(|err| {
                    SynthesisFailure::from_anyhow(
                        "synthesis_failed",
                        err.context("preset synthesis failed"),
                    )
                })?;

            let elapsed = started.elapsed();
            if elapsed > synth_timeout {
                return Err(SynthesisFailure::new(
                    "synthesis_timeout",
                    format!(
                        "synthesis exceeded timeout ({}ms > {}ms)",
                        elapsed.as_millis(),
                        synth_timeout.as_millis()
                    ),
                ));
            }

            (audio, false)
        }
        VoiceSelection::Profile { name } => {
            if model_variant != ModelVariant::Base {
                return Err(SynthesisFailure::new(
                    "device_incompatible",
                    "profile voices require daemon model base",
                ));
            }

            let meta = profile::read_profile_meta(name)
                .map_err(|err| SynthesisFailure::from_anyhow("profile_invalid", err))?;
            let is_icl = meta.mode == ProfileMode::Icl;
            if is_icl && !config.speech_dispatcher.allow_icl {
                return Err(SynthesisFailure::new(
                    "icl_not_allowed",
                    format!(
                        "ICL profile '{name}' is disabled by config (speech_dispatcher.allow_icl=false)"
                    ),
                ));
            }
            if is_icl {
                let text_ids = meta.ref_text_ids.as_ref().map_or(0, Vec::len);
                if text_ids > MAX_ICL_REF_TEXT_TOKENS {
                    return Err(SynthesisFailure::new(
                        "profile_invalid",
                        format!(
                            "ICL profile '{name}' has {text_ids} ref text tokens; limit is {MAX_ICL_REF_TEXT_TOKENS}"
                        ),
                    ));
                }
            }

            let prompt = profile::load_profile(name, device)
                .map_err(|err| SynthesisFailure::from_anyhow("profile_invalid", err))?;

            if is_icl {
                let ref_codes = prompt.ref_codes.as_ref().ok_or_else(|| {
                    SynthesisFailure::new("profile_invalid", "ICL profile is missing ref_codes")
                })?;
                let (frames, codebooks) = ref_codes
                    .dims2()
                    .map_err(|err| SynthesisFailure::from_anyhow("profile_invalid", err.into()))?;

                if codebooks != 16 {
                    return Err(SynthesisFailure::new(
                        "profile_invalid",
                        format!("ICL profile has {codebooks} codebooks; expected 16"),
                    ));
                }
                if frames > MAX_ICL_REF_CODE_FRAMES {
                    return Err(SynthesisFailure::new(
                        "profile_invalid",
                        format!(
                            "ICL profile has {frames} reference frames; limit is {MAX_ICL_REF_CODE_FRAMES}"
                        ),
                    ));
                }
            }

            let options = profile_synthesis_options(is_icl);
            let started = Instant::now();
            let audio = model
                .synthesize_voice_clone(text, &prompt, language, Some(options))
                .map_err(|err| {
                    SynthesisFailure::from_anyhow(
                        "synthesis_failed",
                        err.context("profile synthesis failed"),
                    )
                })?;
            let elapsed = started.elapsed();
            if elapsed > synth_timeout {
                let code = if is_icl {
                    "icl_timeout"
                } else {
                    "synthesis_timeout"
                };
                return Err(SynthesisFailure::new(
                    code,
                    format!(
                        "synthesis exceeded timeout ({}ms > {}ms)",
                        elapsed.as_millis(),
                        synth_timeout.as_millis()
                    ),
                ));
            }

            (audio, is_icl)
        }
    };

    if is_icl {
        eprintln!("warning: ICL profile synthesis was executed (experimental path)");
    }

    Ok(audio)
}

fn synthesize_request(
    model: &Qwen3TTS,
    device: &Device,
    model_variant: ModelVariant,
    config: &Config,
    text: &str,
    language: &str,
    output: &PathBuf,
    voice: &VoiceSelection,
) -> std::result::Result<SynthesisData, SynthesisFailure> {
    let audio = synthesize_to_buffer(model, device, model_variant, config, text, language, voice)?;

    paths::ensure_parent(output)
        .map_err(|err| SynthesisFailure::from_anyhow("synthesis_failed", err))?;
    audio.save(output).map_err(|err| {
        SynthesisFailure::from_anyhow(
            "synthesis_failed",
            err.context(format!("failed to save wav: {}", output.display())),
        )
    })?;

    Ok(SynthesisData {
        output: output.clone(),
    })
}

pub(crate) fn profile_synthesis_options(is_icl: bool) -> SynthesisOptions {
    let mut options = SynthesisOptions {
        // Keep cloned voice output stable and reduce prompt-copy leakage.
        temperature: 0.35,
        top_k: 20,
        top_p: 0.75,
        repetition_penalty: 1.2,
        seed: Some(42),
        ..SynthesisOptions::default()
    };
    if is_icl {
        options.max_length = ICL_MAX_GENERATION_FRAMES;
        options.repetition_penalty = options.repetition_penalty.max(1.35);
    }
    options
}

pub(crate) fn enforce_global_voice_policy(config: &Config) -> Result<()> {
    if config.speech_dispatcher.allow_icl {
        return Ok(());
    }

    let default_voice = config.speech_dispatcher.resolve_voice_selection(None)?;
    if let VoiceSelection::Profile { name } = default_voice {
        let meta = profile::read_profile_meta(&name)
            .with_context(|| format!("default profile '{name}' is invalid"))?;
        if meta.mode == ProfileMode::Icl {
            anyhow::bail!(
                "default speech-dispatcher voice maps to ICL profile '{name}', but speech_dispatcher.allow_icl=false"
            );
        }
    }

    for (symbolic, binding) in &config.speech_dispatcher.voice_map {
        let selection = match speake_rs_core::config::parse_voice_binding(binding) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("warning: invalid voice_map binding for {symbolic}: {err}");
                continue;
            }
        };
        if let VoiceSelection::Profile { name } = selection {
            let meta = match profile::read_profile_meta(&name) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!(
                        "warning: voice_map {symbolic} references invalid profile '{name}': {err}"
                    );
                    continue;
                }
            };
            if meta.mode == ProfileMode::Icl {
                eprintln!(
                    "warning: voice_map {symbolic} references ICL profile '{name}' while allow_icl=false; it will be rejected at request time"
                );
            }
        }
    }

    Ok(())
}

pub(crate) fn collect_error_chain(err: &anyhow::Error) -> Vec<String> {
    err.chain().skip(1).map(|cause| cause.to_string()).collect()
}
