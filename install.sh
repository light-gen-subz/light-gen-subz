#!/usr/bin/env bash
# light-gen-subz — one-line installer for macOS and Linux.
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
APP_NAME="light-gen-subz"
APP_SLUG="light-gen-subz"
APP_COMMENT="Offline subtitle generator"
CATEGORIES="AudioVideo;Video;"
ICON_URL="https://raw.githubusercontent.com/$REPO/main/src-tauri/icons/icon.png"
ICON_SIZE="512x512"
RUNTIME_DEPS="ffmpeg"

log() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# fetch <url> [output] — writes to <output>, or to stdout when omitted
fetch() {
  if command -v curl >/dev/null 2>&1; then
    if [ $# -ge 2 ]; then curl -fsSL "$1" -o "$2"; else curl -fsSL "$1"; fi
  elif command -v wget >/dev/null 2>&1; then
    if [ $# -ge 2 ]; then wget -qO "$2" "$1"; else wget -qO- "$1"; fi
  else
    die "curl or wget is required"
  fi
}

# latest_asset <extension> — download URL of the first matching asset of the latest release
latest_asset() {
  fetch "https://api.github.com/repos/$REPO/releases/latest" \
    | grep -o "\"browser_download_url\": *\"[^\"]*\\.$1\"" \
    | head -1 \
    | sed 's/.*"\(https[^"]*\)"/\1/'
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
    sudo apt-get update && sudo apt-get install -y $missing
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y $missing
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -S --noconfirm $missing
  elif command -v zypper >/dev/null 2>&1; then
    sudo zypper install -y $missing
  else
    log "No supported package manager found — install these manually:$missing"
  fi
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
  url="$(latest_asset deb)"
  [ -n "$url" ] || die "No .deb in the latest release — see https://github.com/$REPO/releases"
  tmp="$(mktemp -d)/$(basename "$url")"
  log "Downloading $(basename "$url")…"
  fetch "$url" "$tmp"
  log "Installing the .deb package (sudo required)…"
  sudo apt-get install -y "$tmp"
  rm -rf "$(dirname "$tmp")"
  log "Done. Launch $APP_NAME from your applications menu."
}

install_appimage() {
  url="$(latest_asset AppImage)"
  [ -n "$url" ] || die "No .AppImage in the latest release — see https://github.com/$REPO/releases"

  bin_dir="$HOME/.local/bin"
  desktop_dir="$HOME/.local/share/applications"
  icon_dir="$HOME/.local/share/icons/hicolor/$ICON_SIZE/apps"
  mkdir -p "$bin_dir" "$desktop_dir" "$icon_dir"

  target="$bin_dir/$APP_SLUG.AppImage"
  log "Downloading $(basename "$url")…"
  fetch "$url" "$target"
  chmod +x "$target"

  fetch "$ICON_URL" "$icon_dir/$APP_SLUG.png" 2>/dev/null || true

  cat > "$desktop_dir/$APP_SLUG.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_NAME
Comment=$APP_COMMENT
Exec=$target
Icon=$APP_SLUG
Terminal=false
Categories=$CATEGORIES
StartupWMClass=$APP_NAME
EOF

  update-desktop-database "$desktop_dir" 2>/dev/null || true

  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) log "Note: $bin_dir is not in your PATH — add it to your shell profile to launch from a terminal." ;;
  esac
  log "Done. Installed to $target — launch $APP_NAME from your applications menu."
}

echo "$APP_NAME installer"

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
