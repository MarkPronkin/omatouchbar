#!/usr/bin/env bash
# Restores the previous Touch Bar renderer and the on-screen Omarchy bar,
# then removes the files installed by install.sh. Settings in
# ~/.config/omatouchbar are kept.
set -euo pipefail
PREFIX=${PREFIX:-$HOME/.local}
CONFIG_HOME=${XDG_CONFIG_HOME:-$HOME/.config}

if [[ -x $PREFIX/bin/omatouchbar ]]; then
  "$PREFIX/bin/omatouchbar" disable || true
  "$PREFIX/bin/omatouchbar" restore-bar || true
fi
rm -f "$PREFIX/bin/omatouchbar" "$PREFIX/bin/omatouchbar-settings" \
  "$PREFIX/share/icons/hicolor/scalable/apps/dev.omatouchbar.Settings.svg" \
  "$PREFIX/share/applications/dev.omatouchbar.Settings.desktop"
gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true

menu_ext="$CONFIG_HOME/omarchy/extensions/omarchy-menu.jsonc"
if [[ -f $menu_ext ]]; then
  python3 - "$menu_ext" <<'PY'
import json, os, re, sys, tempfile
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
lines = [l for l in text.splitlines() if not re.match(r'^\s*"setup\.touch-bar"\s*:', l)]
updated = "\n".join(lines) + "\n"
if updated != text:
    stripped = re.sub(r'^\s*//[^\n]*(\n|$)', '', updated, flags=re.MULTILINE)
    stripped = re.sub(r',(\s*[}\]])', r'\1', stripped)
    json.loads(stripped)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=".omarchy-menu.", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(updated)
        os.replace(tmp, path)
    except Exception:
        os.unlink(tmp)
        raise
PY
fi

echo "Removed omatouchbar. Settings remain in ~/.config/omatouchbar."
