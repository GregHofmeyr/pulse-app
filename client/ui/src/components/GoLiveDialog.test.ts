// @vitest-environment jsdom
// The dialog owns the picked capture until Go Live hands it over: it must stop it on every path
// where nobody else will.
import { flushSync, mount, unmount } from 'svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const live = vi.hoisted(() => ({ goLive: null as null | (() => Promise<void>) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))
vi.mock('../lib/stream/stream.svelte', () => ({
  stream: { error: '' },
  streamApi: { goLive: () => live.goLive!() },
}))

import GoLiveDialog from './GoLiveDialog.svelte'

function picked() {
  return { track: { mediaStreamTrack: {}, stop: vi.fn() }, label: 'Game', surface: 'window', width: 1920, height: 1080 }
}

function deferred() {
  let resolve!: () => void
  let reject!: (e: unknown) => void
  const p = new Promise<void>((res, rej) => ((resolve = res), (reject = rej)))
  return { p, resolve, reject }
}

/** Opens the dialog and clicks Go Live, with `goLive` pending until the test settles it. */
function startGoLive() {
  const going = deferred()
  live.goLive = () => going.p
  const p = picked()
  const c = mount(GoLiveDialog, { target: document.body, props: { channelId: 'c1', picked: p as never, onClose: () => {} } })
  flushSync()
  ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Go Live')!.click()
  flushSync()
  return { c, going, track: p.track }
}

describe('GoLiveDialog capture ownership', () => {
  beforeEach(() => {
    globalThis.MediaStream ??= class {} as never
    document.body.innerHTML = ''
  })

  it('a_failed_go_live_after_the_dialog_closed_stops_the_capture', async () => {
    const { c, going, track } = startGoLive()
    unmount(c) // e.g. the voice panel went away mid Go Live
    expect(track.stop).not.toHaveBeenCalled() // handed over: the stream may still take it
    going.reject(new Error('no'))
    await going.p.catch(() => {})
    await Promise.resolve()
    expect(track.stop).toHaveBeenCalledTimes(1)
  })

  it('a_failed_go_live_keeps_the_capture_for_retry', async () => {
    const { c, going, track } = startGoLive()
    going.reject(new Error('no'))
    await going.p.catch(() => {})
    await Promise.resolve()
    expect(track.stop).not.toHaveBeenCalled()
    unmount(c) // Cancel after the failure: now the dialog stops it
    expect(track.stop).toHaveBeenCalledTimes(1)
  })
})
