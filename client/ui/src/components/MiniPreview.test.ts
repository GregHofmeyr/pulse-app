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
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [samStream], custom_emoji: [], frequent_emoji: [],
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

    q('.mini-preview')!.click()
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
    q('.mini-preview')!.click()
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

  it('drag_snaps_and_remembers_the_corner', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    const mini = q('.mini-preview')!
    document.body.getBoundingClientRect = () => ({ left: 0, top: 0, right: 1000, bottom: 600, width: 1000, height: 600, x: 0, y: 0, toJSON() {} })
    mini.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0, clientX: 900, clientY: 550 }))
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 400, clientY: 300 }))
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 120, clientY: 80 }))
    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 120, clientY: 80 }))
    // the click that ends a drag isn't a click on the preview
    mini.click()
    flushSync()
    expect(mini.classList.contains('tl')).toBe(true)
    expect(callView.goTo).toBeNull()
    expect(JSON.parse(localStorage.getItem('pulse.stream.minicorner')!)).toEqual({ corner: 'tl' })

    unmount(cmp!)
    mountMini()
    expect(q('.mini-preview')!.classList.contains('tl')).toBe(true)
  })

  it('a_plain_click_goes_to_the_call', async () => {
    await streamApi.watch('V1', 'U2')
    mountMini()
    const mini = q('.mini-preview')!
    mini.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0, clientX: 900, clientY: 550 }))
    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 901, clientY: 550 }))
    mini.click()
    expect(callView.goTo).toBe('V1')
  })
})
