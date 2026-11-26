# omatouchbar

omatouchbar is an MIT-licensed Touch Bar implementation for Omarchy, written
in Rust. Use the strip as a function row or bring Omarchy’s bar, menus, and
controls onto it. A companion settings app lets you configure either mode
and preview it on the desktop.

## Hardware scope

The current target is the 2016–2017 MacBook Pro with a T1 chip and Touch Bar
(for example, `MacBookPro14,3`). The renderer talks to the hardware through
T1Bridge. MacBook Pro models with a Touch Bar and a T2, M1, or M2 chip are in
scope for future support.

## Features

- **Keyboard mode** (the default) turns the strip into a keyboard's
  function row: Esc, then brightness, keyboard light, previous /
  play-pause / next and mute / volume keys (hold to repeat), with a player
  on the right. The player shows what's playing (or your output when
  nothing is) and opens the full Sound panel when tapped; Back or the panel
  timeout returns to the keys. A thin meter under a group shows its level
  for a moment after it changes. Holding Fn shows F1–F12, and Touch ID
  takes over the strip as in every mode. Switch modes in Touch Bar
  settings.
- **Omarchy Bar mode** — moves the Omarchy top bar onto the Touch Bar and
  (optionally) hides the on-screen one. It mirrors the widget layout in
  `~/.config/omarchy/shell.json`: Omarchy menu, workspaces, indicators,
  clock, world clocks, tray, weather, agents, system update, keyboard layout, Bluetooth, network,
  volume, brightness and battery, plus optional now-playing. **Esc** is always pinned
  to the left edge.
- **Horizontal Omarchy menu** — the logo opens Omarchy's own menu
  (`omarchy-menu.jsonc` plus your extension) as a swipeable row; submenus
  slide in, `when`/`checked` conditions are evaluated, actions run exactly
  as they would from the on-screen menu.
- **Panels** grow out of their widgets: Sound with Omarchy's Output, Input
  and Sources sections (output volume, output device picker and media
  transport; microphone volume, mute and input device picker; per-app volume
  and mute),
  Display controls, battery details like Omarchy's panel
  (time left/to full, charge/discharge rate, battery size, charge cycles,
  charge limit) with power profiles, network status like Omarchy's panel (Wi-Fi
  toggle, captive portal sign-in, QR share, speed test, live ping, packet loss,
  receiving/sending rates, downloaded/uploaded totals, copyable IP address and
  gateway, Wi-Fi band selector, DNS selector, and Wi-Fi networks), Bluetooth
  like Omarchy's panel (see below), the calendar and Elsewhen world clocks, and quick-toggle indicators
  (Passwordless Sudo, Screen Recording, Reminders, Night Light, Do Not Disturb,
  Stay Awake, and active Remote Sessions — or keep all indicators inline on the
  bar via `alwaysShow` in `shell.json` or Touch Bar settings).
- **Display** opens from the brightness widget or `omatouchbar ctl panel display`.
  Adjust brightness on the focused monitor and the keyboard backlight, choose
  a desktop text size (9, 10, 11, 12, 14, 16, or 20 px), or tap a scale preset.
  Scale choices follow the focused screen’s resolution. With multiple monitors
  connected, swipe along the row to turn individual outputs on or off. The
  last enabled display stays on. Fixed-brightness monitors still offer scale
  and text size.
- **Time** opens from the clock, the globe, or `omatouchbar ctl panel time`.
  Tap **Add a city** and type a city or time zone. Pick a match or press Return.
  Tap **People**, choose a city, and enter comma-separated names or GitHub
  `@handles`. Return saves and Esc cancels. Names and avatars appear beside
  the city. Tap a city to reorder or remove it, see its local greeting and
  daylight, or drag the daylight strip to compare times. **Globe** lets you
  rotate the Earth and jump to another city. **Where are you?** sets home.
  City edits, people, hour format, offsets, and temperature units are shared
  with Omarchy’s Elsewhen widget. With no desktop widget, the Touch Bar saves
  its own list. The date opens a six-week calendar with month and year
  navigation, week-start settings, and year and life progress.
- **Bluetooth** has the power switch, then connected and paired devices with
  what they are and their battery level. Tap a device to connect or disconnect
  it, or its trash can twice to forget it. While the panel is open it scans
  for new devices, the same way Omarchy's panel does: put one in pairing mode
  and tap it when it shows up to pair, trust and connect it. Audio then
  switches to it. Each device shows its progress ("Pairing…") and says so if
  it fails. The scan stops when the panel closes, which it does a minute after
  you last touch the strip, so it can't keep running and make Bluetooth audio
  stutter. Pairing goes through Omarchy's `omarchy-bluetooth-device` and its
  pairing agent.
- **Tray** lists registered StatusNotifier apps, puts pinned apps first and
  honors hidden items. Tap an app to activate it or its chevron to browse
  its menu and submenus on the strip.
- **Weather** shows Omarchy’s condition icon with temperature on the bar. Its
  panel has everything Omarchy’s weather panel shows — conditions, feels like,
  wind, humidity and the forecast — plus today’s high and low, rain chance, the
  next 12 hours, a 6-day forecast, sunrise, sunset and UV index. Tap the
  location to change it: type a city on the keyboard and pick a match (Return
  takes the first), or tap **Auto** to follow your network’s location, exactly
  like the desktop panel (both share Omarchy’s `weather.json`). Units follow
  Omarchy (the widget’s `unit` setting, else the place’s country, else your
  locale). **Refresh** updates the report; **Show on desktop** opens Omarchy’s
  panel. Data comes from Open-Meteo, as in Omarchy, with Omarchy’s helpers as
  a fallback.
- **Agents** mirrors Omarchy’s Agents panel, from the same usage records and
  in the order you dragged the agents into on the desktop. Each subscription
  gets its logo, name, plan and tokens used today, then a meter per limit
  window (Session, Weekly, Monthly, named the same way for every agent) with
  the time until it resets. A meter turns red at 90%, and model-scoped
  allowances such as a Fable weekly limit show as a tick on their window’s
  meter. Banked free resets, prepaid balances (a meter that drains) and
  several accounts per agent are shown too: tap **Use** to switch accounts,
  or **Sign in** when a sign-in has lapsed. Limits that couldn’t refresh fade
  and say why. After the agents come today’s tokens, prompts and sessions
  across all of them, this week’s total, the most used model and the busiest
  day. **Refresh** asks Omarchy to update the records and shimmers until they
  land; **Start agent** launches your default agent; **Show on desktop** opens
  Omarchy’s panel. The bar icon turns orange when any agent is nearly out.
- **Any script**: characters missing from the bar font (Japanese, Chinese,
  Korean, Arabic, …) fall back per character through fontconfig, honoring
  `omarchy font cjk`; color emoji render in color.
- **Fn** shows F1–F12 while held. Other modes: classic media controls, or
  function keys by default.
- **Touch ID** prompts from T1Bridge take over the strip with an animation in
  the spirit of Omarchy's screensaver, anchored on the real sensor (the
  power key just past the right end of the Touch Bar). A dot-matrix
  fingerprint swarms into place beside it, behind chevrons pointing at the
  key, and a field of scrambling glyphs boots out of the sensor and fills
  the strip. Everything moves outward from the sensor. While it waits, a
  scanner beam sweeps the print and, when authenticating, the sensor pings:
  the end of the strip flashes, the chevrons light up and a wave runs down
  the whole strip. While enrolling nothing pulses on a timer; every pulse is
  a step of progress, and the whole Touch Bar is the progress bar: it fills out of
  the sensor end with a glowing leading edge, rails along the top and bottom
  and a dotted track ahead, and each step's slice flashes in with a
  shockwave and sparks. A miss shakes, glitches and burns red out of the
  sensor. Success blows the print apart into block floods racing down the
  strip, and a check mark draws itself. Status text decrypts itself,
  everything uses your theme's colors, and with motion off it settles to a
  still frame. `omatouchbar ctl touchid [unlock|enroll]` plays a demo.
- **Motion**: every node springs between layouts (position, size, color,
  emphasis), with staggered entrances, ink ripples, a sliding workspace pill,
  rolling clock digits and inline level meters when volume/brightness change.
  Speed, bounce, stagger and frame rate are adjustable.
- **Follows your Omarchy theme live**: switch themes and the new one opens
  out of the middle of the strip in the same slanted wipe the desktop plays
  on the wallpaper, starting with it. When the new wallpaper has an intro
  video, it starts as the video starts. It dims when idle (OLED-friendly)
  and uses no CPU while nothing moves.

## Requirements

- A T1 MacBook Pro running [Omarchy](https://omarchy.org) with
  [T1Bridge](https://github.com/standardagents/t1bridge) set up (the Touch Bar
  hardware service `t1-touchbar-hw` and renderer launcher `t1-touchbar`).
- Rust (`sudo pacman -S rust`), GTK 4, libadwaita and librsvg
  (`rsvg-convert` renders the provider artwork). The Time panel uses Node.js
  and Omarchy’s installed Elsewhen and calendar models. ImageMagick decodes
  cached GitHub avatars when available.

## Install

```bash
./install.sh            # build, install to ~/.local, select as the Touch Bar renderer
```

`omatouchbar enable` points `~/.config/t1bridge/renderer` at omatouchbar
(keeping any previous selection in `renderer.previous`) and restarts
`t1-touchbar.service`. If omatouchbar ever exits, T1Bridge falls back to its
built-in renderer.

Open **Setup → Touch Bar** from the Omarchy menu (`omatouchbar-settings`) to
configure it. The settings app has a live, clickable preview that runs the same
engine as the Touch Bar.

To remove it: `./uninstall.sh` (restores the previous renderer and the
on-screen bar; keeps your settings).

## Commands

```bash
omatouchbar status                 # JSON status (connected, fps, page, …)
omatouchbar ctl menu [id]          # open the Omarchy menu, e.g. `ctl menu system`
omatouchbar ctl panel sound        # sound|mic|sources|display|brightness|battery|network|bluetooth|clock|tray|weather|agents|indicators
omatouchbar ctl home
omatouchbar ctl touchid enroll     # demo the Touch ID animation (unlock|enroll)
omatouchbar ctl mode bar           # keyboard|bar|controls|function
omatouchbar ctl reload
omatouchbar ctl snapshot frame.png # save what the Touch Bar shows right now
omatouchbar ctl debug on           # verbose logging to the journal
omatouchbar disable                # back to the built-in renderer
omatouchbar restore-bar            # show the on-screen bar if omatouchbar hid it
```

Logs: `journalctl --user -u t1-touchbar.service -f`.

## Settings

`~/.config/omatouchbar/config.toml` — written by the settings app and reloaded
live by the renderer. Every key is optional; see `crates/core/src/config.rs`.

```toml
mode = "keyboard"         # keyboard | bar | controls | function
fn_key = "function"       # what holding Fn shows: function | controls | none
hide_system_bar = true    # in Omarchy Bar mode
show_escape = true

[layout]
source = "omarchy"        # mirror shell.json, or "custom" with the lists below
right = ["keyboard", "bluetooth", "network", "audio", "brightness", "battery"]

[motion]
speed = 1.0
bounce = 0.3
fps = 60

[power]
dim_after = 60
dim_level = 0.35
```

## How it works

```
crates/t1hw      T1Bridge hardware IPC v1 client: seqpacket socket, sealed
                 memfd frame buffers, input frames, typed actions
crates/core      theme, config, fonts (ab_glyph), canvas (tiny-skia),
                 springs + keyed node animator, scene (pages, touch handling,
                 Omarchy menu/bar mirroring) and renderer
crates/daemon    `omatouchbar`: event loop, damage-tracked frame submission,
                 providers (Hyprland IPC, PipeWire, UPower, power-profiles,
                 BlueZ, NetworkManager, MPRIS, sysfs), control socket
crates/settings  `omatouchbar-settings`: GTK 4 / libadwaita app
```

The renderer runs unprivileged as the T1Bridge-selected renderer: it draws
complete XRGB8888 frames into its own buffers and the root hardware service
copies the changed region to the panel. Commands it launches for you (menu
actions, updates) run as transient `systemd-run --user` units, outside the
renderer's `NoNewPrivileges` sandbox.

Design checks without hardware:

```bash
cargo run -p omatouchbar-core --example snapshot -- bar /tmp/bar.png       # or menu, sound, network, bluetooth(-busy, -failed), weather, fkeys, keyboard(-idle, -meter, -sound), …
cargo run -p omatouchbar-core --example snapshot -- agents-live /tmp/a.png # your own agent records (agents-edge: every state; add -end, or SNAPSHOT_SCROLL=<px>)
cargo run -p omatouchbar-core --example snapshot -- menu /tmp/menu.png 12  # contact sheet of a transition
SNAPSHOT_THEME=gruvbox cargo run -p omatouchbar-core --example snapshot -- theme-wipe /tmp/w.png 26  # a theme switch
cargo run --release -p omatouchbar-core --example stress                   # frame timing under rapid input and theme switches
cargo run --release -p omatouchbar-core --example touchfx -- enroll /tmp/t.png 500,2000,4000  # Touch ID demo frames (ms)
```

## License

MIT — see [LICENSE](LICENSE). The T1Bridge protocol client is an independent
implementation of T1Bridge's published interface; no third-party code is
vendored.
