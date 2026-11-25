//! Which window may call which of the app's own commands (#620).
//!
//! Tauri 2 checks plugin commands (`plugin:…|…`) against the capability files, but lets every window on the app's own
//! origin call every app command. The build pop-out loads the app too, so a capability file alone can't keep it
//! read-only: this gate sits in front of `generate_handler!` and refuses, per window label, every command that
//! label isn't allowed. It fails closed: an unknown label gets nothing, and a command added later reaches the
//! pop-out only when it is added to [`BUILD_COMMANDS`] here (a test enumerates every registered command).

/// The main window: every command.
pub const MAIN_LABEL: &str = "main";

/// The build pop-out's window label (build_window.rs). No capability names it (tests/capabilities.rs).
pub const BUILD_LABEL: &str = "build";

/// All the build pop-out may call: its own build's snapshot, fresh links to that build's screenshots, and its
/// always-on-top switch.
pub const BUILD_COMMANDS: [&str; 3] = [
    "build_window_snapshot",
    "build_window_image_urls",
    "set_build_window_on_top",
];

/// Whether the window labelled `label` may call the app command `command`.
pub fn allowed(label: &str, command: &str) -> bool {
    match label {
        MAIN_LABEL => true,
        BUILD_LABEL => BUILD_COMMANDS.contains(&command),
        // `embed` (a provider's page) and any label not named here: nothing
        _ => false,
    }
}

/// The app's invoke handler with the gate in front: a command this window may not call is refused (and logged, by
/// label and command name only) without reaching `next`.
pub fn guard<R: tauri::Runtime>(
    invoke: tauri::ipc::Invoke<R>,
    next: &dyn Fn(tauri::ipc::Invoke<R>) -> bool,
) -> bool {
    let label = invoke.message.webview_ref().label().to_owned();
    if !allowed(&label, invoke.message.command()) {
        tracing::warn!(window = %label, command = invoke.message.command(), "ipc gate: refused");
        invoke.resolver.reject("Not allowed in this window.");
        return true;
    }
    next(invoke)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_may_call_anything() {
        assert!(allowed(MAIN_LABEL, "send_message"));
        assert!(allowed(MAIN_LABEL, "open_build_window"));
        assert!(allowed(MAIN_LABEL, "a_command_added_later"));
    }

    #[test]
    fn the_build_window_may_call_only_its_three_commands() {
        for c in BUILD_COMMANDS {
            assert!(allowed(BUILD_LABEL, c), "{c}");
        }
        for c in [
            "send_message",
            "attachment_urls",
            "get_build",
            "open_build_window",
            "update_build_window",
            "close_build_window",
            "logout",
            "a_command_added_later",
            "",
            "Build_window_snapshot",
            "build_window_snapshot ",
        ] {
            assert!(!allowed(BUILD_LABEL, c), "{c:?}");
        }
    }

    #[test]
    fn the_embed_window_may_call_nothing() {
        for c in BUILD_COMMANDS
            .iter()
            .chain(&["send_message", "open_embed_window"])
        {
            assert!(!allowed("embed", c), "{c}");
        }
    }

    #[test]
    fn an_unknown_label_may_call_nothing() {
        for label in ["", "Main", "main ", "Build", "build2", "picker"] {
            assert!(!allowed(label, "send_message"), "{label:?}");
            assert!(!allowed(label, "build_window_snapshot"), "{label:?}");
        }
    }

    /// Fail closed for every command registered now and any added later: of everything in `generate_handler!`, the
    /// pop-out passes exactly its three (which are therefore registered), and nothing passes for `embed`.
    #[test]
    fn of_every_registered_command_the_build_window_passes_only_its_three() {
        let all: Vec<&str> = crate::registered_commands().collect();
        assert!(
            all.len() > 300,
            "the list is the real handler's: {}",
            all.len()
        );
        // names as the page invokes them (the path's last segment)
        assert!(all.contains(&"send_message") && all.contains(&"attachment_urls"));
        let mut passed: Vec<&str> = all
            .iter()
            .copied()
            .filter(|c| allowed(BUILD_LABEL, c))
            .collect();
        passed.sort();
        let mut want = BUILD_COMMANDS.to_vec();
        want.sort();
        assert_eq!(passed, want);
        for c in &all {
            assert!(allowed(MAIN_LABEL, c), "{c}");
            assert!(!allowed("embed", c), "{c}");
        }
    }
}
