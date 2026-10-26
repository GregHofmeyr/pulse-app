// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a), convertFileSrc: (s: string) => s }))
/** The core's events, by name: tests emit them as the Rust side would. */
const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>())
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, fn: (e: { payload: unknown }) => void) => {
    handlers.set(name, fn)
    return () => handlers.delete(name)
  },
}))
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

import Shell from './Shell.svelte'
import LivePopover from '../components/LivePopover.svelte'
import { app, startListening } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { startVoiceListening, voice } from '../lib/voice.svelte'
import { stream, streamApi } from '../lib/stream/stream.svelte'
import { callView, joinAndWatch, openFull, showToast } from '../lib/stream/viewer.svelte'
import type { Ready } from '../lib/protocol/Ready'

let cmp: ReturnType<typeof mount> | undefined
let offs: (() => void)[] = []
beforeEach(() => {
  invoke.mockReset()
  // a text channel opening asks for its history
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : undefined))
  // jsdom has no scrolling
  Element.prototype.scrollTo ??= () => {}
})
afterEach(async () => {
  if (cmp) unmount(cmp)
  cmp = undefined
  offs.forEach((off) => off())
  offs = []
  callView.full = null
  callView.goTo = null
  callView.joinThenWatch = null
  await streamApi.unwatchAll()
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('Shell', () => {
  it('server settings and the server menu close when you switch server', () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }, { id: 'S2', name: 'Beta', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    const tab = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes(name))!
    tab('Alpha').click()
    flushSync()
    document.querySelector<HTMLButtonElement>('[data-menu-toggle]')!.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent === 'Server settings')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')).not.toBeNull()
    tab('Beta').click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')).toBeNull()
    expect(document.querySelector('[role=menu]')).toBeNull()
  })
})

describe('account menu', () => {
  it('esc_closes_the_account_menu_and_focus_returns_to_its_toggle', async () => {
    app.state = { ...emptyState() }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    const toggle = document.querySelector<HTMLButtonElement>('.who-btn')!
    toggle.click()
    flushSync()
    expect(document.querySelector('.me [role=menu]')).not.toBeNull()
    await new Promise((r) => setTimeout(r, 0)) // armed after the opening click
    const esc = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
    window.dispatchEvent(esc)
    flushSync()
    expect(document.querySelector('.me [role=menu]')).toBeNull()
    expect(esc.defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(toggle)
    // the toggle still toggles: a press on it isn't an outside press
    toggle.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    toggle.click()
    flushSync()
    expect(document.querySelector('.me [role=menu]')).not.toBeNull()
  })

  it('esc_closes_the_server_mute_menu', async () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    const tab = [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Alpha'))!
    tab.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    flushSync()
    expect(document.querySelector('.ctx[role=menu]')).not.toBeNull()
    await new Promise((r) => setTimeout(r, 0))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    flushSync()
    expect(document.querySelector('.ctx[role=menu]')).toBeNull()
  })
})

describe('watching from anywhere', () => {
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
  const q = (sel: string) => document.querySelector<HTMLElement>(sel)
  const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text || b.getAttribute('aria-label') === text)
  const settle = async () => {
    for (let i = 0; i < 5; i++) await tick()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
  }
  const emit = (name: string, payload: unknown) => handlers.get(name)!({ payload })
  const goTo = (id: string) => {
    callView.goTo = id
    flushSync()
  }

  beforeEach(async () => {
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
    stream.notice = ''
    stream.watchError = ''
    offs = [await startListening(), await startVoiceListening()]
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
  })

  it('stream_end_closes_full_view_and_mini_preview', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    expect(q('.full-view')).not.toBeNull()
    emit('pulse://event', { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U2', reason: 'stopped' } })
    await settle()
    expect(q('.full-view')).toBeNull()
    expect(q('.mini-preview')).toBeNull()
    expect(callView.full).toBeNull()
    expect(document.body.textContent).toContain('sam stopped streaming')

    // the same from the mini-preview
    emit('pulse://event', { t: 'StreamStarted', d: { stream: samStream } })
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    goTo('T1')
    await settle()
    expect(q('.mini-preview')).not.toBeNull()
    emit('pulse://event', { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U2', reason: 'stopped' } })
    await settle()
    expect(q('.mini-preview')).toBeNull()
    expect(callView.full).toBeNull()
    expect(document.querySelectorAll('video')).toHaveLength(0)
    expect(document.body.textContent).toContain('sam stopped streaming')
  })

  it('leaving_voice_drops_the_mini_preview_and_its_element', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    goTo('T1')
    await settle()
    expect(q('.mini-preview video')).not.toBeNull()
    emit('voice://event', { kind: 'state', channel_id: null, connection: 'disconnected', controls: flags })
    await settle()
    expect(q('.mini-preview')).toBeNull()
    expect(document.querySelectorAll('video').length).toBe(0)
    expect(stream.watching).toEqual([])
  })

  it('toast_watch_opens_the_call_view_in_full_view', async () => {
    // somewhere else: Home
    expect(q('.full-view')).toBeNull()
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Watch')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('stream_watch', { channelId: 'V1', streamer: 'U2' })
    expect(callView.full).toEqual(SAM)
    expect(q('.full-view .stage video')).not.toBeNull()
    expect(q('.mini-preview')).toBeNull()
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'high')
  })

  it('join_and_watch_joins_then_opens_the_stream', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    voice.stage = { stage: 'disconnected', attempt: null }
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Join & watch')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('join_voice', expect.objectContaining({ channelId: 'V1' }))
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)
    // not yet connected: still waiting
    emit('voice://event', { kind: 'state', channel_id: 'V1', connection: 'connecting', controls: flags })
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)

    emit('voice://event', { kind: 'state', channel_id: 'V1', connection: 'connected', controls: flags })
    await settle()
    expect(invoke).toHaveBeenCalledWith('stream_watch', { channelId: 'V1', streamer: 'U2' })
    expect(callView.joinThenWatch).toBeNull()
    expect(callView.full).toEqual(SAM)
    expect(q('.full-view .stage video')).not.toBeNull()
  })

  it('a_failed_join_drops_the_pending_watch', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'join_voice') throw 'no route to the voice server'
    })
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Join & watch')!.click()
    await settle()
    expect(voice.joinFailed).not.toBeNull()
    expect(callView.joinThenWatch).toBeNull()
    // joining later by hand doesn't start the old watch
    emit('voice://event', { kind: 'state', channel_id: 'V1', connection: 'connected', controls: flags })
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)
  })

  it('joining_somewhere_else_meanwhile_drops_the_pending_watch', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Join & watch')!.click()
    await settle()
    voice.joining = 'V2'
    await settle()
    expect(callView.joinThenWatch).toBeNull()
  })

  it('the_live_popover_watches_in_the_call_and_joins_first_from_outside', async () => {
    const anchor = document.createElement('span')
    document.body.append(anchor)
    const pop = mount(LivePopover, { target: document.body, props: { channelId: 'V1', userId: 'U2', anchor, onClose: () => {} } })
    flushSync()
    expect(q('[aria-label="Live stream"] .watch')!.textContent).toBe('Watch')
    unmount(pop)

    voice.channelId = null
    voice.connection = 'disconnected'
    const pop2 = mount(LivePopover, { target: document.body, props: { channelId: 'V1', userId: 'U2', anchor, onClose: () => {} } })
    flushSync()
    const watch = q('[aria-label="Live stream"] .watch')!
    expect(watch.textContent).toBe('Join & watch')
    watch.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('join_voice', expect.objectContaining({ channelId: 'V1' }))
    expect(callView.joinThenWatch).toEqual(SAM)
    unmount(pop2)
  })

  it('join_and_watch_twice_joins_once', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    joinAndWatch('V1', 'U2')
    joinAndWatch('V1', 'U2')
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'join_voice')).toHaveLength(1)
    expect(callView.joinThenWatch).toEqual(SAM)
  })
})
