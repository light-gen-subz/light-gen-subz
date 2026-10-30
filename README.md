<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="LightGenSubz" width="128" />
</p>

# LightGenSubz

Turn any video or audio file into a subtitle track. Pick a file, generate, get an `.srt` with accurate timestamps — offline by default.

![license](https://img.shields.io/badge/license-MIT-blue) ![platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS-lightgrey)

---

## Features

- **Offline speech-to-text** — no account, and your media is never uploaded anywhere (unless you pick a cloud engine yourself)
- **Optional cloud transcription** — Groq, OpenAI, Deepgram or AssemblyAI, if you would rather not run locally
- **Multilingual** — the language is auto-detected
- **Optional translation** — offline (local NLLB-200 model) or via DeepL, OpenAI, Google Translate or Azure Translator
- **Standard `.srt` output** — ready to drop into any video editor, with a `.vtt` export for the web
- **Drag and drop** a file onto the window, pick the audio track of a multi-language film, choose the Whisper model (base / small / medium) and force the spoken language when detection gets it wrong
- **Edit the cues** in the app and export as `.srt`, `.vtt` or `.ass`
- **Native desktop app** — small and fast

---

## Install

One command, identical on macOS and Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/light-gen-subz/light-gen-subz/main/install.sh | bash
```

| Platform | What it installs |
|----------|------------------|
| macOS | Homebrew cask `light-gen-subz/tap/light-gen-subz` |
| Linux — Debian / Ubuntu | `.deb` package |
| Linux — other distributions | `.AppImage` in `~/.local/bin`, registered in your applications menu |

Re-run the exact same command to upgrade to the latest release.

> Apple Silicon only on macOS (13.4 or later), x86_64 only on Linux, for now. The installer refuses other machines.

### Manual install

**macOS — Homebrew**

```bash
brew install --cask light-gen-subz/tap/light-gen-subz
```

**Linux — Debian / Ubuntu**

Download the `.deb` from the [latest release](https://github.com/light-gen-subz/light-gen-subz/releases/latest), then:

```bash
sudo apt install ./LightGenSubz_*_amd64.deb
```

**Linux — other distributions**

Download the `.AppImage` from the [latest release](https://github.com/light-gen-subz/light-gen-subz/releases/latest), then:

```bash
chmod +x LightGenSubz_*_amd64.AppImage
./LightGenSubz_*_amd64.AppImage
```

---

## Requirements

- [`ffmpeg`](https://ffmpeg.org) must be installed and on your `PATH` — it extracts audio from video files. The one-line installer sets it up for you on Linux.
- The first run downloads a whisper model (~190 MB) to `~/.local/share/light-gen-subz/models/` on Linux, or the equivalent app-data directory on macOS. Enabling local translation downloads an additional NLLB-200 model (~900 MB).
- **Linux:** requires glibc ≥ 2.39 (Ubuntu 24.04+, Debian 13+, Fedora 40+).
- **macOS:** requires macOS 13.4 or later (the bundled ONNX Runtime is built for it).
- Cloud engines require an API key from that provider, entered in the app's Settings panel and stored in your OS keychain. **A cloud engine uploads your audio to that provider** (as compressed audio, in chunks), and a cloud translator sends the subtitle text.
- Downloaded models are checked against a pinned SHA-256 before use.

---

## Usage

1. Open the app, choose a video or audio file.
2. Click **Generate subtitles**.
3. Once done, the subtitles are shown in the app (nothing is written to your disk yet). Use **Export** to save them where you want, as `.srt`, `.vtt` or `.ass`, the original, the translation or both.

The save dialog proposes `clip.srt` next to your video, and never silently replaces an existing file. Press **Cancel** to stop a run in progress.

To also remove downloaded models, settings and cached audio after uninstalling:

```bash
# Linux
rm -rf ~/.local/share/light-gen-subz ~/.config/light-gen-subz ~/.cache/com.sikander.light-gen-subz
# macOS
rm -rf ~/Library/Application\ Support/light-gen-subz ~/Library/Caches/com.sikander.light-gen-subz
```

---

## Uninstall

**macOS — Homebrew**

```bash
brew uninstall --cask light-gen-subz
brew untap light-gen-subz/tap
```

Add `--zap` to also remove settings, caches and application data:

```bash
brew uninstall --zap --cask light-gen-subz
```

**Linux — Debian / Ubuntu**

```bash
sudo apt remove light-gen-subz
```

> Older installs registered the package as `light-gen-sub-z`. If `apt` reports that
> `light-gen-subz` is not installed, use that name instead.

**Linux — AppImage**

```bash
rm ~/.local/bin/light-gen-subz.AppImage
rm ~/.local/share/applications/light-gen-subz.desktop
rm ~/.local/share/icons/hicolor/512x512/apps/light-gen-subz.png
update-desktop-database ~/.local/share/applications
```

---

## Updates

The app looks for a new version a few seconds after it starts and offers it in a banner, with what
it brings; *Settings → Check for updates* does the same on demand. After an update, a *What's new*
window opens once, and it is always available from *Settings*.

The look (colours, font, sizes, light and dark theme) and these screens are shared with
LightCutVidz and the other apps of the suite: they live in `src/suite/`, which is the same in
each app — change them everywhere or nowhere.

---

## Development

### Prerequisites

- [Rust](https://rustup.rs) stable
- [Node.js](https://nodejs.org) 22+
- `ffmpeg`, plus `cmake` and `clang` for the local whisper engine

### Setup

```bash
git clone https://github.com/light-gen-subz/light-gen-subz.git
cd light-gen-subz
npm install
```

### Checks

```bash
npm run typecheck && npm run lint && npm test           # frontend
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Two tests need the real models and are skipped by default
(`cargo test -- --ignored real_`): the NLLB translation (downloads ~900 MB) and a local
transcription (`LGS_TEST_AUDIO` and `LGS_TEST_MODEL` point at a speech sample and a ggml model).

### Run in dev mode

```bash
npm run tauri dev
```

### Build the packaged app

```bash
npm run tauri build   # release bundles (.deb / .AppImage / .rpm / .app)
```

---

## How it works

```
file → ffmpeg (audio in a temporary folder, deleted afterwards)
         local : 16 kHz mono WAV
         cloud : 32 kbit/s mono MP3, encoded once from the source
     → transcription
         local : voice-activity detection (Silero) finds the speech, then whisper.cpp
                 transcribes each stretch of speech on its own — silence and music are
                 skipped, and every word keeps its own time
         cloud : Groq / OpenAI (20-minute chunks cut in a pause), Deepgram, AssemblyAI;
                 word timings are requested too; a busy provider (429 / 5xx / network)
                 is retried with a growing delay
     → segmentation (cues of at most 84 columns / 7 s; cut at sentence ends and real
       pauses, never across a silence of 1.2 s or more; full-width scripts such as
       Chinese or Japanese are cut between characters)
     → .srt writer (two lines of at most 42 columns, 32 for full-width scripts)
     → (optional) translation, in batches, the end of the previous batch given as context
         local : NLLB-200 (ONNX)
         cloud : DeepL, OpenAI, Google Translate, Azure Translator
```

See `src-tauri/src/pipeline/` for the implementation.

Quitting the app during a run asks it to stop, and waits a few seconds for it to delete its
temporary audio and end ffmpeg.

---

## Changelog

[CHANGELOG.md](CHANGELOG.md) (English) and [CHANGELOG.fr.md](CHANGELOG.fr.md) (French) list the changes of every version and follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Keep both in step.

---

## License

MIT — see [LICENSE](LICENSE).
