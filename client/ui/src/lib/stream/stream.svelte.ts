// Screen-share state and actions: going live, watching, and keeping both in step with voice and
// the gateway. The `#v` connection is `StreamRoom`; stream audio is the Rust `#a` side.
import type { LocalVideoTrack } from 'livekit-client'
import type { Event } from '../protocol/Event'
import type { Ready } from '../protocol/Ready'
import type { StreamEndReason } from '../protocol/StreamEndReason'
import type { StreamInfo } from '../protocol/StreamInfo'
import { displayName } from '../selectors'
import { app } from '../store.svelte'
import { errorText, streamCmd } from '../tauri'
import { voice } from '../voice.svelte'
import { startDiagnostics } from './diag.svelte'
import type { Fps, Res } from './encodings'
import type { GoLive } from './prefs'
import { StreamRoom } from './room'
import { toastFor } from './viewer'
import { removeToast, showToast } from './viewer.svelte'

type Live = { channelId: string; res: Res; fps: Fps; audio: boolean; audioMuted: boolean; label: string; encoderWarning: boolean }
type Watching = { channelId: string; streamer: string; quality: 'auto' | 'saver' }

export const stream = $state({
  room: null as StreamRoom | null,
  live: null as Live | null,
  watching: null as Watching | null,
  error: '',
  /** A failed watch, shown by the viewer's toast (`error` belongs to Go Live). */
  watchError: '',
  /** A neutral one-liner for the viewer, e.g. "<name> stopped streaming". */
  notice: '',
})

/** After a server restart Ready lacks our stream (streams live in server memory): announce it again. */
export function needsReannounce(meId: string, live: { channelId: string } | null, readyStreams: StreamInfo[]): boolean {
  if (!live) return false
  return !readyStreams.some((s) => s.channel_id === live.channelId && s.user_id === meId)
}

/**
 * Tokens for async calls that a later call (or a cancel) supersedes: after every `await`, a call
 * checks `current(token)` and backs out if something newer has started since.
 */
export function generation() {
  let n = 0
  return {
    next: () => ++n,
    current: (token: number) => token === n,
    cancel: () => void ++n,
  }
}

const PREVIEW_W = 480
const PREVIEW_H = 270
const PREVIEW_FIRST_MS = 1_000
const PREVIEW_EVERY_MS = 15_000

// Not reactive: handles the UI never reads.
let roomChannel: string | null = null
let connecting: { channelId: string; p: Promise<StreamRoom> } | null = null
let liveTrack: LocalVideoTrack | null = null
/** The shared window's title while live with stream audio (`null`: a screen), for an audio restart. */
let liveWindowTitle: string | null = null
let stopPreview: (() => void) | null = null
let stopOutDiag: (() => void) | null = null
let stopInDiag: (() => void) | null = null
let watchingEl: HTMLVideoElement | null = null
/** Go Live in flight (a second click joins it) and its channel. */
let pendingLive: { channelId: string; p: Promise<void> } | null = null
const liveGen = generation()
const watchGen = generation()

const logError = (ev: string) => (e: unknown) => streamCmd.log({ ev, error: errorText(e) })
const meId = () => app.state.me?.id ?? ''

function nameOf(channelId: string, userId: string): string {
  return displayName(app.state, app.state.channels[channelId]?.server_id ?? null, userId)
}

/** The `#v` connection for `channelId`, connecting (once) if needed. */
async function ensureRoom(channelId: string): Promise<StreamRoom> {
  if (stream.room && roomChannel === channelId && stream.room.connected) return stream.room
  if (connecting?.channelId === channelId) return connecting.p
  if (stream.room) await disconnect()
  const p = StreamRoom.connect(channelId)
  connecting = { channelId, p }
  try {
    const room = await p
    stream.room = room
    roomChannel = channelId
    return room
  } finally {
    if (connecting?.p === p) connecting = null
  }
}

async function disconnect() {
  const room = stream.room
  stream.room = null
  roomChannel = null
  await room?.disconnect().catch(logError('disconnect_error'))
}

/** Nothing to publish or watch: let the connection go. */
async function releaseIfIdle() {
  if (!stream.live && !stream.watching && !connecting && !pendingLive) await disconnect()
}

function onTrackEnded() {
  streamCmd.log({ ev: 'track_ended' })
  liveGen.cancel()
  void endLive(true)
}

/** Every 15 s while live: a 480×270 WebP of what we share, for the LIVE popover. */
function startPreview(channelId: string, track: MediaStreamTrack): () => void {
  let stopped = false
  const video = document.createElement('video')
  video.muted = true
  video.playsInline = true
  video.srcObject = new MediaStream([track])
  void video.play().catch(() => {})
  const canvas = document.createElement('canvas')
  canvas.width = PREVIEW_W
  canvas.height = PREVIEW_H
  const ctx = canvas.getContext('2d')

  const shot = () => {
    if (stopped || !ctx || !video.videoWidth || !video.videoHeight) return
    const scale = Math.min(PREVIEW_W / video.videoWidth, PREVIEW_H / video.videoHeight)
    const w = video.videoWidth * scale
    const h = video.videoHeight * scale
    ctx.fillStyle = '#000'
    ctx.fillRect(0, 0, PREVIEW_W, PREVIEW_H)
    ctx.drawImage(video, (PREVIEW_W - w) / 2, (PREVIEW_H - h) / 2, w, h)
    canvas.toBlob(
      (blob) => {
        if (!blob || stopped) return
        blobBase64(blob)
          .then((b64) => streamCmd.previewPut(channelId, b64))
          .catch(logError('preview_error'))
      },
      'image/webp',
      0.7,
    )
  }
  const first = setTimeout(shot, PREVIEW_FIRST_MS)
  const every = setInterval(shot, PREVIEW_EVERY_MS)
  return () => {
    stopped = true
    clearTimeout(first)
    clearInterval(every)
    video.srcObject = null
  }
}

/** Base64 of a blob, without the `data:` prefix. */
function blobBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(String(r.result).slice(String(r.result).indexOf(',') + 1))
    r.onerror = () => reject(r.error)
    r.readAsDataURL(blob)
  })
}

/** Ends our local share; `announce` false when the server already ended it. */
async function endLive(announce: boolean) {
  const live = stream.live
  if (!live) return
  stream.live = null
  const track = liveTrack
  liveTrack = null
  track?.mediaStreamTrack.removeEventListener('ended', onTrackEnded)
  stopOutDiag?.()
  stopOutDiag = null
  stopPreview?.()
  stopPreview = null
  if (live.audio) await streamCmd.audioStop().catch(logError('audio_stop_error'))
  // the server call never holds up the local stop
  const server = announce ? streamCmd.stop(live.channelId).catch(logError('stop_error')) : Promise.resolve()
  await stream.room?.unpublish().catch(logError('unpublish_error'))
  await server
  streamCmd.log({ ev: 'stopped' })
  await releaseIfIdle()
}

/** Stops watching locally; `announce` false when the stream is already gone on the server. */
async function endWatch(announce: boolean) {
  const w = stream.watching
  if (!w) return
  stream.watching = null
  watchingEl = null
  stopInDiag?.()
  stopInDiag = null
  stream.room?.unwatch()
  await Promise.all([
    streamCmd.audioWatch(w.channelId, w.streamer, false).catch(logError('audio_watch_error')),
    announce ? streamCmd.unwatch(w.channelId, w.streamer).catch(logError('unwatch_error')) : undefined,
  ])
}

const CAPTURE_ENDED = 'The shared window or screen closed'

async function goLive(token: number, channelId: string, track: LocalVideoTrack, label: string, settings: GoLive) {
  stream.error = ''
  const { res, fps, audio } = settings
  const ended = () => track.mediaStreamTrack.readyState === 'ended'
  let room: StreamRoom | null = null
  let announced = false
  try {
    if (ended()) throw new Error(CAPTURE_ENDED)
    room = await ensureRoom(channelId)
    if (!liveGen.current(token)) throw new Error('cancelled')
    await room.publish(track, res, fps)
    if (!liveGen.current(token)) throw new Error('cancelled')
    await streamCmd.goLive(channelId, { height: res, fps, audio })
    announced = true
    if (!liveGen.current(token)) throw new Error('cancelled')
  } catch (e) {
    const cancelled = !liveGen.current(token)
    pendingLive = null // this attempt is over: let releaseIfIdle drop the connection
    if (announced) await streamCmd.stop(channelId).catch(logError('stop_error'))
    // cancelled = Stop or a voice change meanwhile: the capture goes too; a plain failure keeps it for Retry
    await room?.unpublish(cancelled).catch(() => {})
    await releaseIfIdle()
    if (cancelled) {
      // cancelled before publishing: unpublish had nothing to stop, and the dialog has handed it over
      track.stop()
      return
    }
    stream.error = errorText(e)
    streamCmd.log({ ev: 'go_live_error', error: stream.error })
    throw e
  }
  liveTrack = track
  stream.live = { channelId, res, fps, audio, audioMuted: false, label, encoderWarning: false }
  streamCmd.log({ ev: 'live', res, fps, audio })
  // the shared window closed (or the capture otherwise ended); it may already have, before we listened
  track.mediaStreamTrack.addEventListener('ended', onTrackEnded)
  if (ended()) return onTrackEnded()
  if (audio) {
    const windowTitle = track.mediaStreamTrack.getSettings().displaySurface === 'window' ? label : null
    liveWindowTitle = windowTitle
    // Video carries on without it; Rust retries the capture itself.
    await streamCmd.audioStart(channelId, windowTitle).catch(logError('audio_start_error'))
    if (liveTrack !== track) {
      // stopped meanwhile: that stop's audio_stop ran before this start landed
      await streamCmd.audioStop().catch(logError('audio_stop_error'))
      return
    }
  }
  stopPreview = startPreview(channelId, track.mediaStreamTrack)
  const r = room
  stopOutDiag = startDiagnostics('out', () => r.localStats(), fps, (warn) => {
    if (stream.live) stream.live.encoderWarning = warn
  })
}

/** The captured track while live, for the local preview (not reactive state: a track must not be proxied). */
export const liveMediaTrack = (): MediaStreamTrack | null => liveTrack?.mediaStreamTrack ?? null

export const streamApi = {
  /**
   * Publishes the picked `track` and tells everyone. Throws (and sets `stream.error`) on failure;
   * the track is left running for a retry. A second call while one is in flight joins it; while
   * live it does nothing.
   */
  goLive(channelId: string, track: LocalVideoTrack, label: string, settings: GoLive): Promise<void> {
    if (pendingLive) return pendingLive.p
    if (stream.live) return Promise.resolve()
    const p = goLive(liveGen.next(), channelId, track, label, settings).finally(() => {
      if (pendingLive?.p === p) pendingLive = null
      // a share that ended meanwhile skipped this (the attempt was still pending)
      void releaseIfIdle()
    })
    pendingLive = { channelId, p }
    return p
  },

  stop() {
    liveGen.cancel()
    return endLive(true)
  },

  /** Watches `streamer` into `el` (switching from anyone watched before). Errors land in `stream.watchError`. */
  async watch(channelId: string, streamer: string, el: HTMLVideoElement): Promise<void> {
    const same = (w: Watching | null) => w?.channelId === channelId && w.streamer === streamer
    if (same(stream.watching) && watchingEl === el) return // a second click
    const token = watchGen.next()
    const current = () => watchGen.current(token)
    await endWatch(true)
    if (!current()) return
    stream.watchError = ''
    stream.notice = ''
    stream.watching = { channelId, streamer, quality: 'auto' }
    watchingEl = el
    try {
      const room = await ensureRoom(channelId)
      if (!current()) return
      await streamCmd.watch(channelId, streamer)
      if (!current()) return
      await streamCmd.audioWatch(channelId, streamer, true).catch(logError('audio_watch_error'))
      if (!current()) {
        // switched or closed meanwhile, and that endWatch's "off" ran before this "on" landed
        if (!same(stream.watching)) await streamCmd.audioWatch(channelId, streamer, false).catch(logError('audio_watch_error'))
        return
      }
      await room.watch(streamer, el)
      if (!current()) return
      room.setQuality(stream.watching!.quality)
      stopInDiag?.()
      stopInDiag = startDiagnostics('in', () => room.remoteStats(), 0)
    } catch (e) {
      if (!current()) return
      stream.watchError = errorText(e)
      streamCmd.log({ ev: 'watch_error', error: stream.watchError })
      await endWatch(true)
      await releaseIfIdle()
    }
  },

  async unwatch(): Promise<void> {
    watchGen.cancel()
    await endWatch(true)
    await releaseIfIdle()
  },

  setQuality(q: 'auto' | 'saver') {
    if (!stream.watching) return
    stream.watching.quality = q
    stream.room?.setQuality(q)
  },

  async setAudioMuted(muted: boolean) {
    if (!stream.live?.audio) return
    stream.live.audioMuted = muted
    await streamCmd.audioMute(muted).catch(logError('audio_mute_error'))
  },

  /** Voice channel changed (`null` = left voice): a stream never outlives your voice channel. */
  async onVoiceChanged(channelId: string | null) {
    if (pendingLive && pendingLive.channelId !== channelId) liveGen.cancel()
    if (stream.live && stream.live.channelId !== channelId) {
      liveGen.cancel()
      await endLive(true)
    }
    if (stream.watching && stream.watching.channelId !== channelId) {
      watchGen.cancel()
      await endWatch(true)
    }
    if (stream.room && roomChannel !== channelId) await disconnect()
    // voice can end without leave_voice (rejoin given up, a failed join): `#a` goes with it
    await streamCmd.audioLeave(channelId).catch(logError('audio_leave_error'))
  },

  /** LiveKit closed the `#a` connection under us: connect again once, for what we share and watch. */
  async onAudioClosed(channelId: string) {
    streamCmd.log({ ev: 'audio_closed' })
    const live = stream.live
    if (live?.channelId === channelId && live.audio) {
      try {
        await streamCmd.audioStart(channelId, liveWindowTitle)
        if (!stream.live) {
          // stopped meanwhile: that stop's audio_stop ran before this start landed
          await streamCmd.audioStop().catch(logError('audio_stop_error'))
        } else if (stream.live === live && live.audioMuted) {
          await streamCmd.audioMute(true).catch(logError('audio_mute_error'))
        }
      } catch (e) {
        logError('audio_restart_error')(e)
        if (stream.live === live) stream.notice = 'Stream audio stopped'
      }
    }
    const w = stream.watching
    if (w?.channelId === channelId) {
      await streamCmd.audioWatch(channelId, w.streamer, true).catch(logError('audio_watch_error'))
      // switched or closed meanwhile: that endWatch's "off" ran before this "on" landed
      const now = stream.watching
      if (now?.channelId !== channelId || now.streamer !== w.streamer) await streamCmd.audioWatch(channelId, w.streamer, false).catch(logError('audio_watch_error'))
    }
  },

  /** A `StreamEnded` from the gateway. */
  async onStreamEnded(channelId: string, userId: string, reason: StreamEndReason) {
    const w = stream.watching
    if (w && w.channelId === channelId && w.streamer === userId) {
      watchGen.cancel()
      await endWatch(false)
      stream.notice = `${nameOf(channelId, userId)} stopped streaming`
      await releaseIfIdle()
    }
    // the server ended ours (e.g. timed out): stop sending into nothing
    if (userId === meId() && stream.live?.channelId === channelId) {
      streamCmd.log({ ev: 'ended_by_server', reason })
      liveGen.cancel()
      await endLive(false)
    }
  },

  /** Every gateway event: stream starts toast the people in that voice channel, ends close the viewer. */
  onEvent(e: Event) {
    if (e.t === 'StreamStarted') {
      const { channel_id: channelId, user_id: streamer } = e.d.stream
      if (toastFor({ channelId, streamer }, voice.channelId, meId())) showToast({ channelId, streamer, name: nameOf(channelId, streamer) })
    } else if (e.t === 'StreamEnded') {
      removeToast(e.d.channel_id, e.d.user_id)
      void streamApi.onStreamEnded(e.d.channel_id, e.d.user_id, e.d.reason)
    }
  },

  /** After every gateway `Ready`. */
  async onReady(ready: Ready) {
    const live = stream.live
    if (!live || !needsReannounce(ready.me.id, live, ready.streams)) return
    streamCmd.log({ ev: 'reannounce' })
    try {
      await streamCmd.reannounce(live.channelId, { height: live.res, fps: live.fps, audio: live.audio })
    } catch (e) {
      logError('reannounce_error')(e)
      if (stream.live !== live) return // stopped meanwhile
      // nobody can see it: stop sending into nothing, and say so
      stream.notice = "Your stream stopped: the server couldn't take it back"
      liveGen.cancel()
      await endLive(false)
    }
  },
}
