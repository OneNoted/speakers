# speake-rs

Local Linux TTS daemon plus Speech Dispatcher bridge built on Qwen3-TTS.

## What it provides

- Persistent local daemon (`speake-rs-daemon`) over a Unix socket
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

## Documentation

- `docs/setup-speech-dispatcher.md` - global user setup for `spd-say`
- `docs/voice-cloning.md` - optional local profile cloning workflow
- `docs/gpu-cuda.md` - CUDA build/runtime notes
- `docs/troubleshooting.md` - common runtime and routing issues
