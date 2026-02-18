# CUDA Notes

`speake-rs-daemon` uses `qwen3-tts::auto_device()` at startup.

- If CUDA is available and the binary is built with `--features cuda`, GPU should be selected.
- If CUDA is unavailable, daemon logs a warning and falls back to CPU.

## Build with CUDA

```bash
cargo build --workspace --features cuda
```

## Runtime check

```bash
speake-rs daemon status
```

Inspect `device:` in output.
