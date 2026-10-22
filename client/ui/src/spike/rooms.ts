// THROWAWAY (screen-share spike): the JS side of the stream room (`<uid>#v` / `#v2` in `<channel>~s`).
import { Room, Track, VideoPreset, type LocalTrackPublication, type RemoteTrack, type ScreenShareCaptureOptions, type TrackPublishOptions } from 'livekit-client'
import { invoke } from '@tauri-apps/api/core'
import { encodingFor, type Fps, type Res } from './encodings'
import { addRtxTime } from './sdp'
import { spikeLog } from './log'

type Ticket = { url: string; token: string }
const ticket = (channelId: string, role: 'v' | 'v2') => invoke<Ticket>('spike_ticket', { channelId, role })

export type PublishOpts = { res: Res; fps: Fps; lowLayer: boolean; codec: 'h264' | 'av1' }

/** S4(b): when on, every answer this page creates asks for rtx-time=300. */
let rtxTimeOn = false
let patched = false
function patchAnswers() {
  if (patched) return
  patched = true
  type SetLocal = (this: RTCPeerConnection, desc?: RTCLocalSessionDescriptionInit) => Promise<void>
  const orig = RTCPeerConnection.prototype.setLocalDescription as SetLocal
  const patchedFn: SetLocal = function (desc) {
    if (rtxTimeOn && desc?.type === 'answer' && desc.sdp) desc = { ...desc, sdp: addRtxTime(desc.sdp, 300) }
    return orig.call(this, desc)
  }
  RTCPeerConnection.prototype.setLocalDescription = patchedFn as typeof RTCPeerConnection.prototype.setLocalDescription
}

export async function connect(channelId: string, role: 'v' | 'v2', rtxTime: boolean): Promise<Room> {
  const t0 = performance.now()
  patchAnswers()
  rtxTimeOn = rtxTime
  const room = new Room({ adaptiveStream: false, dynacast: true })
  const tk = await ticket(channelId, role)
  await room.connect(tk.url, tk.token, { autoSubscribe: false })
  spikeLog({ test: 'S12', event: 'connected', role, ms: Math.round(performance.now() - t0) })
  return room
}

// createScreenTracks lives on LocalParticipant but needs no connection: an unconnected Room's is enough.
const pickerRoom = new Room()

/** Pick a screen/window (the OS picker) and describe what came back (S8). */
export async function pick(o: Pick<PublishOpts, 'res' | 'fps'>) {
  const e = encodingFor(o.res, o.fps)
  const capture: ScreenShareCaptureOptions = {
    audio: false,
    resolution: { width: Math.round((o.res * 16) / 9), height: o.res, frameRate: o.fps },
    contentHint: e.contentHint,
    selfBrowserSurface: 'exclude',
  }
  const t0 = performance.now()
  const tracks = await pickerRoom.localParticipant.createScreenTracks(capture)
  const video = tracks.find((t) => t.kind === Track.Kind.Video)
  if (!video) throw new Error('no video track from the picker')
  for (const t of tracks) if (t !== video) t.stop()
  const settings = video.mediaStreamTrack.getSettings()
  const info = {
    label: video.mediaStreamTrack.label,
    displaySurface: settings.displaySurface,
    w: settings.width,
    h: settings.height,
    pickMs: Math.round(performance.now() - t0),
  }
  return { video, info }
}

export async function publish(room: Room, o: PublishOpts): Promise<LocalTrackPublication> {
  const e = encodingFor(o.res, o.fps)
  const { video, info } = await pick(o)
  const pubOpts: TrackPublishOptions = {
    source: Track.Source.ScreenShare,
    videoCodec: o.codec,
    screenShareEncoding: { maxBitrate: e.maxBitrate, maxFramerate: e.maxFramerate },
    screenShareSimulcastLayers: o.lowLayer ? [new VideoPreset(640, 360, 500_000, 30)] : [],
    degradationPreference: e.degradation,
    simulcast: o.lowLayer,
  }
  const pub = await room.localParticipant.publishTrack(video, pubOpts)
  spikeLog({ test: 'S8', event: 'picked', ...info })
  return pub
}

/** The first `<uid>#v` in the room that publishes a screen share (when no streamer is named). */
export function firstStreamer(room: Room): string | undefined {
  for (const p of room.remoteParticipants.values()) {
    if (p.identity.endsWith('#v') && p.getTrackPublication(Track.Source.ScreenShare)) return p.identity.slice(0, -2)
  }
  return undefined
}

export async function watch(room: Room, streamer: string, el: HTMLVideoElement): Promise<RemoteTrack> {
  const t0 = performance.now()
  const p = room.remoteParticipants.get(`${streamer}#v`)
  const pub = p?.getTrackPublication(Track.Source.ScreenShare)
  if (!pub) throw new Error(`${streamer} is not publishing`)
  pub.setSubscribed(true)
  const deadline = Date.now() + 15_000
  return new Promise<RemoteTrack>((resolve, reject) => {
    const check = () => {
      if (!pub.track) {
        if (Date.now() > deadline) return reject(new Error('no track 15 s after subscribing'))
        return void requestAnimationFrame(check)
      }
      pub.track.attach(el)
      el.requestVideoFrameCallback(() => {
        spikeLog({ test: 'S12', event: 'first_frame', ms: Math.round(performance.now() - t0), epochMs: Date.now() })
        resolve(pub.track!)
      })
    }
    check()
  })
}
