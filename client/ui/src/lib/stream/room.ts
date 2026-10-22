// The `<uid>#v` connection to the stream room `<channel_id>~s`: publishes our screen share and
// subscribes to the one we watch. Stream audio is the Rust `#a` connection, not this one.
import {
  ConnectionState,
  Room,
  RoomEvent,
  Track,
  VideoPreset,
  VideoQuality,
  type LocalTrack,
  type LocalVideoTrack,
  type RemoteParticipant,
  type RemoteTrack,
  type RemoteTrackPublication,
} from 'livekit-client'
import { streamCmd } from '../tauri'
import { diag } from './diag.svelte'
import { encodingFor, LOW_LAYER, type Fps, type Res } from './encodings'
import { withDeadline } from './frames'
import { installRtxTime } from './sdp'

export const STREAM_UNAVAILABLE = "Streaming isn't available on this server"
const DEADLINE_MS = 15_000

export type Picked = { track: LocalVideoTrack; label: string; surface: string; width: number; height: number }

type Watched = {
  streamer: string
  pub: RemoteTrackPublication
  track: RemoteTrack | null
  el: HTMLVideoElement
  abort: AbortController
  /** The first track is attached: later ones (after a resubscribe) attach as they arrive. */
  settled: boolean
  quality: 'auto' | 'saver'
}

/** A publication from stream-room participant `identity` with `source` is `streamer`'s screen share. */
export function isScreenOf(identity: string, source: Track.Source, streamer: string): boolean {
  return source === Track.Source.ScreenShare && identity === `${streamer}#v`
}

// createScreenTracks lives on LocalParticipant but needs no connection: an unconnected Room's is enough.
let pickerRoom: Room | null = null

export class StreamRoom {
  private published: LocalVideoTrack | null = null
  private watched: Watched | null = null

  private constructor(private readonly room: Room) {
    // The streamer's `#v` came back with a new publication, or ours rejoined (a full reconnect
    // brings new publication objects): autoSubscribe is off, so subscribe to it again.
    room.on(RoomEvent.TrackPublished, (pub: RemoteTrackPublication, p: RemoteParticipant) => {
      const w = this.watched
      if (w && pub !== w.pub && isScreenOf(p.identity, pub.source, w.streamer)) this.resubscribe(w, pub)
    })
    room.on(RoomEvent.TrackSubscribed, (track: RemoteTrack, pub: RemoteTrackPublication) => {
      const w = this.watched
      if (!w || !w.settled || pub !== w.pub || w.track === track) return
      w.track?.detach(w.el)
      w.track = track
      track.attach(w.el)
    })
    room.on(RoomEvent.Reconnected, () => {
      const w = this.watched
      const pub = w && this.screenOf(w.streamer)
      if (w && pub && (pub !== w.pub || !pub.isSubscribed)) this.resubscribe(w, pub)
    })
  }

  private screenOf(streamer: string): RemoteTrackPublication | undefined {
    return this.room.remoteParticipants.get(`${streamer}#v`)?.getTrackPublication(Track.Source.ScreenShare)
  }

  /** Moves the watched stream onto `pub` (same element, same quality); its track attaches when it arrives. */
  private resubscribe(w: Watched, pub: RemoteTrackPublication): void {
    streamCmd.log({ ev: 'resubscribe' })
    if (w.pub !== pub) w.pub.setSubscribed(false)
    w.track?.detach(w.el)
    w.track = null
    w.pub = pub
    pub.setSubscribed(true)
    pub.setVideoQuality(w.quality === 'saver' ? VideoQuality.LOW : VideoQuality.HIGH)
  }

  static async connect(channelId: string): Promise<StreamRoom> {
    const t0 = performance.now()
    installRtxTime()
    const room = new Room({ adaptiveStream: false, dynacast: true })
    // A LiveKit URL the CSP doesn't allow fails as a generic network error; the violation event says why.
    let blocked = false
    const onViolation = (e: SecurityPolicyViolationEvent) => {
      if (e.effectiveDirective === 'connect-src') blocked = true
    }
    document.addEventListener('securitypolicyviolation', onViolation)
    try {
      const t = await streamCmd.ticket(channelId, 'v')
      await room.connect(t.url, t.token, { autoSubscribe: false })
    } catch (e) {
      void room.disconnect()
      if (blocked || (e instanceof Error && e.name === 'SecurityError')) throw new Error(STREAM_UNAVAILABLE)
      throw e
    } finally {
      document.removeEventListener('securitypolicyviolation', onViolation)
    }
    const ms = Math.round(performance.now() - t0)
    diag.connectMs = ms
    streamCmd.log({ ev: 'connect', ms })
    return new StreamRoom(room)
  }

  /** The OS picker. Rejects with `NotAllowedError` when the person cancels. */
  static async pick(): Promise<Picked> {
    pickerRoom ??= new Room()
    await streamCmd.pickerWindow(true).catch(() => {})
    let tracks: LocalTrack[]
    try {
      // resolution 0×0 = uncapped: livekit-client otherwise asks for 1080p, and we offer up to the source size
      tracks = await pickerRoom.localParticipant.createScreenTracks({
        audio: false,
        selfBrowserSurface: 'exclude',
        resolution: { width: 0, height: 0 },
      })
    } finally {
      await streamCmd.pickerWindow(false).catch(() => {})
    }
    const video = tracks.find((t) => t.kind === Track.Kind.Video) as LocalVideoTrack | undefined
    for (const t of tracks) if (t !== video) t.stop()
    if (!video) throw new Error('The picker returned no video')
    const s = video.mediaStreamTrack.getSettings()
    return {
      track: video,
      label: video.mediaStreamTrack.label,
      surface: s.displaySurface ?? 'monitor',
      width: s.width ?? 0,
      height: s.height ?? 0,
    }
  }

  get connected(): boolean {
    return this.room.state !== ConnectionState.Disconnected
  }

  async publish(track: LocalVideoTrack, res: Res, fps: Fps): Promise<void> {
    const e = encodingFor(res, fps)
    await track.mediaStreamTrack.applyConstraints({ height: { max: res }, frameRate: { max: fps } })
    track.mediaStreamTrack.contentHint = e.contentHint
    await this.room.localParticipant.publishTrack(track, {
      source: Track.Source.ScreenShare,
      videoCodec: 'h264',
      simulcast: true,
      screenShareEncoding: { maxBitrate: e.maxBitrate, maxFramerate: e.maxFramerate },
      screenShareSimulcastLayers: [new VideoPreset(LOW_LAYER.width, LOW_LAYER.height, LOW_LAYER.maxBitrate, LOW_LAYER.maxFramerate)],
      degradationPreference: e.degradation,
    })
    this.published = track
  }

  /** Unpublishes; `stop` false keeps the capture running (a failed Go Live can retry with it). */
  async unpublish(stop = true): Promise<void> {
    const t = this.published
    this.published = null
    if (t) await this.room.localParticipant.unpublishTrack(t, stop)
  }

  /** Subscribes and waits for the first painted frame. `unwatch()` meanwhile rejects it with an `AbortError`. */
  async watch(streamer: string, el: HTMLVideoElement): Promise<void> {
    this.unwatch()
    const t0 = performance.now()
    const pub = this.screenOf(streamer)
    if (!pub) throw new Error('That stream has ended')
    const abort = new AbortController()
    const w: Watched = { streamer, pub, track: null, el, abort, settled: false, quality: 'auto' }
    this.watched = w
    const unwatched = new Promise<never>((_, reject) =>
      abort.signal.addEventListener('abort', () => reject(new DOMException('Stopped watching', 'AbortError')), { once: true }),
    )
    unwatched.catch(() => {}) // only ever raced
    pub.setSubscribed(true)
    let frameId: number | null = null
    try {
      // timers, not requestAnimationFrame: rAF stops while the window is hidden
      const track = await withDeadline(
        Promise.race([
          new Promise<RemoteTrack>((resolve) => {
            const check = () => {
              if (abort.signal.aborted) return
              // w.pub, not pub: a resubscribe meanwhile moves it
              if (w.pub.track) resolve(w.pub.track)
              else setTimeout(check, 50)
            }
            check()
          }),
          unwatched,
        ]),
        DEADLINE_MS,
        'No video 15 s after subscribing',
      )
      w.track = track
      track.attach(el)
      w.settled = true
      await withDeadline(
        Promise.race([new Promise<void>((resolve) => (frameId = el.requestVideoFrameCallback(() => resolve()))), unwatched]),
        DEADLINE_MS,
        'No picture 15 s after the video arrived',
      )
    } catch (e) {
      if (this.watched === w) this.unwatch() // timed out: stop polling and subscribing
      throw e
    } finally {
      if (frameId !== null) el.cancelVideoFrameCallback(frameId)
    }
    if (this.watched !== w) throw new DOMException('Stopped watching', 'AbortError')
    const ms = Math.round(performance.now() - t0)
    diag.firstFrameMs = ms
    streamCmd.log({ ev: 'first_frame', ms })
  }

  unwatch(): void {
    const w = this.watched
    this.watched = null
    if (!w) return
    w.abort.abort()
    w.track?.detach(w.el)
    w.pub.setSubscribed(false)
  }

  setQuality(q: 'auto' | 'saver'): void {
    if (!this.watched) return
    this.watched.quality = q
    this.watched.pub.setVideoQuality(q === 'saver' ? VideoQuality.LOW : VideoQuality.HIGH)
  }

  localStats(): Promise<RTCStatsReport | undefined> {
    return this.published?.getRTCStatsReport() ?? Promise.resolve(undefined)
  }

  remoteStats(): Promise<RTCStatsReport | undefined> {
    return this.watched?.track?.getRTCStatsReport() ?? Promise.resolve(undefined)
  }

  async disconnect(): Promise<void> {
    this.unwatch()
    await this.unpublish().catch(() => {})
    await this.room.disconnect()
  }
}
