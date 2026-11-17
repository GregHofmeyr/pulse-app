//! Windows: the allowlisted players' sessions in the system media transport controls (GSMTC, WinRT through the
//! `windows` crate; which app ids count: [`classify_gsmtc`]). Event-driven: one thread sleeps on a channel that only
//! the manager's `SessionsChanged` and the followed sessions' `MediaPropertiesChanged` / `PlaybackInfoChanged`
//! handlers (and the stop) wake; each wake reads the followed sessions once and shows the preferred one that plays
//! ([`best`]). Windows gives title, artist and album, but no Spotify id (so no art and no direct link), and no page
//! address for a browser.

use std::sync::mpsc::{Receiver, Sender, channel};

use tokio::sync::mpsc::UnboundedSender;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
};

use super::{Seen, Source, Sources, best, classify_gsmtc, now_playing_gsmtc};

enum Wake {
    /// Sessions came or went: find the players' again.
    Sessions,
    /// A followed session changed song or status.
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

pub fn spawn(sources: Sources, tx: UnboundedSender<Seen>) -> Handle {
    let (wake, woken) = channel();
    let w = wake.clone();
    let spawned = std::thread::Builder::new()
        .name("pulse-media-activity".into())
        .spawn(move || {
            if let Err(e) = run(sources, &tx, w, woken) {
                tracing::info!(error = %e, "media activity: can't read the media session");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "media activity: no watcher thread");
    }
    Handle { stop: Some(wake) }
}

/// A player's session, with the handlers hooked on it.
struct Hooked {
    session: Session,
    source: Source,
    media: i64,
    playback: i64,
}

impl Hooked {
    fn unhook(self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.media);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
    }
}

/// The manager's sessions that are players `sources` lets in, in the manager's order.
fn players(manager: &Manager, sources: Sources) -> Vec<(Session, Source)> {
    let Ok(sessions) = manager.GetSessions() else {
        return Vec::new();
    };
    (0..sessions.Size().unwrap_or(0))
        .filter_map(|i| sessions.GetAt(i).ok())
        .filter_map(|s| {
            let id = s.SourceAppUserModelId().ok()?.to_string_lossy();
            let source = classify_gsmtc(&id).filter(|src| sources.allows(src.group))?;
            Some((s, source))
        })
        .collect()
}

/// What `session` plays now. The properties are an async call: joined here, on this (MTA) thread, never a UI one.
fn read(session: &Session, source: Source) -> Seen {
    let status = session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()?.0;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let text = |r: windows::core::Result<windows::core::HSTRING>| {
        r.map(|h| h.to_string_lossy()).unwrap_or_default()
    };
    now_playing_gsmtc(
        source,
        status,
        &text(props.Title()),
        &text(props.Artist()),
        &text(props.AlbumTitle()),
    )
}

fn hook(session: Session, source: Source, wake: &Sender<Wake>) -> windows::core::Result<Hooked> {
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
        source,
        media,
        playback,
    })
}

fn run(
    sources: Sources,
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
    let mut hooked: Vec<Hooked> = Vec::new();
    let mut last: Option<Seen> = None;
    let mut wake_up = Wake::Sessions;
    loop {
        match wake_up {
            Wake::Stop => break,
            Wake::Sessions => {
                let now = players(&manager, sources);
                // keep the ones still there, unhook the gone, hook the new (in the manager's order)
                let (mut keep, gone): (Vec<Hooked>, Vec<Hooked>) = hooked
                    .drain(..)
                    .partition(|h| now.iter().any(|(s, _)| *s == h.session));
                gone.into_iter().for_each(Hooked::unhook);
                for (s, source) in now {
                    let h = match keep.iter().position(|h| h.session == s) {
                        Some(i) => Some(keep.swap_remove(i)),
                        None => hook(s, source, &wake).ok(),
                    };
                    hooked.extend(h);
                }
                keep.into_iter().for_each(Hooked::unhook);
            }
            Wake::Changed => {}
        }
        let seen = best(hooked.iter().map(|h| read(&h.session, h.source)));
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
    hooked.into_iter().for_each(Hooked::unhook);
    let _ = manager.RemoveSessionsChanged(sessions);
    Ok(())
}
