//! Where stream audio plays with no call: a ghost (watching from outside the voice channel) has
//! no voice mixer, so a small speaker output of its own takes the stream tracks. It opens when a
//! watched track first needs it and closes when the ghost stops watching or joins a call; closing
//! clears its mixer, so the tracks bound to it look for the current one again (`audio::Slot`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};

use crate::voice::devices::{Parked, build_output, find_device, prefer_internal_rate};
use crate::voice::mixer::{MixerInput, mixer};
use crate::voice::playout::Playout;

/// Received audio kept ahead of the speakers (ms), as a call keeps per peer.
const BUFFER_MS: u32 = 200;
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

struct Open {
    mix: Arc<MixerInput>,
    out: Parked<()>,
}

#[derive(Default)]
pub struct GhostOutput {
    open: Mutex<Option<Open>>,
    /// The output device chosen in voice settings (`None`: the system default).
    device: Mutex<Option<String>>,
}

impl GhostOutput {
    /// Use `device` from the next open on.
    pub fn set_device(&self, device: Option<String>) {
        *self.device.lock().unwrap() = device;
    }

    /// The mixer to play stream audio through, opening the speakers if they aren't. `None` if
    /// they can't be opened (logged; the caller tries again later).
    pub fn mixer(&self) -> Option<(Arc<MixerInput>, u64)> {
        let mut open = self.open.lock().unwrap();
        if open.is_none() {
            let device = self.device.lock().unwrap().clone();
            match open_output(device) {
                Ok(o) => *open = Some(o),
                Err(e) => {
                    tracing::warn!(error = %e, "stream audio: couldn't open the speakers for watching outside a call");
                    return None;
                }
            }
        }
        open.as_ref().map(|o| (o.mix.clone(), o.mix.generation()))
    }

    /// The mixer if the speakers are open (nothing is opened).
    pub fn open_mixer(&self) -> Option<Arc<MixerInput>> {
        self.open.lock().unwrap().as_ref().map(|o| o.mix.clone())
    }

    /// Close the speakers (nothing watched, or a call's mixer takes over).
    pub fn close(&self) {
        if let Some(o) = self.open.lock().unwrap().take() {
            o.mix.clear();
            o.out.close();
            tracing::info!("stream audio: closed the speakers used outside a call");
        }
    }
}

fn open_output(device: Option<String>) -> anyhow::Result<Open> {
    let (mix_in, mut mix) = mixer(crate::voice::devices::INTERNAL_RATE, BUFFER_MS);
    let out = Parked::spawn("pulse-stream-out", OPEN_TIMEOUT, move || {
        let dev = find_device(false, device.as_deref())?;
        let cfg = prefer_internal_rate(
            dev.default_output_config()?,
            dev.supported_output_configs()?,
        );
        let (rate, ch) = (cfg.sample_rate(), usize::from(cfg.channels()).max(1));
        let (mut playout, mut mono) = (Playout::new(rate), Vec::new());
        let stream = build_output(&dev, &cfg, move |out: &mut [f32], _| {
            mono.resize(out.len() / ch, 0.0);
            playout.fill(&mut mono, |buf| mix.mix_into(buf, 1, 1.0));
            for (frame, v) in out.chunks_mut(ch).zip(mono.iter()) {
                frame.fill(*v);
            }
        })?;
        stream.play()?;
        tracing::info!(
            rate,
            ch,
            "stream audio: opened the speakers for watching outside a call"
        );
        Ok((stream, ()))
    })?;
    Ok(Open {
        mix: Arc::new(mix_in),
        out,
    })
}
