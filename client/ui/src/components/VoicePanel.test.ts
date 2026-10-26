// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
const room = vi.hoisted(() => ({
  connected: true,
  disconnect: async () => {},
  watch: async () => 0,
  unwatch: () => {},
  setLayer: vi.fn(),
  remoteStats: async () => undefined,
}))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => room, pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import VoicePanel from './VoicePanel.svelte'
import UserTile from './UserTile.svelte'
import StreamTileForTest from './StreamTile.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { voice } from '../lib/voice.svelte'
import { stream, streamApi } from '../lib/stream/stream.svelte'
import type { Ready } from '../lib/protocol/Ready'

const me = { id: 'U1', username: 'alex', avatar_hash: null }
const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const flags = { muted: false, deafened: false }
const samStream = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false }, status: 'live' as const, viewers: [], started_at: '2026-10-26T10:00:00Z' }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [{ channel_id: 'V1', members: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }] }],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [samStream], custom_emoji: [], frequent_emoji: [],
}

let cmp: ReturnType<typeof mount> | undefined
const mountPanel = () => {
  cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1' } })
  flushSync()
}
const q = (sel: string) => document.querySelector<HTMLElement>(sel)
const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text || b.getAttribute('aria-label') === text)
const WATCH = "Watch sam's stream"
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  await new Promise((r) => setTimeout(r, 0))
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  localStorage.clear()
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Windows NT 10.0)')
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  app.state = applyReady(emptyState(), ready)
  voice.channelId = 'V1'
  voice.joining = null
  voice.leaving = null
  voice.connection = 'connected'
  voice.stage = { stage: 'connected', attempt: null }
  voice.error = ''
  stream.live = null
  stream.watching = []
  room.setLayer.mockClear()
})
afterEach(async () => {
  if (cmp) unmount(cmp)
  cmp = undefined
  await streamApi.unwatchAll()
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('stream tiles', () => {
  it('not_watching_tile_shows_live_watch_and_viewer_count', () => {
    mountPanel()
    const tile = q('[data-stream="U2"]')!
    expect(tile.textContent).toContain('LIVE')
    expect(button(WATCH)).toBeDefined()
    expect(tile.querySelector('.viewers')).toBeNull()
    unmount(cmp!)
    app.state = applyEvent(app.state, { t: 'ViewersChanged', d: { channel_id: 'V1', user_id: 'U2', viewers: ['U1', 'U3'] } }, 0)
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1' } })
    flushSync()
    expect(q('[data-stream="U2"] .viewers')?.textContent?.trim()).toBe('2')
  })

  it('one_click_watches_the_stream', async () => {
    mountPanel()
    q('[data-stream="U2"]')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('stream_watch', { channelId: 'V1', streamer: 'U2' })
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(1)
  })

  it('the_watch_button_watches_once', async () => {
    mountPanel()
    button(WATCH)!.click()
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(1)
  })

  it('watching_tile_has_volume_quality_stop_and_full_view', async () => {
    mountPanel()
    await streamApi.watch('V1', 'U2')
    await settle()
    const tile = q('[data-stream="U2"]')!
    expect(tile.querySelector('video')).not.toBeNull()
    expect(tile.querySelector('input[aria-label="Stream volume"]')).not.toBeNull()
    expect(tile.querySelector('[aria-label="Quality"]')).not.toBeNull()
    expect(button('Stop watching')).toBeDefined()
    expect(button('Full view')).toBeDefined()
    expect(tile.textContent).not.toContain('LIVE')
  })

  it('stop_watching_returns_the_tile_to_not_watching', async () => {
    mountPanel()
    await streamApi.watch('V1', 'U2')
    await settle()
    button('Stop watching')!.click()
    await settle()
    expect(stream.watching).toEqual([])
    expect(q('[data-stream="U2"]')!.textContent).toContain('LIVE')
  })

  it('on_linux_the_reason_replaces_watch', () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (X11; Linux x86_64)')
    mountPanel()
    const tile = q('[data-stream="U2"]')!
    expect(tile.textContent).toContain("Watching from Linux isn't supported yet")
    tile.click()
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)
  })

  it('in_a_channel_you_are_not_in_the_button_says_join_and_watch', () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    mountPanel()
    const tile = q('[data-stream="U2"]')!
    expect(tile.textContent).toContain('Join & watch')
    tile.click()
    button("Join and watch sam's stream")!.click()
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)
    expect(stream.watching).toEqual([])
  })

  it('own_stream_tile_shows_the_local_preview_and_hide_toggle', () => {
    app.state = applyEvent(app.state, { t: 'StreamStarted', d: { stream: { ...samStream, user_id: 'U1' } } }, 0)
    stream.live = { channelId: 'V1', res: 720, fps: 60, audio: false, audioMuted: false, label: 'Game', encoderWarning: false }
    mountPanel()
    const tile = q('[data-stream="U1"]')!
    expect(tile.querySelector('video')).not.toBeNull()
    expect(button('Hide preview')).toBeDefined()
    expect(tile.textContent).not.toContain('Watch')
    expect(tile.classList.contains('own')).toBe(true)
    expect(tile.querySelector('.card.tile')).not.toBeNull()
  })

  it('own_tile_shows_from_the_local_live_state_before_the_server_lists_it', () => {
    stream.live = { channelId: 'V1', res: 720, fps: 60, audio: false, audioMuted: false, label: 'Game', encoderWarning: false }
    mountPanel()
    expect(q('[data-stream="U1"] .card.tile')).not.toBeNull()
  })

  it('a_grid_watch_asks_for_the_low_layer', async () => {
    mountPanel()
    q('[data-stream="U2"]')!.click()
    await settle()
    expect(room.setLayer).toHaveBeenCalledWith('U2', 'low')
  })

  it('the_quality_list_opens_upward_on_the_tile', async () => {
    mountPanel()
    await streamApi.watch('V1', 'U2')
    await settle()
    expect(q('[data-stream="U2"] .select')?.getAttribute('data-drop')).toBe('up')
  })

  it('the_preview_is_a_keyboard_target_for_full_view', async () => {
    const onFull = vi.fn()
    cmp = mount(StreamTileForTest, { target: document.body, props: { channelId: 'V1', serverId: 'S1', streamer: 'U2', mine: false, onMenu: () => {}, onFull } })
    await streamApi.watch('V1', 'U2')
    await settle()
    const host = q('.host')!
    expect(host.getAttribute('role')).toBe('button')
    host.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(onFull).toHaveBeenCalledWith({ channelId: 'V1', streamer: 'U2' })
  })
})

describe('everyone toggle', () => {
  const toggle = () => document.querySelector<HTMLButtonElement>('button[aria-label="Show non-streaming participants"]')!

  it('toggle_hides_user_tiles_and_is_remembered_per_user', () => {
    mountPanel()
    expect(toggle().getAttribute('aria-pressed')).toBe('true')
    expect(document.querySelectorAll('[data-user]').length).toBe(2)
    toggle().click()
    flushSync()
    expect(toggle().getAttribute('aria-pressed')).toBe('false')
    expect(document.querySelectorAll('[data-user]').length).toBe(0)
    expect(q('[data-stream="U2"]')).not.toBeNull()
    expect(localStorage.getItem('pulse.callgrid.everyone.U1')).toContain('false')
    unmount(cmp!)
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1' } })
    flushSync()
    expect(toggle().getAttribute('aria-pressed')).toBe('false')
  })

  it('the_choice_follows_the_signed_in_user_once_known', () => {
    localStorage.setItem('pulse.callgrid.everyone.U1', JSON.stringify({ on: false }))
    app.state = { ...app.state, me: null }
    mountPanel()
    expect(toggle().getAttribute('aria-pressed')).toBe('true')
    app.state = { ...app.state, me }
    flushSync()
    expect(toggle().getAttribute('aria-pressed')).toBe('false')
    toggle().click()
    flushSync()
    expect(localStorage.getItem('pulse.callgrid.everyone.U1')).toContain('true')
  })

  it('empty_state_button_shows_everyone_again', () => {
    app.state = applyEvent(app.state, { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U2', reason: 'stopped' } }, 0)
    localStorage.setItem('pulse.callgrid.everyone.U1', JSON.stringify({ on: false }))
    mountPanel()
    expect(document.body.textContent).toContain("Nobody's streaming — show everyone")
    expect(document.querySelectorAll('[data-user]').length).toBe(0)
    button('show everyone')!.click()
    flushSync()
    expect(document.querySelectorAll('[data-user]').length).toBe(2)
    expect(document.body.textContent).not.toContain("Nobody's streaming")
  })
})

describe('user tiles', () => {
  it('a_joining_entry_is_greyed_with_a_spinner', () => {
    voice.joining = 'V1'
    app.state = { ...app.state, voice: { ...app.state.voice, V1: [{ user_id: 'U2', flags }] } }
    mountPanel()
    const mineTile = q('[data-user="U1"]')!
    expect(mineTile.classList.contains('pending')).toBe(true)
    expect(mineTile.textContent).toContain('Joining…')
    expect(mineTile.querySelector('[role=progressbar]')).not.toBeNull()
  })

  it('the_avatar_still_opens_the_volume_control', () => {
    mountPanel()
    q('[data-user="U2"] .avatar-btn')!.click()
    flushSync()
    expect(q('[data-user="U2"] input[type=range]')).not.toBeNull()
  })

  it('right_click_asks_for_the_menu', () => {
    // the menu itself arrives with the participant menu; the tile only reports the request
    mountPanel()
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    q('[data-user="U2"]')!.dispatchEvent(ev)
    expect(ev.defaultPrevented).toBe(true)
  })

  it('only_one_volume_popover_is_open_at_a_time', () => {
    app.state = { ...app.state, voice: { V1: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }, { user_id: 'U3', flags }] } }
    app.state = { ...app.state, members: { S1: [{ user: me, nickname: null }, { user: sam, nickname: null }, { user: { id: 'U3', username: 'kim', avatar_hash: null }, nickname: null }] } }
    mountPanel()
    q('[data-user="U2"] .avatar-btn')!.click()
    flushSync()
    q('[data-user="U3"] .avatar-btn')!.click()
    flushSync()
    expect(document.querySelectorAll('[role=dialog]').length).toBe(1)
    expect(q('[data-user="U3"] [role=dialog]')).not.toBeNull()
  })

  it('the_menu_key_works_on_your_own_tile', () => {
    const onMenu = vi.fn()
    cmp = mount(UserTile, { target: document.body, props: { entry: { userId: 'U1', name: 'alex', muted: false, deafened: false, pending: false }, channelId: 'V1', onMenu } })
    flushSync()
    const btn = q('.avatar-btn')!
    expect(btn.getAttribute('aria-disabled')).toBe('true')
    expect((btn as HTMLButtonElement).disabled).toBe(false)
    btn.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }))
    expect(onMenu).toHaveBeenCalledTimes(1)
  })
})
