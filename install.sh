#!/usr/bin/env bash
# LightGenSubz — one-line installer for macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/light-gen-subz/light-gen-subz/main/install.sh | bash
#
#   macOS         → Homebrew cask (light-gen-subz/tap/light-gen-subz)
#   Linux + apt   → .deb package
#   Linux (other) → .AppImage in ~/.local/bin, registered in the applications menu
#
# Re-run the same command to upgrade an existing install.
set -eo pipefail

REPO="light-gen-subz/light-gen-subz"
TAP="light-gen-subz/tap"
CASK="light-gen-subz"
APP_NAME="LightGenSubz"
APP_SLUG="light-gen-subz"
APP_COMMENT="Offline subtitle generator"
CATEGORIES="AudioVideo;Video;"
ICON_URL="https://raw.githubusercontent.com/$REPO/main/src-tauri/icons/icon.png"
ICON_SIZE="512x512"
RUNTIME_DEPS="ffmpeg"

log() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# fetch <url> [output] — writes to <output>, or to stdout when omitted.
# GITHUB_TOKEN, when set, is sent to the GitHub API (60 requests an hour otherwise, shared
# by everyone behind the same address).
fetch() {
  if command -v curl >/dev/null 2>&1; then
    auth=""
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      case "$1" in https://api.github.com/*) auth="Authorization: Bearer $GITHUB_TOKEN" ;; esac
    fi
    if [ $# -ge 2 ]; then curl -fsSL ${auth:+-H "$auth"} "$1" -o "$2"; else curl -fsSL ${auth:+-H "$auth"} "$1"; fi
  elif command -v wget >/dev/null 2>&1; then
    if [ $# -ge 2 ]; then wget -qO "$2" "$1"; else wget -qO- "$1"; fi
  else
    die "curl or wget is required"
  fi
}

# The latest release, read once: every asset below comes from the same release, even if a
# new one is published while this script runs.
RELEASE_JSON=""
release_json() {
  if [ -z "$RELEASE_JSON" ]; then
    RELEASE_JSON="$(fetch "https://api.github.com/repos/$REPO/releases/latest")" \
      || die "Could not read the latest release from the GitHub API (offline, or rate limited — set GITHUB_TOKEN to raise the limit)."
  fi
  printf '%s' "$RELEASE_JSON"
}

# latest_asset <extension> — download URL of the first matching asset of the latest release
latest_asset() {
  release_json \
    | grep -o "\"browser_download_url\": *\"[^\"]*\\.$1\"" \
    | head -1 \
    | sed 's/.*"\(https[^"]*\)"/\1/'
}

# sha256 <file> — hex digest, with whichever tool the system has
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# verify_download <file> — checks it against the release's SHA256SUMS, when published
verify_download() {
  sums_url="$(release_json \
    | grep -o "\"browser_download_url\": *\"[^\"]*/SHA256SUMS\"" | head -1 \
    | sed 's/.*"\(https[^"]*\)"/\1/')" || sums_url=""
  if [ -z "$sums_url" ]; then
    log "Warning: this release has no SHA256SUMS — the download cannot be verified."
    return 0
  fi
  expected="$(fetch "$sums_url" | awk -v f="$(basename "$1")" '$2 == f || $2 == "*" f { print $1 }')"
  [ -n "$expected" ] || die "$(basename "$1") is not listed in SHA256SUMS."
  [ "$(sha256 "$1")" = "$expected" ] || die "Checksum mismatch for $(basename "$1") — refusing to install it."
  log "Checksum OK."
}

# try_install <package>… — installs the first of the packages the system knows. A failure
# is reported, not fatal: the app itself is still worth installing.
try_install() {
  for pkg in "$@"; do
    if "${PKG_INSTALL[@]}" "$pkg"; then return 0; fi
  done
  return 1
}

install_runtime_deps() {
  [ -n "$RUNTIME_DEPS" ] || return 0
  missing=""
  for dep in $RUNTIME_DEPS; do
    command -v "$dep" >/dev/null 2>&1 || missing="$missing $dep"
  done
  [ -n "$missing" ] || return 0
  log "Installing runtime dependencies:$missing"
  if command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update || true
    PKG_INSTALL=(sudo apt-get install -y)
  elif command -v dnf >/dev/null 2>&1; then
    # Fedora ships ffmpeg-free; the full ffmpeg is in RPM Fusion.
    PKG_INSTALL=(sudo dnf install -y)
  elif command -v pacman >/dev/null 2>&1; then
    PKG_INSTALL=(sudo pacman -S --noconfirm)
  elif command -v zypper >/dev/null 2>&1; then
    PKG_INSTALL=(sudo zypper install -y)
  else
    log "No supported package manager found — install these manually:$missing"
    return 0
  fi
  for dep in $missing; do
    case "$dep" in
      ffmpeg) try_install ffmpeg ffmpeg-free ;;
      *) try_install "$dep" ;;
    esac || log "Could not install $dep — install it yourself, the app needs it."
  done
}

install_macos() {
  command -v brew >/dev/null 2>&1 \
    || die "Homebrew is required on macOS. Install it from https://brew.sh"
  brew tap "$TAP"
  if brew list --cask "$CASK" >/dev/null 2>&1; then
    log "Upgrading $APP_NAME via Homebrew…"
    brew upgrade --cask "$TAP/$CASK"
  else
    log "Installing $APP_NAME via Homebrew…"
    brew install --cask "$TAP/$CASK"
  fi
  log "Done. Launch $APP_NAME from Spotlight or your Applications folder."
}

install_deb() {
  release_json >/dev/null # read in this shell, so the lookups below reuse it
  url="$(latest_asset deb)"
  [ -n "$url" ] || die "No .deb in the latest release — see https://github.com/$REPO/releases"
  tmp="$(mktemp -d)/$(basename "$url")"
  log "Downloading $(basename "$url")…"
  fetch "$url" "$tmp"
  verify_download "$tmp"
  log "Installing the .deb package (sudo required)…"
  sudo apt-get install -y "$tmp"
  rm -rf "$(dirname "$tmp")"
  log "Done. Launch $APP_NAME from your applications menu."
}

install_appimage() {
  release_json >/dev/null
  url="$(latest_asset AppImage)"
  [ -n "$url" ] || die "No .AppImage in the latest release — see https://github.com/$REPO/releases"

  bin_dir="$HOME/.local/bin"
  desktop_dir="$HOME/.local/share/applications"
  icon_dir="$HOME/.local/share/icons/hicolor/$ICON_SIZE/apps"
  mkdir -p "$bin_dir" "$desktop_dir" "$icon_dir"

  target="$bin_dir/$APP_SLUG.AppImage"
  log "Downloading $(basename "$url")…"
  fetch "$url" "$target"
  verify_download "$target" || { rm -f "$target"; exit 1; }
  chmod +x "$target"

  fetch "$ICON_URL" "$icon_dir/$APP_SLUG.png" 2>/dev/null || true

  cat > "$desktop_dir/$APP_SLUG.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_NAME
Comment=$APP_COMMENT
Exec="$target"
Icon=$APP_SLUG
Terminal=false
Categories=$CATEGORIES
StartupWMClass=$APP_SLUG
EOF

  update-desktop-database "$desktop_dir" 2>/dev/null || true

  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) log "Note: $bin_dir is not in your PATH — add it to your shell profile to launch from a terminal." ;;
  esac
  log "Done. Installed to $target — launch $APP_NAME from your applications menu."
}

echo "$APP_NAME installer"

# The releases ship one build per platform: refuse the others up front rather than
# installing a binary that cannot run.
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64|Linux-x86_64) ;;
  Darwin-*) die "Only Apple Silicon Macs are supported for now (this machine is $(uname -m))." ;;
  Linux-*) die "Only x86_64 Linux is supported for now (this machine is $(uname -m))." ;;
esac

case "$(uname -s)" in
  Darwin)
    install_macos
    ;;
  Linux)
    install_runtime_deps
    if command -v apt-get >/dev/null 2>&1; then
      install_deb
    else
      install_appimage
    fi
    ;;
  *)
    die "Unsupported platform: $(uname -s). Download manually from https://github.com/$REPO/releases"
    ;;
esac
