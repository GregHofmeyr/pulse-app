// Screen-share state and actions: going live, watching, and keeping both in step with voice and
// the gateway. The `#v` connection is `StreamRoom`; stream audio is the Rust `#a` side.
import type { LocalVideoTrack } from 'livekit-client'
import type { Event } from '../protocol/Event'
import type { Ready } from '../protocol/Ready'
import type { StreamEndReason } from '../protocol/StreamEndReason'
import type { StreamInfo } from '../protocol/StreamInfo'
import { displayName } from '../selectors'
import type { AppState } from '../state'
import { app } from '../store.svelte'
import { errorText, streamCmd } from '../tauri'
import { voice } from '../voice.svelte'
import { clampVolume, playSound } from '../voiceui'
import { diag, startDiagnostics } from './diag.svelte'
import type { Fps, Res } from './encodings'
import { loadAudioMuted, loadStreamVolumes, saveAudioMuted, saveStreamVolumes, type GoLive } from './prefs'
import { StreamRoom } from './room'
import { motion } from '../animate.svelte'
import { admit, layerFor, liveEndSound, toastFor, type Quality, type View } from './viewer'
import { removeToast, showToast } from './viewer.svelte'

type Live = { channelId: string; res: Res; fps: Fps; audio: boolean; audioMuted: boolean; label: string; encoderWarning: boolean }
export type Watching = { channelId: string; streamer: string; quality: Quality; view: View }

/** How many streams you can watch at once; at the cap, a new watch stops the oldest first. */
export const MAX_WATCHED = 1

export const stream = $state({
  room: null as StreamRoom | null,
  live: null as Live | null,
  /** Oldest first. An entry exists from the Watch click on, before its video is up. */
  watching: [] as Watching[],
  error: '',
  /** A failed watch, shown by the viewer's toast (`error` belongs to Go Live). */
  watchError: '',
  /** A neutral one-liner for the viewer, e.g. "<name> stopped streaming". */
  notice: '',
  /** The channel our own Stop just ended a share in, until the server's list drops us (not "live elsewhere"). */
  stopping: null as string | null,
  /** Stream playback volume per streamer (percent), remembered on this device: tiles and full view read it. */
  volumes: loadStreamVolumes() as Record<string, number>,
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
/**
 * LiveKit closing `#a` more often than this is a fight, not bad luck (the same account signed in
 * twice: each `#a` join kicks the other's): stop reconnecting, or the two clients loop forever.
 */
const AUDIO_RECONNECTS_MAX = 3
const AUDIO_RECONNECT_WINDOW_MS = 60_000
/** A stream a server restart dropped from Ready and that comes back within this is re-announced, not new. */
const BACK_WITHIN_MS = 60_000

// Not reactive: handles the UI never reads.
let roomChannel: string | null = null
let connecting: { channelId: string; p: Promise<StreamRoom> } | null = null
let liveTrack: LocalVideoTrack | null = null
/** The shared window's title while live with stream audio (`null`: a screen), for an audio restart. */
let liveWindowTitle: string | null = null
/** When LiveKit last closed `#a` under us, within `AUDIO_RECONNECT_WINDOW_MS`. */
let audioClosedAt: number[] = []
/** Streams we knew that the last Ready left out (`channel/user` → when): a server restart. */
const vanished = new Map<string, number>()
const streamKey = (channelId: string, userId: string) => `${channelId}/${userId}`
let stopPreview: (() => void) | null = null
let stopOutDiag: (() => void) | null = null
/** Inbound diagnostics sample the watch shown in full view: its key, and the stop function. */
let inDiag: { key: string; stop: () => void } | null = null
/** One `<video>` per watch, owned here so it moves between hosts without restarting. */
const videos = new Map<string, HTMLVideoElement>()
/** Watches whose video is up (`room.watch` resolved) → their time to the first frame, in ms. */
const up = new Map<string, number>()
/** Per watch: a later watch of the same stream, or its end, supersedes the calls in flight. */
const watchGens = new Map<string, ReturnType<typeof generation>>()
/** Go Live in flight (a second click joins it) and its channel. */
let pendingLive: { channelId: string; p: Promise<void> } | null = null
const liveGen = generation()

const logError = (ev: string) => (e: unknown) => streamCmd.log({ ev, error: errorText(e) })
const meId = () => app.state.me?.id ?? ''

// Forget `stopping` once the server's list no longer has our stream there (it follows app state, mounted or not).
$effect.root(() => {
  $effect(() => {
    const ch = stream.stopping
    if (ch && !app.state.streams[ch]?.[meId()]) stream.stopping = null
  })
})

function nameOf(channelId: string, userId: string): string {
  return displayName(app.state, app.state.channels[channelId]?.server_id ?? null, userId)
}

/** Which `#v` connection is current (by identity: the store's copy may be a reactive proxy). */
let roomGen = 0

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
    const mine = ++roomGen
    room.onLost(() => void roomLost(mine))
    return room
  } finally {
    if (connecting?.p === p) connecting = null
  }
}

async function disconnect() {
  roomGen++ // what LiveKit reports about the old connection from here on is our own doing
  const room = stream.room
  stream.room = null
  roomChannel = null
  await room?.disconnect().catch(logError('disconnect_error'))
}

/**
 * LiveKit gave up on the `#v` connection: what we share and watch on it is dead, so end it (and
 * the live freeze with it) rather than sit "live" with nothing going out. `mine` is the
 * connection's generation: our own `disconnect()` (or a newer connection) moves it on, so it never counts.
 */
async function roomLost(mine: number) {
  if (mine !== roomGen || !stream.room) return
  stream.room = null
  roomChannel = null
  streamCmd.log({ ev: 'room_lost', live: !!stream.live, watching: stream.watching.length })
  if (!stream.live && !stream.watching.length) return
  stream.notice = stream.live ? 'Streaming stopped: the connection was lost' : 'Stopped watching: the connection was lost'
  // only what was on the lost connection: a watch started on a new one meanwhile stays
  const lost = new Set(stream.watching.map((w) => streamKey(w.channelId, w.streamer)))
  if (stream.live) {
    liveGen.cancel()
    await endLive(true)
  }
  await endWatches((w) => lost.has(streamKey(w.channelId, w.streamer)), true)
}

/** Nothing to publish or watch: let the connection go. */
async function releaseIfIdle() {
  if (!stream.live && !stream.watching.length && !connecting && !pendingLive) await disconnect()
}

function onTrackEnded() {
  streamCmd.log({ ev: 'track_ended' })
  liveGen.cancel()
  void endLive(true)
}

/**
 * While we're live the UI holds still (emoji show their first frame, app.css `.streaming` pauses
 * CSS animation) so it doesn't cost game frames; it follows `stream.live`, so every way a share
 * ends (Stop, the capture ending, the server or a voice change ending it) lifts it. Call once
 * (the shell does, so it outlives the voice panel); the returned stop lifts it too.
 */
export function followLiveFreeze(): () => void {
  return $effect.root(() => {
    $effect(() => {
      if (!stream.live) return
      document.body.classList.add('streaming')
      motion.streaming = true
      return () => {
        document.body.classList.remove('streaming')
        motion.streaming = false
      }
    })
  })
}

/** Every 15 s while live: a 480×270 WebP (JPEG where the webview has no WebP) of what we share, for the LIVE popover. */
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
    previewBlob(canvas)
      .then((blob) => (blob && !stopped ? blobBase64(blob).then((b64) => streamCmd.previewPut(channelId, b64)) : undefined))
      .catch(logError('preview_error'))
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

/** The canvas as WebP; where the webview can't encode WebP (it hands back PNG, which for a busy
 *  frame can pass the server's 100 KB cap), as JPEG instead. */
export async function previewBlob(canvas: HTMLCanvasElement): Promise<Blob | null> {
  const encode = (type: string) => new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, 0.7))
  const webp = await encode('image/webp')
  if (!webp || webp.type === 'image/webp') return webp
  return encode('image/jpeg')
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
  // from the moment the share is gone here: audio_stop below can take a while
  if (announce) stream.stopping = live.channelId
  const track = liveTrack
  liveTrack = null
  track?.mediaStreamTrack.removeEventListener('ended', onTrackEnded)
  stopOutDiag?.()
  stopOutDiag = null
  stopPreview?.()
  stopPreview = null
  if (live.audio) await streamCmd.audioStop().catch(logError('audio_stop_error'))
  // the server call never holds up the local stop
  const server = announce
    ? streamCmd.stop(live.channelId).catch((e) => {
        // the server still lists us: say so again rather than hide it
        if (stream.stopping === live.channelId) stream.stopping = null
        logError('stop_error')(e)
      })
    : Promise.resolve()
  await stream.room?.unpublish().catch(logError('unpublish_error'))
  await server
  streamCmd.log({ ev: 'stopped' })
  await releaseIfIdle()
}

/** The watch of `streamer` in `channelId`, if any. */
export function findWatch(channelId: string, streamer: string): Watching | undefined {
  return stream.watching.find((w) => w.channelId === channelId && w.streamer === streamer)
}

/** The watch's `<video>` (not reactive: read it where a watch is known to exist), or null. */
export function videoOf(channelId: string, streamer: string): HTMLVideoElement | null {
  return videos.get(streamKey(channelId, streamer)) ?? null
}

/** Inbound diagnostics (and the first-frame time) follow the watch in full view, once its video is up. */
function syncInDiag() {
  const full = stream.watching.find((w) => w.view === 'full' && up.has(streamKey(w.channelId, w.streamer)))
  const key = full && streamKey(full.channelId, full.streamer)
  diag.firstFrameMs = key ? (up.get(key) ?? null) : null
  if (inDiag?.key === key) return
  inDiag?.stop()
  inDiag = null
  if (!full || !key) return
  const { streamer } = full
  inDiag = { key, stop: startDiagnostics('in', () => stream.room?.remoteStats(streamer) ?? Promise.resolve(undefined), 0) }
}

/** Asks the room for the layer `w`'s view and quality call for. */
function applyLayer(w: Watching) {
  stream.room?.setLayer(w.streamer, layerFor(w.view, w.quality))
}

/**
 * Stops watching locally; `announce` false when the stream is already gone on the server. Supersedes
 * the watch's calls in flight, and its `<video>` goes (out of the DOM, `srcObject` cleared).
 */
async function endWatch(channelId: string, streamer: string, announce: boolean) {
  const k = streamKey(channelId, streamer)
  if (!findWatch(channelId, streamer)) return
  stream.watching = stream.watching.filter((w) => streamKey(w.channelId, w.streamer) !== k)
  watchGens.get(k)?.cancel()
  watchGens.delete(k)
  up.delete(k)
  stream.room?.unwatch(streamer)
  const el = videos.get(k)
  videos.delete(k)
  if (el) {
    el.srcObject = null
    el.remove()
  }
  syncInDiag()
  await Promise.all([
    streamCmd.audioWatch(channelId, streamer, false).catch(logError('audio_watch_error')),
    announce ? streamCmd.unwatch(channelId, streamer).catch(logError('unwatch_error')) : undefined,
  ])
}

/** A snapshot of the watches matching `pred`, ended one by one. */
async function endWatches(pred: (w: Watching) => boolean, announce: boolean) {
  for (const { channelId, streamer } of stream.watching.filter(pred)) await endWatch(channelId, streamer, announce)
}

async function watch(channelId: string, streamer: string): Promise<void> {
  if (findWatch(channelId, streamer)) return // a second click
  const k = streamKey(channelId, streamer)
  const gen = generation()
  watchGens.set(k, gen)
  const token = gen.next()
  const current = () => gen.current(token)
  stream.watchError = ''
  stream.notice = ''
  const el = document.createElement('video')
  el.autoplay = true
  el.playsInline = true
  el.muted = true // stream audio plays from the Rust `#a` side
  videos.set(k, el)
  const { list, dropped } = admit<Watching>(stream.watching, { channelId, streamer, quality: 'auto', view: 'preview' }, MAX_WATCHED)
  // the cap: the oldest watches stop first (their calls in flight back out)
  const ending = dropped.map((d) => endWatch(d.channelId, d.streamer, true))
  stream.watching = list
  await Promise.all(ending)
  if (!current()) return
  try {
    const room = await ensureRoom(channelId)
    if (!current()) return
    await streamCmd.watch(channelId, streamer)
    if (!current()) return
    await streamCmd.audioWatch(channelId, streamer, true).catch(logError('audio_watch_error'))
    if (!current()) {
      // stopped meanwhile, and that endWatch's "off" ran before this "on" landed
      if (!findWatch(channelId, streamer)) await streamCmd.audioWatch(channelId, streamer, false).catch(logError('audio_watch_error'))
      return
    }
    const firstFrameMs = await room.watch(streamer, el)
    if (!current()) return
    up.set(k, firstFrameMs)
    const w = findWatch(channelId, streamer)
    if (w) applyLayer(w)
    void streamCmd.setVolume(streamer, stream.volumes[streamer] ?? 100).catch(logError('stream_volume_error'))
    syncInDiag()
  } catch (e) {
    if (!current()) return
    stream.watchError = errorText(e)
    streamCmd.log({ ev: 'watch_error', error: stream.watchError })
    await endWatch(channelId, streamer, true)
    await releaseIfIdle()
  }
}

const CAPTURE_ENDED = 'The shared window or screen closed'

/** The server lists a stream of ours, in the call this client is in, that we aren't sending and
 *  aren't starting: e.g. its end reached us after a re-announce had overtaken it, so we stopped,
 *  and the server (rightly) sends the stream again. Nobody would ever see it end: stop it. (Not in
 *  that call: another of our devices may be the one streaming.) */
function stopGhost(info: StreamInfo) {
  const { channel_id: channelId, user_id: user } = info
  if (user !== meId() || voice.channelId !== channelId) return
  if (stream.live?.channelId === channelId || pendingLive) return
  streamCmd.log({ ev: 'ghost_stop' })
  void streamCmd.stop(channelId).catch(logError('ghost_stop_error'))
}

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
  audioClosedAt = [] // a new share gets its own reconnects
  // muted stays muted from the last Go Live
  const audioMuted = audio && loadAudioMuted()
  stream.live = { channelId, res, fps, audio, audioMuted, label, encoderWarning: false }
  streamCmd.log({ ev: 'live', res, fps, audio })
  // the shared window closed (or the capture otherwise ended); it may already have, before we listened
  track.mediaStreamTrack.addEventListener('ended', onTrackEnded)
  if (ended()) return onTrackEnded()
  if (audio) {
    const windowTitle = track.mediaStreamTrack.getSettings().displaySurface === 'window' ? label : null
    liveWindowTitle = windowTitle
    // Video carries on without it; Rust retries the capture itself.
    await streamCmd.audioStart(channelId, windowTitle, audioMuted).catch(logError('audio_start_error'))
    if (liveTrack !== track) {
      // stopped meanwhile: that stop's audio_stop ran before this start landed
      await streamCmd.audioStop().catch(logError('audio_stop_error'))
      return
    }
    // a mute clicked while starting reached Rust before there was a track to mute
    const muted = stream.live?.audioMuted ?? audioMuted
    if (muted !== audioMuted) await streamCmd.audioMute(muted).catch(logError('audio_mute_error'))
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

  /**
   * Watches `streamer` into a `<video>` this store owns (`videoOf`); a second call for the same
   * stream does nothing. At `MAX_WATCHED` the oldest watch stops first. Errors land in `stream.watchError`.
   */
  watch,

  async unwatch(channelId: string, streamer: string): Promise<void> {
    await endWatch(channelId, streamer, true)
    await releaseIfIdle()
  },

  async unwatchAll(): Promise<void> {
    await endWatches(() => true, true)
    await releaseIfIdle()
  },

  setQuality(channelId: string, streamer: string, q: Quality) {
    const w = findWatch(channelId, streamer)
    if (!w) return
    w.quality = q
    applyLayer(w)
  },

  /** Grid tile or mini-preview (`preview`), or the main area (`full`): the layer and diagnostics follow. */
  setView(channelId: string, streamer: string, view: View) {
    const w = findWatch(channelId, streamer)
    if (!w) return
    w.view = view
    applyLayer(w)
    syncInDiag()
  },

  /** Remembered per streamer, and applied now (and each time a watch of them is up). */
  setStreamVolume(streamer: string, pct: number) {
    const v = clampVolume(pct)
    stream.volumes = { ...stream.volumes, [streamer]: v }
    saveStreamVolumes(stream.volumes)
    void streamCmd.setVolume(streamer, v).catch(logError('stream_volume_error'))
  },

  async setAudioMuted(muted: boolean) {
    if (!stream.live?.audio) return
    stream.live.audioMuted = muted
    saveAudioMuted(muted)
    await streamCmd.audioMute(muted).catch(logError('audio_mute_error'))
  },

  /** Voice channel changed (`null` = left voice): a stream never outlives your voice channel. */
  async onVoiceChanged(channelId: string | null) {
    if (pendingLive && pendingLive.channelId !== channelId) liveGen.cancel()
    if (stream.live && stream.live.channelId !== channelId) {
      liveGen.cancel()
      await endLive(true)
    }
    await endWatches((w) => w.channelId !== channelId, true)
    if (stream.room && roomChannel !== channelId) await disconnect()
    // voice can end without leave_voice (rejoin given up, a failed join): `#a` goes with it
    await streamCmd.audioLeave(channelId).catch(logError('audio_leave_error'))
  },

  /** LiveKit closed the `#a` connection under us: connect again once, for what we share and watch (up to `AUDIO_RECONNECTS_MAX` a minute). */
  async onAudioClosed(channelId: string) {
    streamCmd.log({ ev: 'audio_closed' })
    const now = Date.now()
    audioClosedAt = [...audioClosedAt.filter((t) => now - t < AUDIO_RECONNECT_WINDOW_MS), now]
    if (audioClosedAt.length > AUDIO_RECONNECTS_MAX) {
      streamCmd.log({ ev: 'audio_reconnect_given_up' })
      const ours = stream.live?.channelId === channelId && stream.live.audio
      if (ours || stream.watching.some((w) => w.channelId === channelId)) stream.notice = 'Stream audio stopped'
      return
    }
    const live = stream.live
    if (live?.channelId === channelId && live.audio) {
      const muted = live.audioMuted
      try {
        await streamCmd.audioStart(channelId, liveWindowTitle, muted)
        const now = stream.live
        if (!now?.audio) {
          // stopped (or live again without audio) meanwhile: that stop's audio_stop ran before this start landed
          await streamCmd.audioStop().catch(logError('audio_stop_error'))
        } else if (now !== live) {
          // stopped and live again with audio: this start may have replaced the new share's
          // (another window, another mute), so start that one again
          await streamCmd.audioStart(now.channelId, liveWindowTitle, now.audioMuted).catch(logError('audio_start_error'))
        } else if (live.audioMuted !== muted) {
          await streamCmd.audioMute(live.audioMuted).catch(logError('audio_mute_error'))
        }
      } catch (e) {
        logError('audio_restart_error')(e)
        if (stream.live === live) stream.notice = 'Stream audio stopped'
      }
    }
    for (const { streamer } of stream.watching.filter((w) => w.channelId === channelId)) {
      await streamCmd.audioWatch(channelId, streamer, true).catch(logError('audio_watch_error'))
      // stopped meanwhile: that endWatch's "off" ran before this "on" landed
      if (!findWatch(channelId, streamer)) await streamCmd.audioWatch(channelId, streamer, false).catch(logError('audio_watch_error'))
    }
  },

  /** A `StreamEnded` from the gateway. */
  async onStreamEnded(channelId: string, userId: string, reason: StreamEndReason) {
    if (findWatch(channelId, userId)) {
      await endWatch(channelId, userId, false)
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
    if (e.t === 'StreamStarted' || e.t === 'StreamUpdated') stopGhost(e.d.stream)
    if (e.t === 'StreamStarted') {
      const { channel_id: channelId, user_id: streamer } = e.d.stream
      const k = streamKey(channelId, streamer)
      const back = Date.now() - (vanished.get(k) ?? -Infinity) < BACK_WITHIN_MS
      vanished.delete(k)
      // a stream re-announced after a server restart isn't news
      if (!back && toastFor({ channelId, streamer }, voice.channelId, meId())) {
        showToast({ channelId, streamer, name: nameOf(channelId, streamer) })
        playSound('live')
      }
      // only a restart re-starts a stream we watch (a real stop ends the watch first): the
      // restarted server lost its viewers, so list us again
      if (findWatch(channelId, streamer)) void streamCmd.watch(channelId, streamer).catch(logError('rewatch_error'))
    } else if (e.t === 'StreamEnded') {
      removeToast(e.d.channel_id, e.d.user_id)
      // before onStreamEnded ends the watch
      const watching = !!findWatch(e.d.channel_id, e.d.user_id)
      const ending = { channelId: e.d.channel_id, userId: e.d.user_id, reason: e.d.reason }
      if (liveEndSound(ending, { voiceChannelId: voice.channelId, meId: meId(), watching })) playSound('live_end')
      void streamApi.onStreamEnded(e.d.channel_id, e.d.user_id, e.d.reason)
    }
  },

  /** After every gateway `Ready`; `before` is the streams we knew until it came. */
  async onReady(ready: Ready, before: AppState['streams'] = {}) {
    const now = Date.now()
    for (const [k, at] of vanished) if (now - at >= BACK_WITHIN_MS) vanished.delete(k)
    for (const [channelId, byUser] of Object.entries(before)) {
      for (const userId of Object.keys(byUser)) {
        if (!ready.streams.some((s) => s.channel_id === channelId && s.user_id === userId)) vanished.set(streamKey(channelId, userId), now)
      }
    }
    // the streamer got back to the restarted server before us: its Ready lists the stream we
    // watch without us, and no StreamStarted comes
    for (const w of [...stream.watching]) {
      if (!findWatch(w.channelId, w.streamer)) continue // ended while an earlier one was re-listed
      const watched = ready.streams.find((s) => s.channel_id === w.channelId && s.user_id === w.streamer)
      if (watched && !watched.viewers.includes(ready.me.id)) await streamCmd.watch(w.channelId, w.streamer).catch(logError('rewatch_error'))
    }
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
