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
  onLost: () => {},
  disconnect: async () => {},
  watch: async () => 0,
  unwatch: () => {},
  setLayer: vi.fn(),
  remoteStats: async () => undefined,
}))
vi.mock('../components/GoLiveDialog.svelte', () => ({ default: () => {} }))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => room, pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import Shell from './Shell.svelte'
import LivePopover from '../components/LivePopover.svelte'
import { app, startListening } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { startVoiceListening, voice } from '../lib/voice.svelte'
import { stream, streamApi } from '../lib/stream/stream.svelte'
import { closeGoLive, goLiveUi } from '../lib/stream/golive.svelte'
import { callView, joinAndWatch, openFull, showToast, watchStream } from '../lib/stream/viewer.svelte'
import type { Ready } from '../lib/protocol/Ready'
import Waiting from './Waiting.svelte'

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
  it('holds animation still while live, and lets go when live ends or the shell goes', async () => {
    const { motion } = await import('../lib/animate.svelte')
    app.state = emptyState()
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    const live = { channelId: 'V1', res: 720, fps: 60, audio: false, audioMuted: false, label: 'Game', encoderWarning: false } as typeof stream.live
    stream.live = live
    flushSync()
    expect(motion.streaming).toBe(true)
    expect(document.body.classList.contains('streaming')).toBe(true)
    stream.live = null
    flushSync()
    expect(motion.streaming).toBe(false)
    stream.live = live
    flushSync()
    unmount(cmp)
    cmp = undefined
    expect(motion.streaming).toBe(false)
    expect(document.body.classList.contains('streaming')).toBe(false)
    stream.live = null
  })

  it('Esc closes the server menu and puts focus back on the server name', async () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Alpha'))!.click()
    flushSync()
    const toggle = document.querySelector<HTMLButtonElement>('.head-btn')!
    toggle.click()
    flushSync()
    await new Promise((r) => setTimeout(r, 0)) // clickOutside listens from the next tick
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')][0].focus()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('.head-menu')).toBeNull()
    expect(document.activeElement).toBe(toggle)
  })

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

  it('the top bar says why a refused connection is retrying', () => {
    app.state = { ...emptyState(), conn: 'too_many_devices' }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    expect(document.querySelector('.conn')!.textContent).toBe('Too many devices connected — close Pulse on another device')
    app.connAt = Date.now()
    app.state = { ...app.state, conn: { rate_limited: 4 } }
    flushSync()
    expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 4s…')
  })

  it('the retry countdown ticks down and is replaced when the next attempt starts', () => {
    vi.useFakeTimers()
    try {
      vi.setSystemTime(100_000)
      app.connAt = Date.now()
      app.state = { ...emptyState(), conn: { rate_limited: 3 } }
      cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 3s…')
      vi.advanceTimersByTime(1000)
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 2s…')
      vi.advanceTimersByTime(2000)
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Connecting…')
    } finally {
      vi.useRealTimers()
    }
  })

  it('a refused startup shows the message on the waiting screen', () => {
    app.connAt = Date.now()
    app.state = { ...emptyState(), conn: 'too_many_devices' }
    cmp = mount(Waiting, { target: document.body, props: { onSignOut: () => {} } })
    flushSync()
    expect(document.querySelector('[role=status]')!.textContent).toContain('Too many devices connected')
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
    people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [samStream], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
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

  const toggle = (name: 'Channels' | 'Members') => document.querySelector<HTMLButtonElement>(`.edge[aria-label="${name}"]`)

  it('full_view_hides_both_side_panels_with_edge_toggles_to_bring_each_back', async () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem')
    const side = () => q('aside.side')!
    const members = () => q('.right-panel')!
    expect(side().hidden).toBe(false)
    expect(members().hidden).toBe(false)
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    expect(q('.full-view')).not.toBeNull()
    expect(side().hidden).toBe(true)
    expect(members().hidden).toBe(true)
    const left = toggle('Channels')!
    const right = toggle('Members')!
    // what each toggles, in the order they sit: the left one first in main, the right one last
    expect(left.getAttribute('aria-controls')).toBe(side().id)
    expect(right.getAttribute('aria-controls')).toBe(members().id)
    const main = q('main')!
    expect(main.firstElementChild).toBe(left)
    expect(main.lastElementChild).toBe(right)
    expect(left.getAttribute('aria-expanded')).toBe('false')
    left.click()
    flushSync()
    expect(side().hidden).toBe(false)
    expect(members().hidden).toBe(true) // each on its own
    expect(left.getAttribute('aria-expanded')).toBe('true')
    right.click()
    flushSync()
    expect(members().hidden).toBe(false)
    right.click()
    flushSync()
    expect(members().hidden).toBe(true)

    // Esc leaves full view: everything is back, the toggles go
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    flushSync()
    expect(q('.full-view')).toBeNull()
    expect(side().hidden).toBe(false)
    expect(members().hidden).toBe(false)
    expect(toggle('Channels')).toBeNull()

    // next time: hidden again by default (nothing remembered)
    openFull(SAM)
    await settle()
    expect(side().hidden).toBe(true)
    expect(members().hidden).toBe(true)
    expect(setItem.mock.calls.filter(([k]) => !/^pulse\.(stream|sounds|unsent)/.test(String(k)))).toEqual([])
  })

  it('swapping_the_stream_in_full_view_keeps_the_panels_as_you_set_them', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    toggle('Channels')!.click()
    flushSync()
    expect(q('aside.side')!.hidden).toBe(false)
    // another stream in the same call takes full view from its toast's Watch (the watch cap ends sam's)
    watchStream('V1', 'U3')
    await settle()
    expect(callView.full).toEqual({ channelId: 'V1', streamer: 'U3' })
    expect(q('.full-view')).not.toBeNull()
    expect(q('aside.side')!.hidden).toBe(false)
  })

  it('switching_channel_in_full_view_brings_the_panels_back', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    goTo('T1')
    await settle()
    expect(q('aside.side')!.hidden).toBe(false)
    expect(q('.right-panel')!.hidden).toBe(false)
    expect(toggle('Channels')).toBeNull()
  })

  it('a_menu_left_open_in_a_panel_closes_when_full_view_hides_it_so_esc_still_leaves', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    q('.who-btn')!.click() // the account menu, open
    flushSync()
    expect(q('.menu[role="menu"]')).not.toBeNull()
    openFull(SAM)
    await settle()
    expect(q('.menu[role="menu"]')).toBeNull()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    flushSync()
    expect(q('.full-view')).toBeNull()
  })

  it('focus_inside_a_panel_that_hides_moves_to_its_toggle', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    q('aside.side button')!.focus()
    expect(q('aside.side')!.contains(document.activeElement)).toBe(true)
    openFull(SAM)
    await settle()
    expect(document.activeElement).toBe(toggle('Channels'))
  })

  it('full_view_away_from_the_call_view_hides_nothing', async () => {
    // full view kept for the mini-preview while you read a text channel: the panels stay
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    goTo('T1')
    await settle()
    expect(q('.mini-preview')).not.toBeNull()
    expect(q('aside.side')!.hidden).toBe(false)
    expect(q('.right-panel')!.hidden).toBe(false)
    expect(toggle('Channels')).toBeNull()
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

describe('Go Live cleanup', () => {
  const user = { id: 'U1', username: 'alex', avatar_hash: null }
  const mountShell = () => {
    cmp = mount(Shell, { target: document.body, props: { user, onLogout: () => {} } })
    flushSync()
  }
  afterEach(() => {
    closeGoLive()
    voice.channelId = null
    voice.connection = 'disconnected'
  })

  it('losing_the_connection_drops_the_picked_capture', async () => {
    voice.channelId = 'V1'
    voice.connection = 'connected'
    const stop = vi.fn()
    goLiveUi.picked = { track: { stop } } as never
    goLiveUi.channelId = 'V1'
    mountShell()
    expect(goLiveUi.picked).not.toBeNull()
    expect(stop).not.toHaveBeenCalled()
    voice.connection = 'reconnecting'
    await tick()
    flushSync()
    expect(stop).toHaveBeenCalledTimes(1)
    expect(goLiveUi.picked).toBeNull()
  })

  it('a_picker_error_does_not_outlive_the_call', async () => {
    voice.channelId = 'V1'
    voice.connection = 'connected'
    goLiveUi.error = 'no screen'
    goLiveUi.channelId = 'V1'
    mountShell()
    expect(goLiveUi.error).toBe('no screen')
    voice.channelId = null
    voice.connection = 'disconnected'
    await tick()
    flushSync()
    expect(goLiveUi.error).toBe('')
    expect(goLiveUi.channelId).toBeNull()
  })
})
