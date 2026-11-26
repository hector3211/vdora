# Action latency

## Reproduce the one-shot benchmark

Build release binaries from `8eb52b7` (the baseline) and from the candidate with `cargo build --release --locked`. Use the same local Whisper model and 16 kHz mono speech WAVs for both builds:

```sh
python3 scripts/bench_oneshot.py /path/to/baseline /path/to/candidate \
  ~/.local/share/vdora/models/ggml-base.en.bin short.wav long.wav --check
python3 scripts/bench_oneshot.py /path/to/baseline /path/to/candidate \
  ~/.local/share/vdora/models/ggml-base.en.bin short.wav long.wav --no-notify
```

The harness runs both shortcut invocations and real Whisper inference, alternates build order, discards a warmup per build and fixture, and checks for identical nonempty transcripts. It measures from invocation to fixture recorder readiness and from the stop invocation to the clipboard helper receiving the text. `--check` adds a 150 ms delay to recording and transcription notifications and asserts the candidate recorder starts without waiting for that delay. The harness substitutes the desktop helpers, so it does not require or measure a live microphone, compositor clipboard, notifications, or key injection.

## Results

Release builds on an AMD Ryzen 7 3700X, pinned to one hardware thread per core, using the default `ggml-base.en.bin`. Two synthetic English speech fixtures (3.3 s and 13.5 s), with 10 paired runs each using delayed notifications and six each with notifications disabled:

| Fixture | Shortcut → recorder, before → after | Stop → clipboard helper, before → after |
| --- | ---: | ---: |
| 3.3 s, 150 ms notification delay | 253 → 64 ms | 1,907 → 998 ms |
| 13.5 s, 150 ms notification delay | 255 → 64 ms | 2,166 → 1,264 ms |
| 3.3 s, notifications disabled | 64 → 64 ms | 1,730 → 1,002 ms |
| 13.5 s, notifications disabled | 66 → 64 ms | 2,065 → 1,280 ms |

These are medians. The stop-to-copy timing ranges did not overlap, and baseline and candidate transcripts matched exactly. Notifications no longer gate capture or inference, and English-only models avoid an extra language-detection encoder pass.

## Other checks

- Headless GTK/Broadway smoke test: invoked the real GTK record action over a private D-Bus in both builds and verified the fixture transcript reached the clipboard helper on each cycle. With both builds explicitly set to English, the inference-finished-to-copy median dropped from **51 ms to 22 ms** (10 measured cycles/build). This isolates GUI delivery from the model optimization; it is not a measurement of native compositor latency.
- A Spanish fixture with Whisper Tiny multilingual produced identical baseline and candidate transcripts with both automatic detection and explicit `es`. No multilingual speedup is claimed.
- Tested Whisper flash attention separately on the same CPU; it was slower on both English fixtures, so it was not enabled.
