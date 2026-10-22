// @vitest-environment jsdom
// The stream store's lifecycle against a fake `#v` connection and a fake Tauri core.
import { beforeEach, describe, expect, it, vi } from 'vitest'

type Handler = (args: Record<string, unknown> | undefined) => Promise<unknown>
const core = vi.hoisted(() => ({
  handlers: {} as Record<string, Handler>,
  calls: [] as [string, Record<string, unknown> | undefined][],
}))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => {
    core.calls.push([cmd, args])
    return core.handlers[cmd]?.(args) ?? Promise.resolve(undefined)
  },
}))

const fake = vi.hoisted(() => ({ connect: null as null | (() => Promise<unknown>) }))
vi.mock('./room', () => ({ StreamRoom: { connect: () => fake.connect!() }, STREAM_UNAVAILABLE: 'unavailable' }))
vi.mock('./diag.svelte', () => ({ startDiagnostics: () => () => {} }))

function deferred<T = unknown>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const p = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)))
  return { p, resolve, reject }
}

function fakeRoom() {
  return {
    connected: true,
    publish: vi.fn(async () => {}),
    unpublish: vi.fn(async () => {}),
    disconnect: vi.fn(async () => {}),
    watch: vi.fn(async () => {}),
    unwatch: vi.fn(),
    setQuality: vi.fn(),
    localStats: async () => undefined,
    remoteStats: async () => undefined,
  }
}

/** A picked capture: `end()` closes it the way a closed window does. */
function fakeTrack(surface: 'monitor' | 'window' = 'monitor') {
  const media = Object.assign(new EventTarget(), {
    readyState: 'live',
    getSettings: () => ({ displaySurface: surface }),
  })
  return {
    mediaStreamTrack: media,
    stop: vi.fn(),
    end() {
      media.readyState = 'ended'
      media.dispatchEvent(new Event('ended'))
    },
  }
}

const calls = (cmd: string) => core.calls.filter(([c]) => c === cmd).map(([, a]) => a)
const settle = () => new Promise((r) => setTimeout(r, 0))
const settings = (audio: boolean) => ({ res: 720 as const, fps: 60 as const, audio })

async function load() {
  vi.resetModules()
  const mod = await import('./stream.svelte')
  const { app } = await import('../store.svelte')
  app.state = { ...app.state, me: { id: 'me', username: 'me' } as never }
  return mod
}

/** Live in c1 with a fresh fake connection. */
async function goneLive(audio = false, surface: 'monitor' | 'window' = 'monitor') {
  const mod = await load()
  const room = fakeRoom()
  fake.connect = async () => room
  const track = fakeTrack(surface)
  await mod.streamApi.goLive('c1', track as never, 'Game', settings(audio))
  expect(mod.stream.live).not.toBeNull()
  return { ...mod, room, track }
}

beforeEach(() => {
  core.handlers = {}
  core.calls = []
  // jsdom has no media playback or canvas: the preview loop must not log "not implemented"
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  globalThis.MediaStream ??= class {} as never
})

describe('go live', () => {
  it('a_go_live_cancelled_before_publish_stops_the_capture', async () => {
    const { streamApi, stream } = await load()
    const room = fakeRoom()
    const connecting = deferred<typeof room>()
    fake.connect = () => connecting.p
    const track = fakeTrack()
    const going = streamApi.goLive('c1', track as never, 'Game', settings(false))
    await streamApi.stop() // Stop (or a voice change) while the connection is still coming up
    connecting.resolve(room)
    await going
    expect(room.publish).not.toHaveBeenCalled()
    expect(track.stop).toHaveBeenCalled()
    expect(stream.live).toBeNull()
  })

  it('a_share_ending_while_go_live_finishes_lets_the_connection_go', async () => {
    const { streamApi, stream } = await load()
    const room = fakeRoom()
    fake.connect = async () => room
    const audioStarted = deferred()
    core.handlers.stream_audio_start = () => audioStarted.p
    const track = fakeTrack()
    const going = streamApi.goLive('c1', track as never, 'Game', settings(true))
    while (!calls('stream_audio_start').length) await settle()
    track.end() // the shared window closed while stream audio was starting
    await settle()
    audioStarted.resolve(undefined)
    await going
    await settle()
    expect(stream.live).toBeNull()
    expect(room.disconnect).toHaveBeenCalled()
    expect(stream.room).toBeNull()
  })
})

describe('re-announce after a server restart', () => {
  const ready = { me: { id: 'me' }, streams: [] } as never

  it('reannounce_success_keeps_the_share', async () => {
    const { streamApi, stream } = await goneLive()
    await streamApi.onReady(ready)
    expect(calls('stream_reannounce')).toEqual([{ channelId: 'c1', settings: { height: 720, fps: 60, audio: false } }])
    expect(stream.live).not.toBeNull()
  })

  it('reannounce_failure_ends_the_local_share_and_says_so', async () => {
    const { streamApi, stream, room } = await goneLive()
    core.handlers.stream_reannounce = () => Promise.reject('the server is having trouble (503)')
    await streamApi.onReady(ready)
    expect(stream.live).toBeNull()
    expect(room.unpublish).toHaveBeenCalled()
    expect(stream.notice).toMatch(/stream stopped/)
    expect(calls('stream_stop')).toEqual([]) // the server has no stream to stop
  })
})
