//! Windows: the Spotify app's session in the system media transport controls (GSMTC, WinRT through the `windows`
//! crate). Event-driven: one thread sleeps on a channel that only the manager's `SessionsChanged` and the Spotify
//! session's `MediaPropertiesChanged` / `PlaybackInfoChanged` handlers (and the stop) wake; each wake reads the
//! session once. Windows gives title, artist and album, but no Spotify id (so no art and no direct link).

use std::sync::mpsc::{Receiver, Sender, channel};

use tokio::sync::mpsc::UnboundedSender;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
};

use super::{Seen, is_spotify_app, now_playing_gsmtc};

enum Wake {
    /// Sessions came or went: find Spotify's again.
    Sessions,
    /// Spotify's session changed song or status.
    Changed,
    Stop,
}

/// The watcher thread: stopping it (dropping this) wakes it to unhook its handlers and end.
pub struct Handle {
    stop: Option<Sender<Wake>>,
}

impl Handle {
    /// A handle with no thread (tests).
    #[cfg(test)]
    pub fn none() -> Self {
        Handle { stop: None }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(Wake::Stop);
        }
    }
}

pub fn spawn(tx: UnboundedSender<Seen>) -> Handle {
    let (wake, woken) = channel();
    let w = wake.clone();
    let spawned = std::thread::Builder::new()
        .name("pulse-media-activity".into())
        .spawn(move || {
            if let Err(e) = run(&tx, w, woken) {
                tracing::info!(error = %e, "media activity: can't read the media session");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "media activity: no watcher thread");
    }
    Handle { stop: Some(wake) }
}

/// Spotify's session, with the handlers hooked on it.
struct Hooked {
    session: Session,
    media: i64,
    playback: i64,
}

impl Hooked {
    fn unhook(self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.media);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
    }
}

/// The Spotify app's session among the manager's, if it has one.
fn spotify(manager: &Manager) -> Option<Session> {
    let sessions = manager.GetSessions().ok()?;
    (0..sessions.Size().ok()?)
        .filter_map(|i| sessions.GetAt(i).ok())
        .find(|s| {
            s.SourceAppUserModelId()
                .is_ok_and(|id| is_spotify_app(&id.to_string_lossy()))
        })
}

/// What `session` plays now. The properties are an async call: joined here, on this (MTA) thread, never a UI one.
fn read(session: &Session) -> Seen {
    let status = session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()?.0;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let text = |r: windows::core::Result<windows::core::HSTRING>| {
        r.map(|h| h.to_string_lossy()).unwrap_or_default()
    };
    now_playing_gsmtc(
        status,
        &text(props.Title()),
        &text(props.Artist()),
        &text(props.AlbumTitle()),
    )
}

fn hook(session: Session, wake: &Sender<Wake>) -> windows::core::Result<Hooked> {
    let w = wake.clone();
    let media = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
        let _ = w.send(Wake::Changed);
        Ok(())
    }))?;
    let w = wake.clone();
    let playback = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
        let _ = w.send(Wake::Changed);
        Ok(())
    }))?;
    Ok(Hooked {
        session,
        media,
        playback,
    })
}

fn run(
    tx: &UnboundedSender<Seen>,
    wake: Sender<Wake>,
    woken: Receiver<Wake>,
) -> windows::core::Result<()> {
    let manager = Manager::RequestAsync()?.join()?;
    let w = wake.clone();
    let sessions = manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        let _ = w.send(Wake::Sessions);
        Ok(())
    }))?;
    let mut hooked: Option<Hooked> = None;
    let mut last: Option<Seen> = None;
    let mut wake_up = Wake::Sessions;
    loop {
        match wake_up {
            Wake::Stop => break,
            Wake::Sessions => {
                let now = spotify(&manager);
                if hooked.as_ref().map(|h| &h.session) != now.as_ref() {
                    if let Some(h) = hooked.take() {
                        h.unhook();
                    }
                    hooked = now.and_then(|s| hook(s, &wake).ok());
                }
            }
            Wake::Changed => {}
        }
        let seen = hooked.as_ref().and_then(|h| read(&h.session));
        if last.as_ref() != Some(&seen) {
            if tx.send(seen.clone()).is_err() {
                break;
            }
            last = Some(seen);
        }
        wake_up = match woken.recv() {
            Ok(w) => w,
            Err(_) => break,
        };
    }
    if let Some(h) = hooked.take() {
        h.unhook();
    }
    let _ = manager.RemoveSessionsChanged(sessions);
    Ok(())
}
