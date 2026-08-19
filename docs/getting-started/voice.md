# Voice input

Speak instead of typing. Press the record key, talk, press it again; the
transcription lands in the prompt where you can edit it before sending.

Off by default — typing is the primary input.

## Turn it on

```toml
# config.toml
[voice]
enabled = true
stt_provider = "openai"
stt_url = "https://api.openai.com"
```

Put the key somewhere that is not committed:

```toml
# config.local.toml
[voice]
stt_api_key = "sk-..."
```

Then restart. `/voice on` writes `enabled = true` for you and tells you the
same thing.

Check it came up with `/voice`:

```
Voice configuration:
  enabled:       true
  device:        default
  stt_provider:  openai
  stt_api_key:   (set)
  stt_url:       https://api.openai.com
  vad_threshold: 0.02
  hotkey:        ctrl+v
  toggle_mode:   false
```

If voice is enabled but cannot run, archon says so on stderr at startup and
carries on without it. It does not start a pipeline that cannot hear anything —
that is exactly what it used to do, and the symptom was voice input that never
failed and never worked.

## Using it

`Ctrl+V` starts a recording. What the second press does depends on
`toggle_mode`:

- `toggle_mode = false` (default) — push-to-talk. One press records for a
  two-second window and finalises itself.
- `toggle_mode = true` — press once to start, again to stop.

The capture overlay opens on its own when a recording starts, because a
recording with no visible indicator is how you end up talking to a microphone
that is not listening. It shows a live level meter and, once the recording
stops, the loudest moment measured against the VAD threshold:

```
┌─ Voice — Ctrl+V record/stop · Esc cancel · Enter close ──────────────┐
│ ○ stopped  peak 0.184 (above the 0.020 threshold)                    │
│                                                                       │
│  ▁▂▄▆█▇▅▃▂▁▁▂▃▅▇█▆▄▂▁                                                │
│ older                                                            now  │
│                                                                       │
│ add the parser to the tokenizer module                                │
└───────────────────────────────────────────────────────────────────────┘
```

`Esc` cancels a live recording and closes. `Enter` closes a finished one.
`/voice` opens the same overlay without recording, to look at the last one.

## When nothing happens

A recording whose loudest moment is below `vad_threshold` is discarded without
being transcribed. That is the intended behaviour — it stops a room's ambient
noise being sent to a speech API every time you brush the key — but it is also
the most common reason voice appears to do nothing.

The overlay answers it directly: if the peak is below the threshold, it says so
and says the recording will be discarded. Either speak up, move closer, or lower
`vad_threshold`.

## Choosing a microphone

`device = "default"` uses the system default. To pick another, give its exact
name:

```toml
[voice]
device = "Yeti Stereo Microphone"
```

An unknown name is an error listing the devices that do exist. It does not fall
back to the default: recording the wrong room is worse than not starting.

## Speech-to-text backends

| `stt_provider` | What it talks to |
|---|---|
| `"openai"` | OpenAI Whisper (`/v1/audio/transcriptions`). Needs `stt_api_key`. |
| `"local"` | Any HTTP endpoint at `stt_url` that accepts WAV bytes and returns `{"text": "..."}`. A `whisper.cpp` server is the usual choice. |

Anything else, or `"openai"` with no key, transcribes to a placeholder and logs
a warning — the pipeline runs, so you can see the meter and confirm the
microphone works, but nothing is recognised.

For a local backend:

```toml
[voice]
stt_provider = "local"
stt_url = "http://127.0.0.1:8080/inference"
```

Audio reaches the backend as 16 kHz mono 32-bit-float WAV, whatever the device
natively produces.

## Building with audio support

Microphone capture is the `audio-capture` feature, on by default. It costs
nothing on Windows (WASAPI) or macOS (CoreAudio). On Linux it needs ALSA headers
at build time:

```bash
sudo apt-get install -y libasound2-dev
```

`scripts/install-system-deps.sh` installs the right package for Debian/Ubuntu,
Fedora/RHEL, Amazon Linux, Arch, openSUSE and Alpine.

A build without the feature still runs; `/voice on` then reports that this
binary has no microphone support and names the flag.

### WSL2

WSL2 has no audio device by default. Either run archon from Windows, or set up
PulseAudio/PipeWire forwarding into the WSL instance — `/voice` will report the
device once ALSA can see one.

## Known limits

- **The hotkey is not configurable.** `voice.hotkey` is reported by `/voice` and
  read by nothing else; the binding is fixed at `Ctrl+V`. The default was
  `"ctrl+shift+v"` until v1.9.3, which described a key that had never been
  bound.
- **A recording is capped at five minutes** and says so if it hits the ceiling.
- **There is no text-to-speech.** Voice is input only.

## See also

- [`[voice]` configuration](../reference/config.md#voice)
- [Slash commands](../reference/slash-commands.md) — `/voice`
