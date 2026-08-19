# LightGenSubz

Turn any video or audio file into a subtitle track. Pick a file, generate, get an `.srt` with accurate timestamps — fully offline.

![CI](https://github.com/light-gen-subz/light-gen-subz/actions/workflows/ci.yml/badge.svg) ![license](https://img.shields.io/badge/license-MIT-blue) ![platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS-lightgrey)

**Website** — <https://light-gen-subz.github.io/light-gen-subz/>

---

## Features

- **Offline speech-to-text** — no account, and your media is never uploaded anywhere
- **Optional cloud transcription** — Groq, OpenAI, Deepgram or AssemblyAI, if you would rather not run locally
- **Multilingual** — the language is auto-detected
- **Optional translation** — fully offline (local NLLB-200 model) or via DeepL, OpenAI, Google Translate or Azure Translator
- **Standard `.srt` output** — ready to drop into any video editor
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

> Apple Silicon only for now.

### Manual install

**macOS — Homebrew**

```bash
brew install --cask light-gen-subz/tap/light-gen-subz
```

**Linux — Debian / Ubuntu**

Download the `.deb` from the [latest release](https://github.com/light-gen-subz/light-gen-subz/releases/latest), then:

```bash
sudo apt install ./light-gen-subz_*_amd64.deb
```

**Linux — other distributions**

Download the `.AppImage` from the [latest release](https://github.com/light-gen-subz/light-gen-subz/releases/latest), then:

```bash
chmod +x light-gen-subz_*_amd64.AppImage
./light-gen-subz_*_amd64.AppImage
```

---

## Requirements

- [`ffmpeg`](https://ffmpeg.org) must be installed and on your `PATH` — it extracts audio from video files. The one-line installer sets it up for you on Linux.
- The first run downloads a whisper model (~190 MB) to `~/.local/share/light-gen-subz/models/` on Linux, or the equivalent app-data directory on macOS. Enabling local translation downloads an additional NLLB-200 model (~900 MB).
- **Linux:** requires glibc ≥ 2.38 (Ubuntu 24.04+, Debian 13+, Fedora 39+), needed by the bundled ONNX Runtime used for local translation.
- Cloud engines require an API key from that provider, entered in the app's Settings panel and stored in your OS keychain.

---

## Usage

1. Open the app, choose a video or audio file.
2. Click **Generate subtitles**.
3. Once done, the `.srt` is written next to your source file and previewed in the app. Use **Save as…** to save it elsewhere.

To also remove downloaded models and settings after uninstalling:

```bash
rm -rf ~/.local/share/light-gen-subz ~/.config/light-gen-subz
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

## Development

### Prerequisites

- [Rust](https://rustup.rs) stable
- [Node.js](https://nodejs.org) 20+
- `ffmpeg`, plus `cmake` and `clang` for the local whisper engine

### Setup

```bash
git clone https://github.com/light-gen-subz/light-gen-subz.git
cd light-gen-subz
npm install
```

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
file → ffmpeg (extract to 16kHz mono WAV)
     → whisper.cpp or Groq API (transcription, language auto-detect)
     → segmentation (split overly long cues)
     → .srt writer
     → (optional) NLLB-200 (ONNX) or DeepL API → translated .srt
```

See `src-tauri/src/pipeline/` for the implementation.

---

## License

MIT — see [LICENSE](LICENSE).
