use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use candle_core::Device;
use clap::{Parser, Subcommand};
use qwen3_tts::{auto_device, hub::ModelPaths, AudioBuffer, Qwen3TTS};
use speake_rs_core::config::{parse_voice_binding, Config};
use speake_rs_core::lang;
use speake_rs_core::model::{ModelVariant, BASE_MODEL_ID};
use speake_rs_core::paths;
use speake_rs_core::profile::{self, ProfileMode};
use speake_rs_core::protocol::{DaemonRequest, DaemonResponse, ResponseData, VoiceSelection};
use tempfile::NamedTempFile;

#[derive(Debug, Parser)]
#[command(
    name = "speake-rs",
    about = "Speech Dispatcher bridge and local Qwen3 TTS control"
)]
struct Cli {
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Debug, Subcommand)]
enum TopCommand {
    /// Manage the persistent daemon
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },

    /// Request synthesis through the daemon
    Speak {
        /// Text to synthesize; if omitted, stdin is used
        text: Option<String>,

        /// Preset speaker voice (requires daemon model custom-voice)
        #[arg(long, conflicts_with = "profile")]
        voice: Option<String>,

        /// Cloned profile voice (requires daemon model base)
        #[arg(long, conflicts_with = "voice")]
        profile: Option<String>,

        /// Language code (en, en-us, zh-cn ...)
        #[arg(long, default_value = lang::DEFAULT_LANGUAGE)]
        lang: String,

        /// Output wav path
        #[arg(short, long, default_value = "/tmp/speake-rs-output.wav")]
        output: PathBuf,
    },

    /// Manage cloned profiles
    Clone {
        #[command(subcommand)]
        command: CloneCommand,
    },

    /// Internal command used by Speech Dispatcher sd_generic integration
    SpdSynth {
        /// Text to synthesize; if omitted, stdin is used
        text: Option<String>,

        /// Speech Dispatcher language variable
        #[arg(long)]
        language: Option<String>,

        /// Speech Dispatcher symbolic voice (MALE1, FEMALE1 ...)
        #[arg(long)]
        voice: Option<String>,

        /// Explicit profile override
        #[arg(long, conflicts_with = "preset")]
        profile: Option<String>,

        /// Explicit preset override
        #[arg(long, conflicts_with = "profile")]
        preset: Option<String>,

        /// Speech Dispatcher rate value (-100..100)
        #[arg(long)]
        rate: Option<String>,

        /// Speech Dispatcher pitch value (-100..100)
        #[arg(long)]
        pitch: Option<String>,

        /// Keep generated wav at this path (useful for debugging)
        #[arg(long)]
        output: Option<PathBuf>,

        /// Override playback command (e.g. "pw-play")
        #[arg(long)]
        playback_command: Option<String>,

        /// Skip playback and only synthesize
        #[arg(long)]
        no_playback: bool,
    },

    /// Show local setup and health diagnostics
    Doctor,
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    /// Start daemon process
    Start {
        /// Model variant to run: base or custom-voice
        #[arg(long)]
        model: Option<String>,

        /// Run in the foreground
        #[arg(long)]
        foreground: bool,
    },

    /// Stop daemon process via shutdown RPC
    Stop,

    /// Check daemon health status
    Status,
}

#[derive(Debug, Subcommand)]
enum CloneCommand {
    /// Create or overwrite a cloned profile from reference audio
    Create {
        #[arg(long)]
        name: String,

        #[arg(long)]
        ref_audio: PathBuf,

        #[arg(long)]
        ref_text: Option<String>,
    },

    /// List profile names
    List,

    /// Show metadata for one profile
    Show {
        #[arg(long)]
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        TopCommand::Daemon { command } => run_daemon_command(command),
        TopCommand::Speak {
            text,
            voice,
            profile,
            lang,
            output,
        } => run_speak(text, voice, profile, lang, output),
        TopCommand::Clone { command } => run_clone_command(command),
        TopCommand::SpdSynth {
            text,
            language,
            voice,
            profile,
            preset,
            rate,
            pitch,
            output,
            playback_command,
            no_playback,
        } => run_spd_synth(
            text,
            language,
            voice,
            profile,
            preset,
            rate,
            pitch,
            output,
            playback_command,
            no_playback,
        ),
        TopCommand::Doctor => run_doctor(),
    }
}

fn run_daemon_command(command: DaemonCommand) -> Result<()> {
    let config = Config::load_or_create()?;
    match command {
        DaemonCommand::Start { model, foreground } => {
            let parsed_model = parse_model_arg(model.as_deref())?;
            let daemon_bin = daemon_binary_path();

            if foreground {
                let mut cmd = Command::new(&daemon_bin);
                if let Some(variant) = parsed_model {
                    cmd.arg("--model").arg(variant.as_str());
                }

                let status = cmd.status().with_context(|| {
                    format!("failed to start daemon binary: {}", daemon_bin.display())
                })?;
                anyhow::ensure!(status.success(), "daemon exited with status {status}");
                return Ok(());
            }

            if daemon_is_alive(&config).unwrap_or(false) {
                println!("daemon already running");
                return Ok(());
            }

            let mut cmd = Command::new(&daemon_bin);
            if let Some(variant) = parsed_model {
                cmd.arg("--model").arg(variant.as_str());
            }

            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            cmd.spawn().with_context(|| {
                format!("failed to spawn daemon binary: {}", daemon_bin.display())
            })?;

            std::thread::sleep(Duration::from_millis(400));

            if daemon_is_alive(&config).unwrap_or(false) {
                println!("daemon started");
                return Ok(());
            }

            anyhow::bail!(
                "daemon process was spawned but health check failed; run `speake-rs daemon start --foreground` to inspect logs"
            )
        }
        DaemonCommand::Stop => {
            let response = send_request(&config, &DaemonRequest::Shutdown)
                .context("failed to send shutdown request")?;
            ensure_ok(response)?;
            println!("daemon stop request sent");
            Ok(())
        }
        DaemonCommand::Status => {
            let response =
                send_request(&config, &DaemonRequest::Health).context("daemon is not reachable")?;
            let data = ensure_ok(response)?;

            match data {
                Some(ResponseData::Health(health)) => {
                    println!("status: running");
                    println!("pid: {}", health.pid);
                    println!("model: {}", health.model);
                    println!("device: {}", health.device);
                    println!("socket: {}", health.socket.display());
                    println!("uptime_secs: {}", health.uptime_secs);
                }
                _ => println!("status: running (unexpected health payload)"),
            }

            Ok(())
        }
    }
}

fn run_speak(
    text: Option<String>,
    voice: Option<String>,
    profile_name: Option<String>,
    language: String,
    output: PathBuf,
) -> Result<()> {
    let config = Config::load_or_create()?;
    let text = read_text(text)?;
    if text.trim().is_empty() {
        return Ok(());
    }

    let voice = match (voice, profile_name) {
        (Some(name), None) => VoiceSelection::preset(name),
        (None, Some(name)) => VoiceSelection::profile(name),
        (None, None) => VoiceSelection::preset(lang::DEFAULT_PRESET_VOICE),
        (Some(_), Some(_)) => anyhow::bail!("--voice and --profile are mutually exclusive"),
    };

    let request = DaemonRequest::Synthesize {
        text,
        language,
        output: output.clone(),
        voice,
        rate: None,
        pitch: None,
    };

    let response = send_request(&config, &request).context("failed to communicate with daemon")?;
    ensure_ok(response)?;

    println!("wrote {}", output.display());
    Ok(())
}

fn run_clone_command(command: CloneCommand) -> Result<()> {
    match command {
        CloneCommand::Create {
            name,
            ref_audio,
            ref_text,
        } => {
            let audio = AudioBuffer::load(&ref_audio).with_context(|| {
                format!("failed to load reference audio: {}", ref_audio.display())
            })?;

            let device = choose_device_for_local_tasks();
            let paths = ModelPaths::download(Some(BASE_MODEL_ID))
                .context("failed to download base model for cloning")?;
            let model = Qwen3TTS::from_paths(&paths, device)
                .context("failed to initialize base model for cloning")?;

            let prompt = model
                .create_voice_clone_prompt(&audio, ref_text.as_deref())
                .context("failed to create voice clone prompt")?;

            profile::save_profile(&name, &prompt)?;
            println!("saved profile {name}");
            Ok(())
        }
        CloneCommand::List => {
            let names = profile::list_profiles()?;
            for name in names {
                println!("{name}");
            }
            Ok(())
        }
        CloneCommand::Show { name } => {
            let meta = profile::read_profile_meta(&name)?;
            println!("{}", serde_json::to_string_pretty(&meta)?);
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_spd_synth(
    text: Option<String>,
    language: Option<String>,
    symbolic_voice: Option<String>,
    profile_name: Option<String>,
    preset_name: Option<String>,
    rate: Option<String>,
    pitch: Option<String>,
    output: Option<PathBuf>,
    playback_command: Option<String>,
    no_playback: bool,
) -> Result<()> {
    let config = Config::load_or_create()?;
    let text = sanitize_spd_text(&read_text(text)?);
    if text.trim().is_empty() {
        return Ok(());
    }

    let language = normalize_spd_language(
        language.as_deref(),
        &config.speech_dispatcher.default_language,
    );
    let rate = parse_spd_scalar(rate.as_deref(), "rate")?;
    let pitch = parse_spd_scalar(pitch.as_deref(), "pitch")?;

    let primary_voice = if let Some(name) = profile_name {
        VoiceSelection::profile(name)
    } else if let Some(name) = preset_name {
        VoiceSelection::preset(name)
    } else {
        config
            .speech_dispatcher
            .resolve_voice_selection(symbolic_voice.as_deref())?
    };

    let (output_path, temp_file_guard, keep_output) = match output {
        Some(path) => (path, None, true),
        None => {
            let mut temp = NamedTempFile::new_in(std::env::temp_dir())
                .context("failed to allocate temp wav file")?;
            let path = temp.path().to_path_buf();
            temp.as_file_mut()
                .flush()
                .context("failed to initialize temp wav file")?;
            (path, Some(temp), false)
        }
    };

    let mut attempts = Vec::new();
    attempts.push((primary_voice.clone(), "requested voice".to_string()));
    if let Some(fallback) = config.speech_dispatcher.fallback_voice_selection() {
        if fallback != primary_voice {
            attempts.push((fallback, "fallback profile".to_string()));
        }
    }

    let mut last_success: Option<VoiceSelection> = None;
    let mut failures = Vec::new();
    for (voice, source) in attempts {
        if let Err(err) = validate_voice_for_spd(&config, &voice) {
            failures.push(format!(
                "{source} ({}) skipped: {err}",
                describe_voice(&voice)
            ));
            continue;
        }

        let request = DaemonRequest::Synthesize {
            text: text.clone(),
            language: language.clone(),
            output: output_path.clone(),
            voice: voice.clone(),
            rate,
            pitch,
        };

        match send_request(&config, &request)
            .context("failed to send synth request to daemon")
            .and_then(ensure_ok)
        {
            Ok(_) => {
                last_success = Some(voice);
                break;
            }
            Err(err) => {
                failures.push(format!(
                    "{source} ({}) failed: {err}",
                    describe_voice(&voice)
                ));
            }
        }
    }

    let Some(_voice_used) = last_success else {
        let detail = if failures.is_empty() {
            "no valid voice candidates were available".to_string()
        } else {
            failures.join("; ")
        };
        anyhow::bail!("all synthesis attempts failed: {detail}");
    };

    if !no_playback {
        let playback_command = resolve_playback_command(
            playback_command.as_deref(),
            config.speech_dispatcher.playback_command.as_deref(),
        )?;
        run_playback(&playback_command, &output_path)?;
    }

    if keep_output {
        println!("wrote {}", output_path.display());
    }

    drop(temp_file_guard);
    Ok(())
}

fn run_doctor() -> Result<()> {
    let config = Config::load_or_create()?;
    let config_path = paths::config_path();
    let socket_path = config.daemon.resolved_socket_path();

    println!("config: {}", config_path.display());
    println!("socket: {}", socket_path.display());
    println!("profiles: {}", paths::profiles_dir().display());

    let daemon_status = match send_request(&config, &DaemonRequest::Health) {
        Ok(resp) if resp.ok => "running",
        _ => "not running",
    };
    println!("daemon: {daemon_status}");
    println!("request_timeout_ms: {}", config.daemon.request_timeout_ms);
    println!(
        "synthesis_timeout_ms: {}",
        config.daemon.synthesis_timeout_ms
    );
    println!("allow_icl: {}", config.speech_dispatcher.allow_icl);
    println!(
        "fallback_profile: {}",
        config
            .speech_dispatcher
            .fallback_profile
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("(unset)")
    );

    println!(
        "spd-say: {}",
        if command_in_path("spd-say") {
            "found"
        } else {
            "missing"
        }
    );
    println!(
        "speech-dispatcher: {}",
        if command_in_path("speech-dispatcher") {
            "found"
        } else {
            "missing"
        }
    );

    let spd_dir = paths::config_home().join("speech-dispatcher");
    let user_module = spd_dir.join("modules/speake-rs-generic.conf");
    let user_conf = spd_dir.join("speechd.conf");

    println!(
        "module config: {} ({})",
        user_module.display(),
        if user_module.exists() {
            "present"
        } else {
            "missing"
        }
    );
    println!(
        "speechd.conf: {} ({})",
        user_conf.display(),
        if user_conf.exists() {
            "present"
        } else {
            "missing"
        }
    );

    if user_conf.exists() {
        let body = std::fs::read_to_string(&user_conf)
            .with_context(|| format!("failed to read {}", user_conf.display()))?;
        let default_enabled = body
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case("DefaultModule speake-rs"));
        println!(
            "default module: {}",
            if default_enabled {
                "speake-rs"
            } else {
                "not set to speake-rs"
            }
        );
    }

    for (symbolic, binding) in &config.speech_dispatcher.voice_map {
        let selection = match parse_voice_binding(binding) {
            Ok(value) => value,
            Err(err) => {
                println!("voice-map warning: {symbolic} has invalid binding '{binding}': {err}");
                continue;
            }
        };
        if let VoiceSelection::Profile { name } = selection {
            match profile::read_profile_meta(&name) {
                Ok(meta)
                    if meta.mode == ProfileMode::Icl && !config.speech_dispatcher.allow_icl =>
                {
                    println!(
                        "voice-map warning: {symbolic} uses ICL profile '{name}' but allow_icl=false"
                    );
                }
                Ok(_) => {}
                Err(err) => {
                    println!(
                        "voice-map warning: {symbolic} references unreadable profile '{name}': {err}"
                    );
                }
            }
        }
    }

    if let Some(fallback) = config.speech_dispatcher.fallback_voice_selection() {
        if let VoiceSelection::Profile { name } = fallback {
            match profile::read_profile_meta(&name) {
                Ok(meta)
                    if meta.mode == ProfileMode::Icl && !config.speech_dispatcher.allow_icl =>
                {
                    println!(
                        "fallback warning: fallback profile '{name}' is ICL but allow_icl=false"
                    );
                }
                Ok(_) => {}
                Err(err) => {
                    println!("fallback warning: fallback profile '{name}' is not readable: {err}");
                }
            }
        }
    }

    Ok(())
}

fn send_request(config: &Config, request: &DaemonRequest) -> Result<DaemonResponse> {
    let socket = config.daemon.resolved_socket_path();
    let timeout = Duration::from_millis(config.daemon.request_timeout_ms);

    let mut stream = UnixStream::connect(&socket)
        .with_context(|| format!("failed to connect to daemon socket: {}", socket.display()))?;
    stream
        .set_read_timeout(Some(timeout))
        .context("failed to set daemon read timeout")?;
    stream
        .set_write_timeout(Some(timeout))
        .context("failed to set daemon write timeout")?;

    let mut body = serde_json::to_string(request).context("failed to serialize daemon request")?;
    body.push('\n');
    stream
        .write_all(body.as_bytes())
        .context("failed to write daemon request")?;

    let mut line = String::new();
    match BufReader::new(&stream).read_line(&mut line) {
        Ok(_) => {}
        Err(err)
            if err.kind() == std::io::ErrorKind::TimedOut
                || err.kind() == std::io::ErrorKind::WouldBlock =>
        {
            anyhow::bail!(
                "daemon response timed out after {}ms",
                config.daemon.request_timeout_ms
            );
        }
        Err(err) => return Err(err).context("failed to read daemon response"),
    }

    serde_json::from_str::<DaemonResponse>(&line).context("failed to parse daemon response")
}

fn ensure_ok(response: DaemonResponse) -> Result<Option<ResponseData>> {
    if response.ok {
        return Ok(response.data);
    }

    let code = response.error_code.unwrap_or_else(|| "unknown".to_string());
    let message = response
        .error_message
        .unwrap_or_else(|| "daemon returned an unspecified error".to_string());
    let detail_suffix = response
        .error_details
        .unwrap_or_default()
        .into_iter()
        .map(|detail| format!("cause: {detail}"))
        .collect::<Vec<_>>();

    if detail_suffix.is_empty() {
        anyhow::bail!("{code}: {message}");
    }

    anyhow::bail!("{code}: {message}; {}", detail_suffix.join("; "))
}

fn validate_voice_for_spd(config: &Config, voice: &VoiceSelection) -> Result<()> {
    let VoiceSelection::Profile { name } = voice else {
        return Ok(());
    };

    let meta = profile::read_profile_meta(name)
        .with_context(|| format!("failed to read profile metadata for '{name}'"))?;
    if meta.mode == ProfileMode::Icl && !config.speech_dispatcher.allow_icl {
        anyhow::bail!(
            "icl_not_allowed: profile '{name}' is ICL and speech_dispatcher.allow_icl=false"
        );
    }

    Ok(())
}

fn describe_voice(voice: &VoiceSelection) -> String {
    match voice {
        VoiceSelection::Preset { name } => format!("preset:{name}"),
        VoiceSelection::Profile { name } => format!("profile:{name}"),
    }
}

fn parse_model_arg(value: Option<&str>) -> Result<Option<ModelVariant>> {
    match value {
        Some(raw) => Ok(Some(raw.parse::<ModelVariant>()?)),
        None => Ok(None),
    }
}

fn daemon_binary_path() -> PathBuf {
    let candidate = std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("speake-rs-daemon"));

    match candidate {
        Some(path) if path.exists() => path,
        _ => PathBuf::from("speake-rs-daemon"),
    }
}

fn daemon_is_alive(config: &Config) -> Result<bool> {
    let response = send_request(config, &DaemonRequest::Health)?;
    Ok(response.ok)
}

fn read_text(arg: Option<String>) -> Result<String> {
    match arg {
        Some(value) => Ok(value),
        None => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .context("failed to read text from stdin")?;
            Ok(buffer)
        }
    }
}

fn choose_device_for_local_tasks() -> Device {
    match auto_device() {
        Ok(device) => {
            if matches!(device, Device::Cpu) {
                eprintln!("warning: CUDA not selected for clone task; using CPU");
            }
            device
        }
        Err(err) => {
            eprintln!("warning: failed to initialize accelerated device ({err}); using CPU");
            Device::Cpu
        }
    }
}

fn resolve_playback_command(cli: Option<&str>, config: Option<&str>) -> Result<String> {
    let picked = cli
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| config.map(str::trim).filter(|s| !s.is_empty()))
        .map(ToOwned::to_owned)
        .or_else(default_playback_command);

    picked.ok_or_else(|| {
        anyhow::anyhow!(
            "no playback command found (set speech_dispatcher.playback_command or pass --playback-command)"
        )
    })
}

fn default_playback_command() -> Option<String> {
    for candidate in ["pw-play", "paplay", "aplay"] {
        if command_in_path(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

fn run_playback(command_line: &str, wav_path: &Path) -> Result<()> {
    let parts = shlex::split(command_line)
        .ok_or_else(|| anyhow::anyhow!("failed to parse playback command: {command_line}"))?;
    anyhow::ensure!(!parts.is_empty(), "playback command is empty");

    let mut cmd = Command::new(&parts[0]);
    if parts.len() > 1 {
        cmd.args(&parts[1..]);
    }
    cmd.arg(wav_path);

    let status = cmd
        .status()
        .with_context(|| format!("failed to execute playback command: {command_line}"))?;

    anyhow::ensure!(
        status.success(),
        "playback command exited with status {status}: {command_line}"
    );

    Ok(())
}

fn command_in_path(command: &str) -> bool {
    let path_var = match std::env::var_os("PATH") {
        Some(value) => value,
        None => return false,
    };

    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(command);
        if candidate.exists() {
            return true;
        }
    }

    false
}

fn sanitize_spd_text(input: &str) -> String {
    let trimmed = input.trim();
    if !trimmed.contains('<') || !trimmed.contains('>') {
        return trimmed.to_string();
    }

    let mut out = String::with_capacity(trimmed.len());
    let mut in_tag = false;
    for ch in trimmed.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    out.trim().to_string()
}

fn normalize_spd_language(input: Option<&str>, default: &str) -> String {
    let value = input.map(str::trim).filter(|s| !s.is_empty());
    match value {
        Some(raw) if raw.eq_ignore_ascii_case("c") || raw.eq_ignore_ascii_case("posix") => {
            default.to_string()
        }
        Some(raw) => raw.to_string(),
        None => default.to_string(),
    }
}

fn parse_spd_scalar(raw: Option<&str>, field: &str) -> Result<Option<i32>> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };

    if let Ok(value) = raw.parse::<i32>() {
        return Ok(Some(value.clamp(-100, 100)));
    }

    let value = raw
        .parse::<f32>()
        .with_context(|| format!("invalid {field} value '{raw}'"))?;
    Ok(Some(value.round() as i32).map(|v| v.clamp(-100, 100)))
}

#[cfg(test)]
mod tests {
    use super::sanitize_spd_text;

    #[test]
    fn strips_ssml_tags_from_spd_input() {
        let input = "<speak>module activity test</speak>";
        assert_eq!(sanitize_spd_text(input), "module activity test");
    }

    #[test]
    fn leaves_plain_text_unchanged() {
        assert_eq!(sanitize_spd_text("hello world"), "hello world");
    }
}
