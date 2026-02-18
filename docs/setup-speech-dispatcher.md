# Speech Dispatcher Setup (User-Global)

This setup makes `spd-say` use `speake-rs` as the default TTS module for your user account.

The default/public path uses preset voices on the `custom-voice` model.
No cloned profiles are required.

## 1) Build and install binaries

```bash
cargo install --path crates/speake-rs-cli --force
cargo install --path crates/speake-rs-daemon --force
```

For CUDA acceleration:

```bash
cargo install --path crates/speake-rs-cli --force --features cuda
cargo install --path crates/speake-rs-daemon --force --features cuda
```

## 2) Install Speech Dispatcher module config

```bash
mkdir -p ~/.config/speech-dispatcher/modules
cp packaging/speech-dispatcher/speake-rs-generic.conf ~/.config/speech-dispatcher/modules/
```

## 3) Enable `speake-rs` as default Speech Dispatcher module

Append contents of `packaging/speech-dispatcher/speechd.user.conf.snippet` to:

```text
~/.config/speech-dispatcher/speechd.conf
```

Required lines:

```text
AddModule "speake-rs" "sd_generic" "speake-rs-generic.conf" "/tmp/speake-rs-module.log"
DefaultModule speake-rs
LanguageDefaultModule "en" "speake-rs"
```

Important:
- Speech Dispatcher often starts with a minimal `PATH`.
- Keep `packaging/speech-dispatcher/speake-rs-generic.conf` as provided, or ensure the `speake-rs` binary path resolves correctly.
- If routing fails, inspect `/tmp/speake-rs-module.log` and `$TMPDIR/speake-rs-spd-errors.log`.

## 4) Run daemon

Default/public path (preset voices):

```bash
speake-rs-daemon --model custom-voice
```

Or run as a user service:

```bash
mkdir -p ~/.config/systemd/user
cp packaging/systemd/speake-rs.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now speake-rs.service
```

## 5) Restart Speech Dispatcher session

```bash
systemctl --user restart speech-dispatcher.service 2>/dev/null || true
pkill -f speech-dispatcher || true
speech-dispatcher
```

## 6) Verify global routing

```bash
speake-rs doctor
spd-say "speake-rs is now default"
```

If `spd-say` speaks through the daemon, setup is complete.

## Optional: use your own cloned profile

This project does not ship any cloned/custom profile.
If you want one, create it locally first:

```bash
speake-rs clone create --name sample_voice --ref-audio /path/to/reference.wav
```

Then run daemon in base model mode and map symbolic voices to your profile:

```toml
[daemon]
model = "base"
request_timeout_ms = 60000
synthesis_timeout_ms = 90000

[speech_dispatcher]
allow_icl = false
fallback_profile = "sample_voice"

[speech_dispatcher.voice_map]
MALE1 = "profile:sample_voice"
```

Use profile mapping only after confirming profile synthesis works directly.
