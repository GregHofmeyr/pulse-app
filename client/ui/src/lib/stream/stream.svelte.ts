// Screen-share state and actions: going live, watching, and keeping both in step with voice and
// the gateway. The `#v` connection is `StreamRoom`; stream audio is the Rust `#a` side.
import type { LocalVideoTrack } from 'livekit-client'
import type { Ready } from '../protocol/Ready'
import type { StreamEndReason } from '../protocol/StreamEndReason'
import type { StreamInfo } from '../protocol/StreamInfo'
import { displayName } from '../selectors'
import { app } from '../store.svelte'
import { errorText, streamCmd } from '../tauri'
import { startDiagnostics } from './diag.svelte'
import type { Fps, Res } from './encodings'
import type { GoLive } from './prefs'
import { StreamRoom } from './room'

type Live = { channelId: string; res: Res; fps: Fps; audio: boolean; audioMuted: boolean; label: string; encoderWarning: boolean }
type Watching = { channelId: string; streamer: string; quality: 'auto' | 'saver' }

export const stream = $state({
  room: null as StreamRoom | null,
  live: null as Live | null,
  watching: null as Watching | null,
  error: '',
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
    if (cancelled) return
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
    })
    pendingLive = { channelId, p }
    return p
  },

  stop() {
    liveGen.cancel()
    return endLive(true)
  },

  /** Watches `streamer` into `el` (switching from anyone watched before). Errors land in `stream.error`. */
  async watch(channelId: string, streamer: string, el: HTMLVideoElement): Promise<void> {
    const same = (w: Watching | null) => w?.channelId === channelId && w.streamer === streamer
    if (same(stream.watching) && watchingEl === el) return // a second click
    const token = watchGen.next()
    const current = () => watchGen.current(token)
    await endWatch(true)
    if (!current()) return
    stream.error = ''
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
      stream.error = errorText(e)
      streamCmd.log({ ev: 'watch_error', error: stream.error })
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

  /** After every gateway `Ready`. */
  onReady(ready: Ready) {
    const live = stream.live
    if (!live || !needsReannounce(ready.me.id, live, ready.streams)) return
    streamCmd.log({ ev: 'reannounce' })
    streamCmd.goLive(live.channelId, { height: live.res, fps: live.fps, audio: live.audio }).catch(logError('reannounce_error'))
  },
}
