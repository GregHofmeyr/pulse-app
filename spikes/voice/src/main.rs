//! THROWAWAY voice spike. Answers: PlatformAudio vs manual (cpal + APM) pipeline.
//! Never imported by real code. See FINDINGS.md.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::StreamExt;
use livekit::prelude::*;
use livekit::options::TrackPublishOptions;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::native::apm::AudioProcessingModule;
use livekit::PlatformAudio;
use livekit_api::access_token::{AccessToken, VideoGrants};
use tokio::io::AsyncBufReadExt;

const URL: &str = "ws://localhost:7880";

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List PlatformAudio devices
    Devices,
    /// libwebrtc owns mic + speakers (ADM)
    Platform {
        #[arg(long)]
        room: String,
        #[arg(long)]
        identity: String,
        #[arg(long)]
        mic: Option<String>,
        #[arg(long)]
        speaker: Option<String>,
    },
    /// We own capture/playout: cpal -> gain -> APM -> LiveKit; remote -> per-peer volume -> mix -> cpal
    Manual {
        #[arg(long)]
        room: String,
        #[arg(long)]
        identity: String,
        #[arg(long, default_value_t = 1.0)]
        gain: f32,
        #[arg(long, default_value_t = 1.0)]
        peer_volume: f32,
    },
    /// Subscribe only and write received audio to a WAV (automated flow proof)
    Sink {
        #[arg(long)]
        room: String,
        #[arg(long, default_value_t = 6)]
        seconds: u64,
        #[arg(long)]
        out: String,
    },
}

fn token(room: &str, identity: &str) -> Result<String> {
    Ok(AccessToken::with_api_key("devkey", "secret")
        .with_identity(identity)
        .with_grants(VideoGrants {
            room_join: true,
            room: room.to_string(),
            ..Default::default()
        })
        .to_jwt()?)
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse().ok()))
        })
        .unwrap_or(0)
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Devices => devices(),
        Cmd::Platform { room, identity, mic, speaker } => platform(room, identity, mic, speaker).await,
        Cmd::Manual { room, identity, gain, peer_volume } => manual(room, identity, gain, peer_volume).await,
        Cmd::Sink { room, seconds, out } => sink(room, seconds, out).await,
    }
}

fn devices() -> Result<()> {
    let audio = PlatformAudio::new()?;
    println!("recording:");
    for d in audio.recording_devices() {
        println!("  {}  id={}", d.name, d.id.as_str());
    }
    println!("playout:");
    for d in audio.playout_devices() {
        println!("  {}  id={}", d.name, d.id.as_str());
    }
    Ok(())
}

fn log_event(ev: &RoomEvent) {
    match ev {
        RoomEvent::ParticipantConnected(p) => println!("[ev] joined: {}", p.identity()),
        RoomEvent::ParticipantDisconnected(p) => println!("[ev] left: {}", p.identity()),
        RoomEvent::ActiveSpeakersChanged { speakers } => {
            let ids: Vec<_> = speakers.iter().map(|p| p.identity().to_string()).collect();
            println!("[ev] speaking: {ids:?}")
        }
        RoomEvent::ConnectionQualityChanged { quality, participant } => {
            println!("[ev] quality {}: {quality:?}", participant.identity())
        }
        RoomEvent::Reconnecting => println!("[ev] reconnecting…"),
        RoomEvent::Reconnected => println!("[ev] reconnected"),
        RoomEvent::Disconnected { reason } => println!("[ev] disconnected: {reason:?}"),
        _ => {}
    }
}

async fn platform(room_name: String, identity: String, mic: Option<String>, speaker: Option<String>) -> Result<()> {
    let audio = PlatformAudio::new()?;
    if let Some(m) = mic {
        let id = audio.recording_devices().find(|d| d.id.as_str() == m || d.name.contains(&m)).context("mic not found")?.id;
        audio.set_recording_device(&id)?;
    }
    if let Some(s) = speaker {
        let id = audio.playout_devices().find(|d| d.id.as_str() == s || d.name.contains(&s)).context("speaker not found")?.id;
        audio.set_playout_device(&id)?;
    }
    let (room, mut events) = Room::connect(URL, &token(&room_name, &identity)?, RoomOptions::default()).await?;
    let track = LocalAudioTrack::create_audio_track("mic", audio.rtc_source());
    room.local_participant()
        .publish_track(
            LocalTrack::Audio(track.clone()),
            TrackPublishOptions { source: TrackSource::Microphone, dtx: true, red: true, ..Default::default() },
        )
        .await?;
    println!("connected as {identity} (platform). commands: m=toggle mute, s <name>=switch speaker, r=rss, q=quit");

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut muted = false;
    loop {
        tokio::select! {
            ev = events.recv() => match ev { Some(ev) => log_event(&ev), None => break },
            line = stdin.next_line() => {
                let Some(line) = line? else { break };
                let line = line.trim();
                if line == "q" { break }
                if line == "r" { println!("RSS {} MB", rss_kb() / 1024) }
                if line == "m" { muted = !muted; if muted { track.mute() } else { track.unmute() }; println!("muted={muted}") }
                if let Some(name) = line.strip_prefix("s ") {
                    match audio.playout_devices().find(|d| d.name.contains(name)) {
                        Some(d) => { audio.switch_playout_device(&d.id)?; println!("speaker -> {}", d.name) }
                        None => println!("no speaker matching {name}"),
                    }
                }
            }
        }
    }
    room.close().await?;
    Ok(())
}

type Mixer = Arc<Mutex<HashMap<String, VecDeque<i16>>>>;

async fn manual(room_name: String, identity: String, gain: f32, peer_volume: f32) -> Result<()> {
    let host = cpal::default_host();
    let input = host.default_input_device().context("no input device")?;
    let output = host.default_output_device().context("no output device")?;
    let in_cfg = input.default_input_config()?;
    let out_cfg = output.default_output_config()?;
    let in_rate = in_cfg.sample_rate().0;
    let in_ch = in_cfg.channels() as usize;
    let out_rate = out_cfg.sample_rate().0;
    let out_ch = out_cfg.channels() as usize;
    println!("input {} @{in_rate}Hz x{in_ch}, output {} @{out_rate}Hz x{out_ch}", input.name()?, output.name()?);

    // One APM shared by capture (forward) and playback (reverse/echo reference).
    let apm = Arc::new(Mutex::new(AudioProcessingModule::new(true, true, true, true)));

    // --- capture: f32 interleaved -> mono i16 * gain -> 10 ms chunks -> APM -> channel
    let (cap_tx, mut cap_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
    let chunk = (in_rate / 100) as usize;
    let mut pending: Vec<i16> = Vec::with_capacity(chunk * 2);
    let apm_c = apm.clone();
    let in_stream = input.build_input_stream(
        &in_cfg.config(),
        move |data: &[f32], _| {
            for frame in data.chunks(in_ch) {
                let s = frame.iter().sum::<f32>() / in_ch as f32 * gain;
                pending.push((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                if pending.len() == chunk {
                    let mut buf = std::mem::replace(&mut pending, Vec::with_capacity(chunk));
                    let _ = apm_c.lock().unwrap().process_stream(&mut buf, in_rate as i32, 1);
                    let _ = cap_tx.send(buf);
                }
            }
        },
        |e| eprintln!("input error: {e}"),
        None,
    )?;

    // --- playback: pull one sample per peer, * peer_volume, mix, feed APM reverse
    let mixer: Mixer = Arc::new(Mutex::new(HashMap::new()));
    let mix_c = mixer.clone();
    let apm_p = apm.clone();
    let out_chunk = (out_rate / 100) as usize;
    let mut reverse: Vec<i16> = Vec::with_capacity(out_chunk * 2);
    let out_stream = output.build_output_stream(
        &out_cfg.config(),
        move |data: &mut [f32], _| {
            let mut peers = mix_c.lock().unwrap();
            for frame in data.chunks_mut(out_ch) {
                let mut acc = 0f32;
                for q in peers.values_mut() {
                    if let Some(s) = q.pop_front() {
                        acc += s as f32 / i16::MAX as f32 * peer_volume;
                    }
                }
                let v = acc.clamp(-1.0, 1.0);
                frame.fill(v);
                reverse.push((v * i16::MAX as f32) as i16);
                if reverse.len() == out_chunk {
                    let _ = apm_p.lock().unwrap().process_reverse_stream(&mut reverse, out_rate as i32, 1);
                    reverse.clear();
                }
            }
        },
        |e| eprintln!("output error: {e}"),
        None,
    )?;
    in_stream.play()?;
    out_stream.play()?;

    let (room, mut events) = Room::connect(URL, &token(&room_name, &identity)?, RoomOptions::default()).await?;
    let source = NativeAudioSource::new(AudioSourceOptions::default(), in_rate, 1, 100);
    let track = LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
    room.local_participant()
        .publish_track(
            LocalTrack::Audio(track.clone()),
            TrackPublishOptions { source: TrackSource::Microphone, dtx: true, red: true, ..Default::default() },
        )
        .await?;
    println!("connected as {identity} (manual, gain {gain}, peer volume {peer_volume}). commands: m, r, q");

    let src = source.clone();
    tokio::spawn(async move {
        while let Some(buf) = cap_rx.recv().await {
            let n = buf.len() as u32;
            let frame = AudioFrame { data: buf.into(), sample_rate: in_rate, num_channels: 1, samples_per_channel: n };
            if let Err(e) = src.capture_frame(&frame).await {
                eprintln!("capture_frame: {e}");
            }
        }
    });

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut muted = false;
    loop {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                log_event(&ev);
                if let RoomEvent::TrackSubscribed { track: RemoteTrack::Audio(t), participant, .. } = ev {
                    let id = participant.identity().to_string();
                    let mix = mixer.clone();
                    tokio::spawn(async move {
                        let mut stream = NativeAudioStream::new(t.rtc_track(), out_rate as i32, 1);
                        while let Some(f) = stream.next().await {
                            let mut m = mix.lock().unwrap();
                            let q = m.entry(id.clone()).or_default();
                            q.extend(f.data.iter());
                            // keep at most 200 ms buffered per peer
                            let max = out_rate as usize / 5;
                            if q.len() > max { let drop = q.len() - max; q.drain(..drop); }
                        }
                        mix.lock().unwrap().remove(&id);
                    });
                }
            }
            line = stdin.next_line() => {
                let Some(line) = line? else { break };
                match line.trim() {
                    "q" => break,
                    "r" => println!("RSS {} MB", rss_kb() / 1024),
                    "m" => { muted = !muted; if muted { track.mute() } else { track.unmute() }; println!("muted={muted}") }
                    _ => {}
                }
            }
        }
    }
    room.close().await?;
    Ok(())
}

async fn sink(room_name: String, seconds: u64, out: String) -> Result<()> {
    let (room, mut events) = Room::connect(URL, &token(&room_name, "sink")?, RoomOptions::default()).await?;
    let spec = hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let samples: Arc<Mutex<Vec<i16>>> = Arc::default();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(ev)) = tokio::time::timeout(left, events.recv()).await else { break };
        log_event(&ev);
        if let RoomEvent::TrackSubscribed { track: RemoteTrack::Audio(t), .. } = ev {
            let s = samples.clone();
            tokio::spawn(async move {
                let mut stream = NativeAudioStream::new(t.rtc_track(), 48000, 1);
                while let Some(f) = stream.next().await {
                    s.lock().unwrap().extend(f.data.iter());
                }
            });
        }
    }
    room.close().await?;
    let data = samples.lock().unwrap().clone();
    let mut w = hound::WavWriter::create(&out, spec)?;
    for s in &data {
        w.write_sample(*s)?;
    }
    w.finalize()?;
    let rms = (data.iter().map(|s| (*s as f64 / 32768.0).powi(2)).sum::<f64>() / data.len().max(1) as f64).sqrt();
    println!("wrote {} samples ({:.1}s) to {out}, RMS {rms:.4}, RSS {} MB", data.len(), data.len() as f64 / 48000.0, rss_kb() / 1024);
    Ok(())
}
