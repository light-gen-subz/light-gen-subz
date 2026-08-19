#!/usr/bin/env bash
# light-gen-subz — one-line uninstaller for macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/light-gen-subz/light-gen-subz/main/uninstall.sh | bash
#
#   macOS         → Homebrew cask, or the .app bundle if it was installed by hand
#   Linux + apt   → .deb package
#   Linux (other) → AppImage, its menu entry and its icon
#
# Mirrors install.sh: every install route it can create, this can undo. Removing
# something that was never installed is not an error — the script reports what it
# actually found.
#
# User settings and projects are left untouched.
set -eo pipefail

REPO="light-gen-subz/light-gen-subz"
TAP="light-gen-subz/tap"
CASK="light-gen-subz"
APP_NAME="light-gen-subz"
APP_SLUG="light-gen-subz"
DEB_PACKAGE="light-gen-subz"
ICON_SIZE="512x512"

removed=0

log() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

uninstall_macos() {
  if command -v brew >/dev/null 2>&1 && brew list --cask "$CASK" >/dev/null 2>&1; then
    log "Removing $APP_NAME via Homebrew…"
    brew uninstall --cask "$CASK"
    removed=1
  fi

  # Installed by hand rather than through Homebrew.
  app="/Applications/$APP_NAME.app"
  if [ -d "$app" ]; then
    log "Removing $app…"
    rm -rf "$app"
    removed=1
  fi
}

uninstall_deb() {
  command -v dpkg-query >/dev/null 2>&1 || return 0
  dpkg-query -W -f='${Status}' "$DEB_PACKAGE" 2>/dev/null | grep -q "^install ok installed$" || return 0
  log "Removing the $DEB_PACKAGE package (sudo required)…"
  sudo apt-get remove -y "$DEB_PACKAGE"
  removed=1
}

uninstall_appimage() {
  bin="$HOME/.local/bin/$APP_SLUG.AppImage"
  desktop="$HOME/.local/share/applications/$APP_SLUG.desktop"
  icon="$HOME/.local/share/icons/hicolor/$ICON_SIZE/apps/$APP_SLUG.png"

  found=0
  for f in "$bin" "$desktop" "$icon"; do
    [ -e "$f" ] || continue
    [ "$found" -eq 1 ] || log "Removing the AppImage install…"
    found=1
    note "$f"
    rm -f "$f"
    removed=1
  done

  [ "$found" -eq 1 ] && command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$HOME/.local/share/applications" 2>/dev/null
  return 0
}

echo "$APP_NAME uninstaller"

case "$(uname -s)" in
  Darwin)
    uninstall_macos
    ;;
  Linux)
    # A machine can carry either route — or both, after switching. Undo whatever is there.
    uninstall_deb
    uninstall_appimage
    ;;
  *)
    die "Unsupported platform: $(uname -s)."
    ;;
esac

if [ "$removed" -eq 1 ]; then
  log "Done. $APP_NAME has been removed."
  note "Your settings were left in place."
else
  log "Nothing to remove — $APP_NAME does not appear to be installed."
fi
