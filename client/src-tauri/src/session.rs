//! Session token storage in the OS keychain (Windows Credential Manager / Secret Service).
//! The token never crosses into the webview.

use keyring::Entry;

const SERVICE: &str = "pulse-app";
const LAST_SERVER: &str = "__last_server__";

fn entry(account: &str) -> Option<Entry> {
    Entry::new(SERVICE, account).ok()
}

/// Best effort: if the keychain is unavailable the session simply won't survive a restart.
pub fn save(server_url: &str, token: &str) {
    for (account, value) in [(server_url, token), (LAST_SERVER, server_url)] {
        if let Some(e) = entry(account)
            && let Err(err) = e.set_password(value)
        {
            eprintln!("keychain unavailable, session won't persist: {err}");
        }
    }
}

/// The last server URL and its token, if both are stored.
pub fn load() -> Option<(String, String)> {
    let server = entry(LAST_SERVER)?.get_password().ok()?;
    let token = entry(&server)?.get_password().ok()?;
    Some((server, token))
}

pub fn clear(server_url: &str) {
    if let Some(e) = entry(server_url) {
        let _ = e.delete_credential();
    }
}
