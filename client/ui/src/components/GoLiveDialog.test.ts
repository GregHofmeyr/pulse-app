// @vitest-environment jsdom
// The dialog owns the picked capture until Go Live hands it over: it must stop it on every path
// where nobody else will.
import { flushSync, mount, unmount } from 'svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const live = vi.hoisted(() => ({ goLive: null as null | (() => Promise<void>), settings: [] as unknown[], probe: null as unknown, probed: [] as unknown[] }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: { window?: unknown }) => {
    if (cmd !== 'stream_audio_probe') return undefined
    live.probed.push(args.window)
    return live.probe
  },
}))
vi.mock('../lib/stream/stream.svelte', () => ({
  stream: { error: '' },
  streamApi: {
    goLive: (_c: string, _t: unknown, _l: string, settings: unknown) => {
      live.settings.push(settings)
      return live.goLive!()
    },
  },
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

  it('allow_ghost_viewers_is_on_by_default_goes_with_go_live_and_is_remembered', async () => {
    localStorage.clear()
    live.settings = []
    live.goLive = async () => {}
    let c = mount(GoLiveDialog, { target: document.body, props: { channelId: 'c1', picked: picked() as never, onClose: () => {} } })
    flushSync()
    const box = () => [...document.querySelectorAll('label')].find((l) => l.textContent?.includes('Allow ghost viewers'))!.querySelector('input')!
    expect(box().checked).toBe(true)
    box().click() // off: voice-only
    flushSync()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Go Live')!.click()
    await Promise.resolve()
    expect(live.settings).toEqual([expect.objectContaining({ allowGhosts: false })])
    unmount(c)
    c = mount(GoLiveDialog, { target: document.body, props: { channelId: 'c1', picked: picked() as never, onClose: () => {} } })
    flushSync()
    expect(box().checked).toBe(false) // the last choice, per device
    unmount(c)
  })

  describe('sound choice', () => {
    const settle = async () => {
      for (let i = 0; i < 5; i++) await Promise.resolve()
      flushSync()
    }
    const radios = () => [...document.querySelectorAll<HTMLInputElement>('input[name="sound"]')]
    const text = (r: HTMLInputElement) => r.closest('label')!.textContent!.replace(/\s+/g, ' ').trim()
    const labels = () => radios().map(text)
    const choice = (t: string) => radios().find((r) => text(r).startsWith(t))!
    const open = (p: object = picked()) => mount(GoLiveDialog, { target: document.body, props: { channelId: 'c1', picked: p as never, onClose: () => {} } })

    beforeEach(() => {
      localStorage.clear()
      live.settings = []
      live.probed = []
      live.goLive = async () => {}
    })

    it('a_window_share_offers_this_app_everything_and_no_sound_defaulting_to_this_app', async () => {
      live.probe = { identified: true, app: 'Game', playing: false }
      const c = open({ ...picked(), hwnd: 1312 })
      await settle()
      expect(document.querySelector('fieldset:has(input[name="sound"]) legend')!.textContent).toBe('Sound')
      expect(labels()).toEqual(['This app Not playing sound right now', 'Everything except Pulse', 'No sound'])
      expect(choice('This app').checked).toBe(true)
      expect(choice('This app').disabled).toBe(false)
      expect(live.probed[0]).toEqual({ hwnd: 1312, title: 'Game' })
      unmount(c)
    })

    it('a_playing_app_says_so', async () => {
      live.probe = { identified: true, app: 'Game', playing: true }
      const c = open()
      await settle()
      expect(labels()[0]).toBe('This app Playing sound')
      unmount(c)
    })

    it('this_app_is_greyed_out_only_when_its_app_cannot_be_identified', async () => {
      live.probe = { identified: false, app: null, playing: false }
      const c = open()
      await settle()
      const app = choice('This app')
      expect(app.disabled).toBe(true)
      expect(app.closest('label')!.getAttribute('data-tip')).toBe("Can't tell which app owns this window")
      expect(choice('Everything except Pulse').checked).toBe(true)
      unmount(c)
    })

    it('a_screen_share_offers_everything_and_no_sound', async () => {
      const c = open({ ...picked(), surface: 'monitor' })
      await settle()
      expect(labels()).toEqual(['Everything except Pulse', 'No sound'])
      expect(choice('Everything except Pulse').checked).toBe(true)
      expect(live.probed).toEqual([]) // nothing to identify
      unmount(c)
    })

    it('the_choice_goes_with_go_live', async () => {
      live.probe = { identified: true, app: 'Game', playing: false }
      const c = open()
      await settle()
      choice('No sound').click()
      flushSync()
      ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Go Live')!.click()
      await settle()
      expect(live.settings).toEqual([expect.objectContaining({ audio: false, sound: 'app' })])
      unmount(c)
      const c2 = open()
      await settle()
      expect(choice('No sound').checked).toBe(true) // remembered
      choice('Everything except Pulse').click()
      flushSync()
      ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Go Live')!.click()
      await settle()
      expect(live.settings[1]).toEqual(expect.objectContaining({ audio: true, sound: 'everything' }))
      unmount(c2)
    })
  })

  it('esc_cancels_and_is_marked_handled', () => {
    const onClose = vi.fn()
    const c = mount(GoLiveDialog, { target: document.body, props: { channelId: 'c1', picked: picked() as never, onClose } })
    flushSync()
    const esc = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
    window.dispatchEvent(esc)
    expect(onClose).toHaveBeenCalledTimes(1)
    // the full view under the dialog leaves it alone
    expect(esc.defaultPrevented).toBe(true)
    unmount(c)
  })
})
