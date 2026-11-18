// @vitest-environment jsdom
// The dialog owns the picked capture until Go Live hands it over: it must stop it on every path
// where nobody else will.
import { flushSync, mount, unmount } from 'svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const live = vi.hoisted(() => ({ goLive: null as null | (() => Promise<void>), settings: [] as unknown[], probe: null as unknown, probed: [] as unknown[], clash: null as null | (() => unknown), clashes: false }))
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
    // a ghost watch elsewhere (#360): the clash is asked about, `run` going live once it's settled
    goLiveClashes: (_c: string, run: () => unknown) => {
      if (!live.clashes) return false
      live.clash = run
      return true
    },
    goLive: (_c: string, _t: unknown, _l: string, settings: unknown) => {
      live.settings.push(settings)
      return live.goLive!()
    },
  },
}))

import GoLiveDialog from './GoLiveDialog.svelte'
import { app, resetState } from '../lib/store.svelte'
import { chooseOption, openSelect, selectLabels, selectTrigger, selectValue } from './selectHarness'

const LOUNGE = { id: 'c1', server_id: 'S1', kind: 'voice' as const, name: 'Lounge', position: 0 }
const DM = { id: 'DM1', server_id: null, kind: 'dm' as const, name: null, position: 0 }

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
    resetState()
    app.state = { ...app.state, channels: { c1: LOUNGE, DM1: DM } }
  })

  it('a_ghost_watch_started_while_the_dialog_was_open_is_asked_about_at_the_go_live_press', async () => {
    live.clashes = true
    live.clash = null
    live.settings = []
    const { going } = startGoLive()
    // asked first: nothing has gone live, and the button is still there to press
    expect(live.clash).not.toBeNull()
    expect(live.settings).toEqual([])
    expect(document.querySelector('.btn-primary')?.textContent).toBe('Go Live')
    // "Stop & go live": it goes live, without asking again
    live.clashes = false
    void live.clash!()
    flushSync()
    expect(live.settings).toHaveLength(1)
    going.resolve()
    await going.p
  })

  it('a_clash_answered_after_the_dialog_closed_goes_live_with_nothing', () => {
    live.clashes = true
    live.clash = null
    live.settings = []
    const { c, track } = startGoLive()
    unmount(c) // the dialog went (left voice) while the clash dialog was up
    void live.clash!()
    expect(live.settings).toEqual([])
    expect(track.stop).toHaveBeenCalledTimes(1)
    live.clashes = false
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

  it('a_dm_or_group_call_has_no_ghost_viewers_and_keeps_the_saved_choice_for_servers', async () => {
    localStorage.clear()
    live.settings = []
    live.goLive = async () => {}
    const ghostBox = () => [...document.querySelectorAll('label')].find((l) => l.textContent?.includes('Allow ghost viewers'))
    let c = mount(GoLiveDialog, { target: document.body, props: { channelId: 'DM1', picked: picked() as never, onClose: () => {} } })
    flushSync()
    expect(ghostBox()).toBeUndefined()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Go Live')!.click()
    await Promise.resolve()
    expect(live.settings).toEqual([expect.objectContaining({ allowGhosts: false })])
    unmount(c)
    // the server channel's choice is still the default (on), not the DM's forced off
    c = mount(GoLiveDialog, { target: document.body, props: { channelId: 'c1', picked: picked() as never, onClose: () => {} } })
    flushSync()
    expect(ghostBox()!.querySelector('input')!.checked).toBe(true)
    unmount(c)
  })

  describe('sound choice (#458: the Pulse dropdown)', () => {
    const settle = async () => {
      for (let i = 0; i < 5; i++) await Promise.resolve()
      flushSync()
    }
    const trigger = () => selectTrigger('Sound')!
    const labels = () => selectLabels(trigger())
    const current = () => selectValue(trigger())
    const option = (v: string) => openSelect(trigger()).querySelector<HTMLElement>(`[role=option][data-value="${v}"]`)!
    const open = (p: object = picked()) => mount(GoLiveDialog, { target: document.body, props: { channelId: 'c1', picked: p as never, onClose: () => {} } })
    const ICON = 'data:image/png;base64,AAAA'

    beforeEach(() => {
      localStorage.clear()
      live.settings = []
      live.probed = []
      live.goLive = async () => {}
    })

    it('a_window_share_offers_the_app_by_name_everything_and_no_sound_defaulting_to_the_app', async () => {
      live.probe = { identified: true, app: 'Space Game', playing: false, level: 0, icon: ICON }
      const c = open({ ...picked(), hwnd: 1312 })
      await settle()
      expect(document.querySelector('input[name="sound"]')).toBeNull() // no native radios
      expect(document.querySelector('.sound-row .legend')!.textContent).toBe('Sound')
      expect(labels()).toEqual(['Space Game', 'Everything except Pulse and Discord', 'No sound'])
      expect(current()).toBe('app')
      expect(live.probed[0]).toEqual({ hwnd: 1312, title: 'Game' })
      unmount(c)
    })

    it('the_app_row_shows_its_icon_and_the_others_theirs', async () => {
      live.probe = { identified: true, app: 'Space Game', playing: false, level: 0, icon: ICON }
      const c = open()
      await settle()
      expect(trigger().querySelector<HTMLImageElement>('img.app-icon')!.getAttribute('src')).toBe(ICON)
      expect(option('everything').querySelector('svg')).not.toBeNull()
      expect(option('off').querySelector('svg')).not.toBeNull()
      unmount(c)
    })

    it('without an icon the app row falls back to a window icon, and without a name to This app', async () => {
      live.probe = { identified: true, app: null, playing: false, level: 0, icon: null }
      const c = open()
      await settle()
      expect(trigger().querySelector('img')).toBeNull()
      expect(trigger().querySelector('svg')).not.toBeNull()
      expect(labels()[0]).toBe('This app')
      unmount(c)
    })

    it('the_selected_app_row_has_a_live_level_meter', async () => {
      live.probe = { identified: true, app: 'Space Game', playing: true, level: 40 }
      const c = open()
      await settle()
      const meter = trigger().querySelector<HTMLElement>('[data-sound-level]')!
      expect(meter.getAttribute('aria-label')).toBe('Playing sound')
      expect(meter.querySelector<HTMLElement>('.fill')!.style.transform).toBe('scaleX(0.4)')
      // in the list, on the selected row only
      expect(option('app').querySelector('[data-sound-level]')).not.toBeNull()
      expect(option('everything').querySelector('[data-sound-level]')).toBeNull()
      unmount(c)
      live.probe = { identified: true, app: 'Space Game', playing: false, level: 0 }
      const c2 = open()
      await settle()
      expect(trigger().querySelector('[data-sound-level]')!.getAttribute('aria-label')).toBe('Not playing sound right now')
      expect(trigger().querySelector<HTMLElement>('[data-sound-level] .fill')!.style.transform).toBe('scaleX(0)')
      unmount(c2)
    })

    it('no meter once another row is picked', async () => {
      live.probe = { identified: true, app: 'Space Game', playing: true, level: 40 }
      const c = open()
      await settle()
      chooseOption(trigger(), 'everything')
      expect(current()).toBe('everything')
      expect(trigger().querySelector('[data-sound-level]')).toBeNull()
      unmount(c)
    })

    it('the_app_row_is_disabled_with_the_reason_when_its_app_cannot_be_identified', async () => {
      live.probe = { identified: false, app: null, playing: false, level: 0 }
      const c = open()
      await settle()
      const row = option('app')
      expect(row.getAttribute('aria-disabled')).toBe('true')
      expect(row.getAttribute('data-tip')).toBe("Can't tell which app owns this window")
      expect(current()).toBe('everything')
      unmount(c)
    })

    it('a_screen_share_offers_everything_and_no_sound', async () => {
      const c = open({ ...picked(), surface: 'monitor' })
      await settle()
      expect(labels()).toEqual(['Everything except Pulse and Discord', 'No sound'])
      expect(current()).toBe('everything')
      expect(live.probed).toEqual([]) // nothing to identify
      unmount(c)
    })

    it('the_choice_goes_with_go_live', async () => {
      live.probe = { identified: true, app: 'Game', playing: false, level: 0 }
      const c = open()
      await settle()
      chooseOption(trigger(), 'off')
      ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Go Live')!.click()
      await settle()
      expect(live.settings).toEqual([expect.objectContaining({ audio: false, sound: 'app' })])
      unmount(c)
      const c2 = open()
      await settle()
      expect(current()).toBe('off') // remembered
      chooseOption(trigger(), 'everything')
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
