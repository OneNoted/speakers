# speake-rs

Local Linux TTS daemon plus Speech Dispatcher bridge built on Qwen3-TTS.

## What it provides

- Persistent local daemon (`speake-rs-daemon`) over a Unix socket
- Optional HTTP server mode (`--features http`) for network-accessible TTS
- CLI client (`speake-rs`) for direct synthesis and Speech Dispatcher bridge mode
- `sd_generic` module integration so global `spd-say` can route through `speake-rs`
- Optional user-managed voice cloning profiles

You'll have to clone your own voices.

How:

```bash
# 1) Create a local profile from your own reference audio
speake-rs clone create --name sample_voice --ref-audio /path/to/reference.wav

# 2) Start daemon in base mode for profile synthesis
speake-rs-daemon --model base

# 3) Test profile directly
speake-rs speak "hello from my cloned profile" --profile sample_voice
```

For global `spd-say` profile mapping, see `docs/voice-cloning.md`.

ICL voice cloning is mostly untested in this project right now and should be treated as experimental.

## Build

```bash
cargo build --workspace
```

CUDA build:

```bash
cargo build --workspace --features cuda
```

Build with HTTP server support:

```bash
cargo build -p speake-rs-daemon --features http
```

## Install

```bash
cargo install --path crates/speake-rs-cli --force
cargo install --path crates/speake-rs-daemon --force
```

CUDA install:

```bash
cargo install --path crates/speake-rs-cli --force --features cuda
cargo install --path crates/speake-rs-daemon --force --features cuda
```

Install with HTTP + CUDA:

```bash
cargo install --path crates/speake-rs-daemon --force --features http,cuda
```

## Quickstart

Start daemon (preset-voice default path):

```bash
speake-rs-daemon --model custom-voice
```

Verify local health:

```bash
speake-rs doctor
speake-rs speak "hello from speake-rs" --voice ryan
```

Configure global `spd-say` routing via Speech Dispatcher:

- `docs/setup-speech-dispatcher.md`

## HTTP Server Mode

When built with `--features http`, the daemon can serve TTS over HTTP instead of a Unix socket. This is useful for running in Docker containers or exposing TTS as a network service.

```bash
speake-rs-daemon --http 0.0.0.0:9000
```

### Endpoints

- `GET /health` — returns `{"status":"ok","model":"...","uptime_secs":N}`
- `GET /voices` — returns list of available voice IDs
- `POST /tts` — synthesize speech, returns raw audio bytes

### POST /tts

Request body (JSON):

```json
{
  "text": "Hello world",
  "voice": "ryan",
  "language": "en",
  "speaking_rate": 1.0,
  "format": "ogg"
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `text` | (required) | Text to synthesize |
| `voice` | `"ryan"` | Preset speaker name or `"profile:<name>"` for a cloned voice |
| `language` | `"en"` | Language code (`en`, `zh`, `ja`, `ko`, `de`, `fr`, `ru`, `pt`, `es`, `it`) |
| `speaking_rate` | `1.0` | Playback speed (0.0–5.0, applied via ffmpeg atempo) |
| `format` | `"ogg"` | Output format: `"ogg"` or `"mp3"` |

Response: raw audio bytes with appropriate `Content-Type` header.

Requires `ffmpeg` on the system PATH for audio format conversion.

### Docker

```bash
docker build -t speake-rs .
docker run --gpus all -p 9000:9000 speake-rs
```

```bash
curl http://localhost:9000/health
curl -X POST http://localhost:9000/tts \
  -H 'Content-Type: application/json' \
  -d '{"text":"hello world","voice":"ryan","format":"ogg"}' \
  --output test.ogg
```

## Documentation

- `docs/setup-speech-dispatcher.md` - global user setup for `spd-say`
- `docs/voice-cloning.md` - optional local profile cloning workflow
- `docs/gpu-cuda.md` - CUDA build/runtime notes
- `docs/troubleshooting.md` - common runtime and routing issues
