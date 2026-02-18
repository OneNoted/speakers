# Troubleshooting

## Daemon not reachable

```bash
speake-rs daemon status
```

If unreachable, run in foreground to inspect logs:

```bash
speake-rs daemon start --foreground
```

## `spd-say` uses the wrong module or default robotic voice

Check:

- `~/.config/speech-dispatcher/speechd.conf` contains `DefaultModule speake-rs`
- `~/.config/speech-dispatcher/speechd.conf` contains `AddModule "speake-rs" ...`
- `~/.config/speech-dispatcher/modules/speake-rs-generic.conf` exists

Then restart Speech Dispatcher and retest `spd-say`.

## CPU fallback when GPU expected

- Ensure build/install used `--features cuda`
- Verify CUDA runtime and driver availability
- Check `speake-rs daemon status` and inspect `device:` field

## Profile errors

- Profile names must use `[A-Za-z0-9_-]`
- Base model is required for `profile:*` synthesis
- Check profile files under `$XDG_DATA_HOME/speake-rs/voices/`
- `icl_not_allowed` means `speech_dispatcher.allow_icl = false` and an ICL profile was requested
- Set `speech_dispatcher.fallback_profile` to a valid local profile name for safer `spd-synth` fallback
