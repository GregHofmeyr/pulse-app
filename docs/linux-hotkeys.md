# Global mute/deafen on Linux (Hyprland)

Wayland doesn't let apps grab global keys, so bind them in your compositor. The running app listens on
`$XDG_RUNTIME_DIR/pulse-app.sock`.

```ini
# ~/.config/hypr/hyprland.conf (or your keybinds file)
bind = CTRL SHIFT, M, exec, pulse-app --toggle-mute
bind = CTRL SHIFT, D, exec, pulse-app --toggle-deafen
```

For a dev build use the full path, e.g. `~/Projects/pulse-app/target/debug/pulse-app --toggle-mute`.
Windows uses the same shortcuts (Ctrl+Shift+M / Ctrl+Shift+D) registered by the app itself.
