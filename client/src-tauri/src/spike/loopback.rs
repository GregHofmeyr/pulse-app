//! THROWAWAY (S9, S19): stream audio via WASAPI process loopback, published as `<uid>#a` into `<ch>~s`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use pulse_protocol::ids::ChannelId;
use tauri::{AppHandle, Manager, State};

use crate::commands::Core;
use crate::voice::mixer::{MixerInput, SourceKind};

fn log(line: &str) {
    tracing::info!(target: "spike", "spike: {line}");
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The publishing side (`spike_loopback_start`) and the listening side (`spike_beep_watch`): each keeps its
/// `Room` here, or dropping it would disconnect.
#[derive(Default)]
pub struct Loopback(Mutex<Option<Running>>, Mutex<Option<Watching>>);

struct Running {
    room: Room,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct Watching {
    room: Room,
    task: tokio::task::JoinHandle<()>,
}

#[tauri::command]
pub async fn spike_loopback_start(
    core: State<'_, Core>,
    lb: State<'_, Loopback>,
    channel_id: ChannelId,
    mode: String,
    pid: Option<u32>,
    tone: Option<String>,
) -> Result<(), String> {
    stop_publishing(&lb).await?; // a second Start replaces the first instead of orphaning its thread
    let t = super::spike_ticket(core, channel_id, "a".into())
        .await
        .map_err(|e| e.to_string())?;
    let (room, _ev) = Room::connect(&t.url, &t.token, RoomOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    // stereo 48 kHz (needs the rust-sdks #1018 patch to really go out as stereo; S9 checks L≠R on the far end).
    // webrtc-sys is already patched from vendor/webrtc-sys (root Cargo.toml [patch.crates-io]): if #1018
    // touches webrtc-sys, apply it to that vendored copy rather than adding a second patch.
    // A 100 ms queue, unlike the mic's queue 0 (voice/mod.rs): WASAPI hands over bursts.
    let src = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 2, 100);
    let track =
        LocalAudioTrack::create_audio_track("stream-audio", RtcAudioSource::Native(src.clone()));
    room.local_participant()
        // ScreenshareAudio: step zero's `source_kind` maps it to SourceKind::Stream (own volume, no speaking ring)
        .publish_track(
            LocalTrack::Audio(track),
            TrackPublishOptions {
                source: TrackSource::ScreenshareAudio,
                dtx: false,
                red: false,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, rt) = (stop.clone(), tokio::runtime::Handle::current());
    let target = match (mode.as_str(), pid) {
        ("include", Some(p)) => (p, true),
        // EXCLUDE our own tree: Pulse + its WebView2 children never reach the stream
        _ => (std::process::id(), false),
    };
    let thread = std::thread::spawn(move || {
        let r = capture_loop(
            target.0,
            target.1,
            tone.as_deref(),
            &stop2,
            |chunk: Vec<i16>| {
                let frame = AudioFrame {
                    samples_per_channel: (chunk.len() / 2) as u32,
                    data: chunk.into(),
                    sample_rate: 48_000,
                    num_channels: 2,
                };
                let _ = rt.block_on(src.capture_frame(&frame));
            },
        );
        log(&match r {
            Ok(()) => r#"{"test":"S9","event":"capture_ended","ok":true}"#.to_string(),
            Err(e) => format!(
                r#"{{"test":"S9","event":"capture_ended","ok":false,"error":{}}}"#,
                serde_json::Value::String(e)
            ),
        });
    });
    *lb.0.lock().unwrap() = Some(Running {
        room,
        stop,
        thread: Some(thread),
    });
    Ok(())
}

/// Capture 10 ms stereo i16 chunks until `stop`, the target process exits, or an error.
/// `include=true`: only `pid`'s process tree; `false`: everything EXCEPT `pid`'s tree.
fn capture_loop(
    pid: u32,
    include: bool,
    tone: Option<&str>,
    stop: &AtomicBool,
    mut sink: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    if tone == Some("left") {
        // stereo check: 1 kHz on the left channel only
        let mut n = 0u64;
        while !stop.load(Ordering::Relaxed) {
            let mut c = Vec::with_capacity(960);
            for _ in 0..480 {
                let s = ((n as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin() * 8000.0)
                    as i16;
                c.extend([s, 0]);
                n += 1;
            }
            sink(c);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        return Ok(());
    }
    wasapi_capture(pid, include, stop, sink)
}

#[cfg(not(windows))]
fn wasapi_capture(
    _pid: u32,
    _include: bool,
    _stop: &AtomicBool,
    _sink: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    Err("process loopback is Windows-only".into())
}

#[cfg(windows)]
fn wasapi_capture(
    pid: u32,
    include: bool,
    stop: &AtomicBool,
    mut sink: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    wasapi::initialize_mta()
        .ok()
        .map_err(|e| format!("{e:?}"))?;
    let mut client = wasapi::AudioClient::new_application_loopback_client(pid, include)
        .map_err(|e| format!("activate: {e}"))?;
    let fmt = wasapi::WaveFormat::new(16, 16, &wasapi::SampleType::Int, 48_000, 2, None);
    let mode = wasapi::StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 200_000,
    };
    client
        .initialize_client(&fmt, &wasapi::Direction::Capture, &mode)
        .map_err(|e| format!("init: {e}"))?;
    let ev = client
        .set_get_eventhandle()
        .map_err(|e| format!("event: {e}"))?;
    let cap = client
        .get_audiocaptureclient()
        .map_err(|e| format!("capture client: {e}"))?;
    client.start_stream().map_err(|e| format!("start: {e}"))?;
    let mut q = std::collections::VecDeque::<u8>::new();
    while !stop.load(Ordering::Relaxed) {
        if include && !process_alive(pid) {
            let _ = client.stop_stream();
            return Err(format!("target process {pid} exited"));
        }
        if ev.wait_for_event(200).is_err() {
            continue;
        }
        cap.read_from_device_to_deque(&mut q)
            .map_err(|e| format!("read: {e}"))?;
        while q.len() >= 960 * 2 {
            let bytes: Vec<u8> = q.drain(..960 * 2).collect();
            sink(
                bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes(*b))
                    .collect(),
            );
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        ok && code == 259 // STILL_ACTIVE
    }
}

async fn stop_publishing(lb: &Loopback) -> Result<(), String> {
    let running = lb.0.lock().unwrap().take();
    if let Some(mut r) = running {
        r.stop.store(true, Ordering::Relaxed);
        if let Some(t) = r.thread.take() {
            let _ = tokio::task::spawn_blocking(move || t.join()).await;
        }
        r.room.close().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn spike_loopback_stop(lb: State<'_, Loopback>) -> Result<(), String> {
    stop_publishing(&lb).await
}

/// One subscribed stream-audio track: dropping it stops its task and takes it out of the mixer.
struct Sub {
    task: tokio::task::JoinHandle<()>,
    mixer: Option<(Arc<MixerInput>, u64)>,
    sid: String,
}

impl Drop for Sub {
    fn drop(&mut self) {
        self.task.abort();
        if let Some((m, g)) = &self.mixer {
            m.remove_source(*g, &self.sid);
        }
    }
}

/// Joins `~s` as `#a`, plays received stream audio through the voice mixer, and logs beep onsets (S9).
#[tauri::command]
pub async fn spike_beep_watch(
    app: AppHandle,
    core: State<'_, Core>,
    lb: State<'_, Loopback>,
    channel_id: ChannelId,
) -> Result<(), String> {
    let prev = lb.1.lock().unwrap().take();
    if let Some(w) = prev {
        w.task.abort();
        let _ = w.room.close().await;
    }
    let t = super::spike_ticket(core, channel_id, "a".into())
        .await
        .map_err(|e| e.to_string())?;
    let (room, mut ev) = Room::connect(&t.url, &t.token, RoomOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    let task = tokio::spawn(async move {
        let mut subs: HashMap<String, Sub> = HashMap::new();
        let mut said_no_mixer = false;
        while let Some(e) = ev.recv().await {
            match e {
                RoomEvent::TrackSubscribed {
                    track: RemoteTrack::Audio(t),
                    publication,
                    participant,
                } if publication.source() == TrackSource::ScreenshareAudio => {
                    let sid = publication.sid().to_string();
                    let identity = participant.identity().to_string();
                    let owner = identity.split('#').next().unwrap_or_default().to_string();
                    let mixer = app
                        .state::<crate::voice::VoiceManager>()
                        .spike_mixer()
                        .await
                        .map(|m| {
                            let g = m.generation();
                            m.add_source(g, &sid, &owner, SourceKind::Stream);
                            (m, g)
                        });
                    if mixer.is_none() && !said_no_mixer {
                        said_no_mixer = true;
                        log(r#"{"test":"S9","event":"no_mixer"}"#);
                    }
                    let m2 = mixer.clone();
                    let sid2 = sid.clone();
                    let task = tokio::spawn(async move {
                        let mut stream = NativeAudioStream::new(t.rtc_track(), 48_000, 2);
                        let mut det = super::onset::Detector::new(48_000);
                        let mut left = Vec::new();
                        let mut mono = Vec::new();
                        while let Some(f) = stream.next().await {
                            let arrived = unix_ms();
                            let ch = f.num_channels.max(1) as usize;
                            left.clear();
                            left.extend(f.data.iter().step_by(ch).map(|s| *s as f32 / 32768.0));
                            for off in det.feed(&left) {
                                log(&format!(
                                    r#"{{"test":"S9","event":"beep","t_ms":{}}}"#,
                                    arrived + off as u64 / 48
                                ));
                            }
                            if let Some((m, g)) = &m2 {
                                mono.clear();
                                mono.extend(f.data.chunks_exact(ch).map(|c| {
                                    (c.iter().map(|s| *s as i32).sum::<i32>() / ch as i32) as i16
                                }));
                                m.push_in(*g, &sid2, &mono);
                            }
                        }
                    });
                    // a re-subscribe replaces (and so cleans up) the old entry
                    subs.insert(sid.clone(), Sub { task, mixer, sid });
                }
                RoomEvent::TrackUnsubscribed { publication, .. } => {
                    subs.remove(&publication.sid().to_string());
                }
                _ => {}
            }
        }
    });
    *lb.1.lock().unwrap() = Some(Watching { room, task });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_tone_is_stereo_10ms_chunks_with_a_silent_right_channel() {
        let stop = AtomicBool::new(false);
        let mut chunks = Vec::new();
        let r = capture_loop(0, false, Some("left"), &stop, |c| {
            chunks.push(c);
            stop.store(true, Ordering::Relaxed);
        });
        assert!(r.is_ok());
        assert_eq!(chunks.len(), 1);
        let c = &chunks[0];
        assert_eq!(c.len(), 960, "480 stereo frames = 10 ms at 48 kHz");
        assert!(c.iter().step_by(2).any(|s| *s != 0), "left has the tone");
        assert!(
            c.iter().skip(1).step_by(2).all(|s| *s == 0),
            "right is silent"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn process_loopback_says_it_is_windows_only_elsewhere() {
        let stop = AtomicBool::new(false);
        let r = capture_loop(1, true, None, &stop, |_| {});
        assert!(r.unwrap_err().contains("Windows-only"));
    }

    #[cfg(windows)]
    #[test]
    fn capture_ends_cleanly_when_the_process_exits() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 3 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        let stop = AtomicBool::new(false);
        let r = capture_loop(child.id(), true, None, &stop, |_| {});
        let _ = child.wait();
        assert!(r.unwrap_err().contains("exited"));
    }
}
