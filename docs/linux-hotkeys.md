# Global keybinds on Linux (Hyprland)

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

# bring Pulse up and open the quick switcher (#319); plain bind: the key is Pulse's alone
bind = CTRL SHIFT, K, exec, pulse-app --quick-switch
```

The quick switcher's command asks the running Pulse to come to the front and open the switcher. Wayland lets a
compositor refuse that (no app may take the focus by itself): on Hyprland the default `misc:focus_on_activate = false`
marks Pulse urgent instead of focusing it, so the switcher opens behind your current window. Set
`misc { focus_on_activate = true }`, or have the same bind run `hyprctl dispatch focuswindow class:<Pulse's class>`
first (`hyprctl clients` shows the class).

For a dev build use the full path, e.g. `~/Projects/pulse-app/target/debug/pulse-app --toggle-mute`.
On Windows, Pulse records the binds itself (Settings → Keybinds), the quick switcher's included; nothing is bound by
default.
