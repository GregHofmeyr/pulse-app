# Global mute/deafen on Linux (Hyprland)

Wayland doesn't let apps grab global keys, so bind them in your compositor. The running app listens on
`$XDG_RUNTIME_DIR/pulse-app.sock`.

Use `bindn` (non-consuming), so the key keeps working in the focused app, like Pulse's binds on
Windows. Plain `bind` takes the key away from every other app.

```ini
# ~/.config/hypr/hyprland.conf (or your keybinds file)
bindn = , Next, exec, pulse-app --toggle-mute               # Page Down
bindn = , Control_R, exec, pulse-app --toggle-deafen        # Right Ctrl
bindn = CTRL SHIFT, M, exec, pulse-app --toggle-mute        # a combo works too

# mouse buttons: 274 = wheel click, 275 / 276 = side buttons
bindn = , mouse:274, exec, pulse-app --toggle-mute
```

For a dev build use the full path, e.g. `~/Projects/pulse-app/target/debug/pulse-app --toggle-mute`.
On Windows, Pulse records the binds itself (Settings → Keybinds); nothing is bound by default.
