// @vitest-environment jsdom
// The viewer side of StreamRoom against a fake LiveKit room: watching survives the streamer's
// `#v` coming back with a new publication, and our own reconnect.
import type { EventEmitter } from 'node:events'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const logged = vi.hoisted(() => [] as string[])
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args?: { line?: string }) => {
    if (cmd === 'stream_ticket') return { url: 'ws://lk', token: 't' }
    if (cmd === 'stream_log' && args?.line) logged.push(args.line)
    return undefined
  },
}))

const rooms = vi.hoisted(() => [] as unknown[])
vi.mock('livekit-client', async (importOriginal) => {
  const real = await importOriginal<typeof import('livekit-client')>()
  const { EventEmitter } = await import('node:events')
  class FakeRoom extends EventEmitter {
    state = real.ConnectionState.Connected
    remoteParticipants = new Map<string, unknown>()
    localParticipant = {}
    constructor() {
      super()
      rooms.push(this)
    }
    async connect() {}
    async disconnect() {}
  }
  return { ...real, Room: FakeRoom }
})

import { RoomEvent, Track, VideoQuality } from 'livekit-client'
import { isScreenOf, StreamRoom } from './room'

type FakeRoom = EventEmitter & { remoteParticipants: Map<string, unknown> }

function track() {
  return { attach: vi.fn(), detach: vi.fn(), getRTCStatsReport: async () => undefined }
}

function publication(t: ReturnType<typeof track> | null, source = Track.Source.ScreenShare) {
  const pub = {
    source,
    track: t,
    isSubscribed: false,
    setSubscribed: vi.fn((on: boolean) => void (pub.isSubscribed = on)),
    setVideoQuality: vi.fn(),
  }
  return pub
}

/** `identity` joins the fake room publishing `pub`; returns the participant. */
function join(room: FakeRoom, identity: string, pub: ReturnType<typeof publication>) {
  const p = { identity, getTrackPublication: (s: Track.Source) => (s === pub.source ? pub : undefined) }
  room.remoteParticipants.set(identity, p)
  return p
}

const videoEl = () =>
  Object.assign(document.createElement('video'), {
    requestVideoFrameCallback: (cb: () => void) => (setTimeout(cb, 0), 1),
    cancelVideoFrameCallback: () => {},
  })

/** Watching `other` in a fresh fake room. */
async function watching() {
  const sr = await StreamRoom.connect('c1')
  const room = rooms.at(-1) as FakeRoom
  const t1 = track()
  const pub1 = publication(t1)
  join(room, 'other#v', pub1)
  const el = videoEl()
  await sr.watch('other', el)
  return { sr, room, el, t1, pub1 }
}

beforeEach(() => {
  logged.length = 0
})

describe('isScreenOf', () => {
  it('only_the_streamers_v_screen_share', () => {
    expect(isScreenOf('alex#v', Track.Source.ScreenShare, 'alex')).toBe(true)
    expect(isScreenOf('alex#a', Track.Source.ScreenShare, 'alex')).toBe(false)
    expect(isScreenOf('alex#v', Track.Source.Camera, 'alex')).toBe(false)
    expect(isScreenOf('kim#v', Track.Source.ScreenShare, 'alex')).toBe(false)
  })
})

describe('StreamRoom watching', () => {
  it('resubscribes_when_the_streamer_republishes', async () => {
    const { sr, room, el, t1, pub1 } = await watching()
    expect(t1.attach).toHaveBeenCalledWith(el)
    sr.setQuality('saver')

    // the streamer's `#v` reconnected in full: a new participant object and publication
    const t2 = track()
    const pub2 = publication(null)
    const p2 = join(room, 'other#v', pub2)
    room.emit(RoomEvent.TrackPublished, pub2, p2)
    expect(pub2.setSubscribed).toHaveBeenCalledWith(true)
    expect(pub2.setVideoQuality).toHaveBeenCalledWith(VideoQuality.LOW) // the chosen quality carries over
    expect(t1.detach).toHaveBeenCalledWith(el)
    expect(pub1.setSubscribed).toHaveBeenLastCalledWith(false)
    expect(logged.some((l) => JSON.parse(l).ev === 'resubscribe')).toBe(true)

    room.emit(RoomEvent.TrackSubscribed, t2, pub2, p2)
    expect(t2.attach).toHaveBeenCalledWith(el) // same element
  })

  it('ignores_other_publications', async () => {
    const { room, pub1 } = await watching()
    const camera = publication(null, Track.Source.Camera)
    room.emit(RoomEvent.TrackPublished, camera, { identity: 'other#v' })
    const someoneElse = publication(null)
    room.emit(RoomEvent.TrackPublished, someoneElse, { identity: 'kim#v' })
    expect(camera.setSubscribed).not.toHaveBeenCalled()
    expect(someoneElse.setSubscribed).not.toHaveBeenCalled()
    expect(pub1.setSubscribed).toHaveBeenCalledTimes(1) // only the original subscribe
  })

  it('resubscribes_after_our_own_reconnect', async () => {
    const { room, pub1 } = await watching()
    pub1.isSubscribed = false // the subscription didn't survive
    room.emit(RoomEvent.Reconnected)
    expect(pub1.setSubscribed).toHaveBeenLastCalledWith(true)
    expect(pub1.setSubscribed).toHaveBeenCalledTimes(2)
  })

  it('nothing_after_unwatch', async () => {
    const { sr, room } = await watching()
    sr.unwatch()
    const pub2 = publication(null)
    room.emit(RoomEvent.TrackPublished, pub2, join(room, 'other#v', pub2))
    expect(pub2.setSubscribed).not.toHaveBeenCalled()
  })
})
