# Voice Cloning (Optional)

`speake-rs` can synthesize with user-created cloned profiles.

No cloned voices are bundled with this project.
You must provide your own reference audio (and optional transcript) locally.

## Create a profile

Basic:

```bash
speake-rs clone create --name sample_voice --ref-audio /path/to/reference.wav
```

Higher quality profile creation with transcript:

```bash
speake-rs clone create --name sample_voice --ref-audio /path/to/reference.wav --ref-text "reference transcript"
```

## List and inspect profiles

```bash
speake-rs clone list
speake-rs clone show --name sample_voice
```

## Use profile for direct synthesis

Start daemon with base model:

```bash
speake-rs-daemon --model base
```

Then synthesize:

```bash
speake-rs speak "hello" --profile sample_voice
```

## Use profile from Speech Dispatcher

Edit `~/.config/speake-rs/config.toml`:

```toml
[daemon]
model = "base"
synthesis_timeout_ms = 90000

[speech_dispatcher]
allow_icl = false
fallback_profile = "sample_voice"

[speech_dispatcher.voice_map]
MALE1 = "profile:sample_voice"
```

With daemon running in `base` mode, `spd-say` requests using `MALE1` route to your `sample_voice` profile.

## ICL profiles (advanced)

ICL profiles are optional and experimental. Keep this disabled for global stability unless you are actively testing:

```toml
[speech_dispatcher]
allow_icl = false
```

Enable only when you intentionally want ICL profile execution.
