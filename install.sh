#!/usr/bin/env bash
# Builds omatouchbar and installs it for the current user (~/.local by
# default), then selects it as the T1Bridge Touch Bar renderer.
#
#   ./install.sh               build, install, enable
#   ./install.sh --no-enable   build and install only
#   PREFIX=/opt/x ./install.sh install somewhere else
set -euo pipefail
cd "$(dirname "$0")"

PREFIX=${PREFIX:-$HOME/.local}
CONFIG_HOME=${XDG_CONFIG_HOME:-$HOME/.config}
enable=1
[[ ${1:-} == --no-enable ]] && enable=0

cargo build --release --locked -p omatouchbar -p omatouchbar-settings

install -Dm755 target/release/omatouchbar "$PREFIX/bin/omatouchbar"
install -Dm755 target/release/omatouchbar-settings "$PREFIX/bin/omatouchbar-settings"
install -Dm644 packaging/dev.omatouchbar.Settings.svg \
  "$PREFIX/share/icons/hicolor/scalable/apps/dev.omatouchbar.Settings.svg"
install -d "$PREFIX/share/applications"
sed "s|^Exec=.*|Exec=$PREFIX/bin/omatouchbar-settings|" packaging/dev.omatouchbar.Settings.desktop \
  > "$PREFIX/share/applications/dev.omatouchbar.Settings.desktop"
gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true

# Register Touch Bar under Omarchy's Setup menu (keeping existing extensions intact).
menu_ext="$CONFIG_HOME/omarchy/extensions/omarchy-menu.jsonc"
install -d "$(dirname "$menu_ext")"
python3 - "$menu_ext" "$PREFIX/bin/omatouchbar-settings" <<'PY'
import json, os, re, shlex, sys, tempfile
from pathlib import Path

path = Path(sys.argv[1])
settings_bin = sys.argv[2]
quoted = shlex.quote(settings_bin)
entry = {
    "icon": "󰆽",
    "label": "Touch Bar",
    "after": "setup.typing",
    "aliases": ["touchbar", "touch-bar"],
    "when": f"test -x {quoted}",
    "action": f'omarchy-launch-or-focus dev.omatouchbar.Settings "uwsm-app -- {quoted}"',
    "description": "Configure the MacBook Pro Touch Bar",
}
line = f'  "setup.touch-bar": {json.dumps(entry, ensure_ascii=False)},'

if path.exists():
    text = path.read_text(encoding="utf-8")
else:
    template = Path("/usr/share/omarchy/config/omarchy/extensions/omarchy-menu.jsonc")
    text = template.read_text(encoding="utf-8") if template.exists() else "{\n}\n"

# Remove any existing setup.touch-bar line, then insert right after the opening brace.
lines = [l for l in text.splitlines() if not re.match(r'^\s*"setup\.touch-bar"\s*:', l)]
if len(lines) == 1 and re.match(r'^\s*\{\s*\}\s*$', lines[0]):
    lines = ["{", line, "}"]
else:
    inserted = False
    for i, l in enumerate(lines):
        if re.match(r'^\s*\{\s*$', l):
            lines.insert(i + 1, line)
            inserted = True
            break
        if "{" in l:
            pos = l.index("{")
            before, after = l[:pos + 1], l[pos + 1:].strip()
            replacement = [before, line] + ([f"  {after}"] if after else [])
            lines[i:i + 1] = replacement
            inserted = True
            break
    if not inserted:
        lines = ["{", line, "}"]
updated = "\n".join(lines) + "\n"

# Validate with Omarchy MenuModel.js's exact JSONC stripping rules before saving.
stripped = re.sub(r'^\s*//[^\n]*(\n|$)', '', updated, flags=re.MULTILINE)
stripped = re.sub(r',(\s*[}\]])', r'\1', stripped)
parsed = json.loads(stripped)
assert "setup.touch-bar" in parsed

fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=".omarchy-menu.", suffix=".tmp")
try:
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(updated)
    os.replace(tmp, path)
except Exception:
    os.unlink(tmp)
    raise
PY

echo "Installed omatouchbar to $PREFIX/bin."
if (( enable )); then
  "$PREFIX/bin/omatouchbar" enable
fi
