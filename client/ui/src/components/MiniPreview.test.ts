// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a), convertFileSrc: (s: string) => s }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true, minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }),
}))
const room = vi.hoisted(() => ({
  connected: true,
  disconnect: async () => {},
  watch: async () => 0,
  unwatch: () => {},
  setLayer: vi.fn(),
  remoteStats: async () => undefined,
}))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => room, pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import Shell from '../routes/Shell.svelte'
import MiniPreview from './MiniPreview.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { voice } from '../lib/voice.svelte'
import { stream, streamApi } from '../lib/stream/stream.svelte'
import { callView, openFull } from '../lib/stream/viewer.svelte'
import type { Ready } from '../lib/protocol/Ready'

const me = { id: 'U1', username: 'alex', avatar_hash: null }
const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const flags = { muted: false, deafened: false }
const samStream = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false }, status: 'live' as const, viewers: [], started_at: '2026-10-26T10:00:00Z' }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [
    { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 1 },
  ],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [{ channel_id: 'V1', members: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }] }],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [samStream], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
}
const SAM = { channelId: 'V1', streamer: 'U2' }

let cmp: ReturnType<typeof mount> | undefined
const q = (sel: string) => document.querySelector<HTMLElement>(sel)
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  await new Promise((r) => setTimeout(r, 0))
  flushSync()
}
const mountShell = () => {
  cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
  flushSync()
}
/** Shows a channel in the main area, as the shell's own navigation would. */
const goTo = (id: string) => {
  callView.goTo = id
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  // a text channel opening asks for its history
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : undefined))
  // jsdom has no scrolling
  Element.prototype.scrollTo ??= () => {}
  localStorage.clear()
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Windows NT 10.0)')
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  app.state = applyReady(emptyState(), ready)
  voice.channelId = 'V1'
  voice.joining = null
  voice.leaving = null
  voice.joinFailed = null
  voice.connection = 'connected'
  voice.stage = { stage: 'connected', attempt: null }
  stream.live = null
  stream.watching = []
  callView.full = null
  callView.goTo = null
  callView.joinThenWatch = null
  room.setLayer.mockClear()
})
afterEach(async () => {
  if (cmp) unmount(cmp)
  cmp = undefined
  callView.full = null
  await streamApi.unwatchAll()
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('mini-preview in the shell', () => {
  it('shows_when_watching_and_away_from_the_call', async () => {
    mountShell()
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    expect(q('.mini-preview')).toBeNull()
    expect(q('[data-stream="U2"] video')).not.toBeNull()

    goTo('T1')
    await settle()
    const mini = q('.mini-preview')!
    expect(mini).not.toBeNull()
    expect(mini.querySelector('video')).not.toBeNull()
    expect(document.querySelectorAll('video')).toHaveLength(1)
    expect(mini.classList.contains('br')).toBe(true)
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'low')
  })

  it('click_returns_to_the_call_view_in_full_view_if_you_left_from_there', async () => {
    mountShell()
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    expect(q('.full-view')).not.toBeNull()

    goTo('T1')
    await settle()
    expect(callView.full).toEqual(SAM)
    // away from the call, the mini-preview takes the low layer
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'low')

    q('.mini-preview .open')!.click()
    await settle()
    expect(q('.mini-preview')).toBeNull()
    expect(q('.full-view .stage video')).not.toBeNull()
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'high')
  })

  it('click_returns_to_the_grid_if_you_left_from_there', async () => {
    mountShell()
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    goTo('T1')
    await settle()
    q('.mini-preview .open')!.click()
    await settle()
    expect(q('.full-view')).toBeNull()
    expect(q('[data-stream="U2"] video')).not.toBeNull()
  })
})

describe('mini-preview', () => {
  const mountMini = () => {
    cmp = mount(MiniPreview, { target: document.body, props: SAM })
    flushSync()
  }

  it('x_stops_watching', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    q('.mini-preview button[aria-label="Stop watching"]')!.click()
    await settle()
    expect(stream.watching).toEqual([])
    expect(invoke).toHaveBeenCalledWith('stream_unwatch', { channelId: 'V1', streamer: 'U2' })
    // the ✕ is not a click on the preview
    expect(callView.goTo).toBeNull()
  })

  /** A mouse click (detail 1); `el.click()` has detail 0, like a keyboard activation. */
  const mouseClick = (el: Element) => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 }))
  /** A pointer event with the primary button held (`buttons` 1) until the release. */
  const ptr = (type: string, x: number, y: number) =>
    new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, buttons: type === 'pointerup' ? 0 : 1, clientX: x, clientY: y })
  const area = () => vi.spyOn(document.body, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 0, right: 1000, bottom: 600, width: 1000, height: 600, x: 0, y: 0, toJSON() {} } as DOMRect)
  /** The box under the pointer, with pointer capture (jsdom has none) recorded. */
  const box = () => {
    const mini = q('.mini-preview')!
    const capture = vi.fn()
    Object.assign(mini, { setPointerCapture: capture, releasePointerCapture: vi.fn(), hasPointerCapture: () => true })
    return { mini, open: mini.querySelector<HTMLElement>('.open')!, capture }
  }

  it('the_preview_is_a_plain_box_with_an_open_button_and_a_separate_x', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    const mini = q('.mini-preview')!
    expect(mini.getAttribute('role')).not.toBe('button')
    const open = mini.querySelector('button.open')!
    expect(open.getAttribute('aria-label')).toBe("Back to sam's stream")
    // no control inside another
    expect(open.querySelector('button, [role="button"]')).toBeNull()
    expect(mini.querySelector('button[aria-label="Stop watching"]')!.closest('.open')).toBeNull()
    ;(open as HTMLElement).click() // keyboard activation (Enter/Space on a button)
    expect(callView.goTo).toBe('V1')
  })

  it('drag_snaps_and_remembers_the_corner', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    area()
    const { mini, open, capture } = box()
    open.dispatchEvent(ptr('pointerdown', 900, 550))
    // a press alone doesn't capture: capture would retarget a plain click away from the open button
    expect(capture).not.toHaveBeenCalled()
    mini.dispatchEvent(ptr('pointermove', 898, 551))
    expect(capture).not.toHaveBeenCalled()
    mini.dispatchEvent(ptr('pointermove', 400, 300))
    // past the threshold it's a drag: the box takes the pointer
    expect(capture).toHaveBeenCalledTimes(1)
    mini.dispatchEvent(ptr('pointermove', 120, 80))
    mini.dispatchEvent(ptr('pointerup', 120, 80))
    // the click that ends a drag isn't a click on the preview
    mouseClick(open)
    flushSync()
    expect(mini.classList.contains('tl')).toBe(true)
    expect(callView.goTo).toBeNull()
    expect(JSON.parse(localStorage.getItem('pulse.stream.minicorner')!)).toEqual({ corner: 'tl' })

    unmount(cmp!)
    mountMini()
    expect(q('.mini-preview')!.classList.contains('tl')).toBe(true)
  })

  it('a_cancelled_drag_goes_back_and_saves_nothing', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    area()
    for (const end of ['pointercancel', 'lostpointercapture']) {
      const { mini, open } = box()
      open.dispatchEvent(ptr('pointerdown', 900, 550))
      mini.dispatchEvent(ptr('pointermove', 120, 80))
      flushSync()
      expect(mini.classList.contains('dragging')).toBe(true)
      mini.dispatchEvent(ptr(end, 120, 80))
      flushSync()
      expect(mini.classList.contains('dragging')).toBe(false)
      expect(mini.classList.contains('br')).toBe(true)
      // a later move or release does nothing
      mini.dispatchEvent(ptr('pointermove', 100, 60))
      mini.dispatchEvent(ptr('pointerup', 100, 60))
      flushSync()
      expect(mini.classList.contains('br')).toBe(true)
    }
    expect(localStorage.getItem('pulse.stream.minicorner')).toBeNull()
  })

  it('keyboard_open_works_right_after_a_drag', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    area()
    const { mini, open } = box()
    open.dispatchEvent(ptr('pointerdown', 900, 550))
    mini.dispatchEvent(ptr('pointermove', 120, 80))
    mini.dispatchEvent(ptr('pointerup', 120, 80))
    // released off the preview: no click came to clear the drag
    open.click() // Enter on the focused button
    expect(callView.goTo).toBe('V1')
  })

  it('a_drag_released_off_the_preview_does_not_swallow_the_next_click', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    area()
    const { mini, open } = box()
    open.dispatchEvent(ptr('pointerdown', 900, 550))
    mini.dispatchEvent(ptr('pointermove', 120, 80))
    mini.dispatchEvent(ptr('pointerup', 120, 80))
    await new Promise((r) => setTimeout(r, 0))
    mouseClick(open)
    expect(callView.goTo).toBe('V1')
  })

  it('a_plain_click_does_not_capture_and_goes_to_the_call', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    const { mini, open, capture } = box()
    open.dispatchEvent(ptr('pointerdown', 900, 550))
    mini.dispatchEvent(ptr('pointermove', 901, 550))
    mini.dispatchEvent(ptr('pointerup', 901, 550))
    mouseClick(open)
    expect(capture).not.toHaveBeenCalled()
    expect(callView.goTo).toBe('V1')
  })

  it('a_press_released_off_the_box_does_not_start_a_drag_on_a_later_hover', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    area()
    const { mini, open, capture } = box()
    open.dispatchEvent(ptr('pointerdown', 900, 550))
    // released elsewhere (no capture yet, so the box never hears it); later the mouse passes over, no button held
    mini.dispatchEvent(new MouseEvent('pointermove', { bubbles: true, buttons: 0, clientX: 100, clientY: 60 }))
    flushSync()
    expect(mini.classList.contains('dragging')).toBe(false)
    expect(capture).not.toHaveBeenCalled()
  })
})
