// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
const room = vi.hoisted(() => ({ connected: true, disconnect: async () => {}, watch: async () => 0, unwatch: () => {}, setLayer: vi.fn(), remoteStats: async () => undefined }))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => room, pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import VoicePanel from './VoicePanel.svelte'
import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { voice, voiceApi } from '../lib/voice.svelte'
import { stream, streamApi } from '../lib/stream/stream.svelte'
import { callView } from '../lib/stream/viewer.svelte'
import type { Ready } from '../lib/protocol/Ready'

const me = { id: 'U1', username: 'alex', avatar_hash: null }
const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const flags = { muted: false, deafened: false }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [{ channel_id: 'V1', members: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }] }],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [],
}

let cmp: ReturnType<typeof mount> | undefined
const onMessage = vi.fn()
const onVoiceSettings = vi.fn()
const mountPanel = () => {
  cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1', onMessage, onVoiceSettings } })
  flushSync()
}
const q = (sel: string) => document.querySelector<HTMLElement>(sel)
const menu = () => q('[role=menu]')
const items = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')]
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  await new Promise((r) => setTimeout(r, 0))
  flushSync()
}
const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init })
  el.dispatchEvent(e)
  flushSync()
  return e
}
const rightClick = (el: Element) => {
  el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }))
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  localStorage.clear()
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Windows NT 10.0)')
  app.state = applyReady(emptyState(), ready)
  voice.channelId = 'V1'
  voice.joining = null
  voice.leaving = null
  voice.connection = 'connected'
  voice.stage = { stage: 'connected', attempt: null }
  voice.error = ''
  voice.volumes = {}
  voice.localMutes = {}
  voice.controls = { muted: false, deafened: false }
  stream.live = null
  stream.watching = []
  onMessage.mockReset()
  onVoiceSettings.mockReset()
})
afterEach(async () => {
  if (cmp) unmount(cmp)
  cmp = undefined
  callView.full = null
  await streamApi.unwatchAll()
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('ParticipantMenu', () => {
  it('right_click_on_a_tile_opens_the_menu', () => {
    mountPanel()
    expect(menu()).toBeNull()
    rightClick(q('[data-user="U2"]')!)
    expect(menu()).not.toBeNull()
    expect(menu()!.style.position || getComputedStyle(menu()!).position).toBe('fixed')
    expect(menu()!.querySelector('input[aria-label="User volume"]')).not.toBeNull()
    expect(document.activeElement).toBe(items()[0])
  })

  it('an_avatar_click_opens_the_menu_not_a_popover', () => {
    mountPanel()
    q('[data-user="U2"] .avatar-btn')!.click()
    flushSync()
    expect(menu()).not.toBeNull()
    expect(q('[role=dialog]')).toBeNull()
  })

  it('shift_f10_and_context_menu_key_open_it', () => {
    mountPanel()
    key(q('[data-user="U2"] .avatar-btn')!, 'F10', { shiftKey: true })
    expect(menu()).not.toBeNull()
    key(menu()!, 'Escape')
    expect(menu()).toBeNull()
    key(q('[data-user="U2"] .avatar-btn')!, 'ContextMenu')
    expect(menu()).not.toBeNull()
  })

  it('arrows_move_enter_picks_esc_closes_and_restores_focus', () => {
    mountPanel()
    const opener = q('[data-user="U2"] .avatar-btn')!
    opener.focus()
    key(opener, 'ContextMenu')
    const list = items()
    expect(list.map((i) => i.dataset.item)).toEqual(['volume', 'local-mute', 'message'])
    expect(document.activeElement).toBe(list[0])
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(list[1])
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(list[2])
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(list[0]) // wraps
    key(document.activeElement!, 'ArrowUp')
    expect(document.activeElement).toBe(list[2])
    key(document.activeElement!, 'Escape')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(opener)
  })

  it('left_and_right_adjust_a_focused_slider', () => {
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    const slider = q('[role=menu] input[aria-label="User volume"]') as HTMLInputElement
    slider.focus()
    key(slider, 'ArrowRight')
    expect(invoke).toHaveBeenCalledWith('set_peer_volume', { userId: 'U2', percent: 105 })
    key(slider, 'ArrowLeft')
    expect(voice.volumes.U2).toBe(100)
  })

  it('mute_for_me_persists_and_sends_zero_volume', async () => {
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    document.querySelector<HTMLElement>('[data-item="local-mute"]')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_peer_volume', { userId: 'U2', percent: 0 })
    expect(JSON.parse(localStorage.getItem('pulse.localmutes')!)).toHaveProperty('U2')
    // and back on: the remembered volume returns
    rightClick(q('[data-user="U2"]')!)
    expect(document.querySelector('[data-item="local-mute"]')!.textContent).toContain('Unmute')
    document.querySelector<HTMLElement>('[data-item="local-mute"]')!.click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_peer_volume', { userId: 'U2', percent: 100 })
    expect(JSON.parse(localStorage.getItem('pulse.localmutes')!)).not.toHaveProperty('U2')
  })

  it('message_calls_on_message', () => {
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    document.querySelector<HTMLElement>('[data-item="message"]')!.click()
    flushSync()
    expect(onMessage).toHaveBeenCalledWith('U2')
    expect(menu()).toBeNull()
  })

  it('your_own_menu_has_mute_deafen_and_voice_settings', () => {
    mountPanel()
    rightClick(q('[data-user="U1"]')!)
    expect(items().map((i) => i.dataset.item)).toEqual(['mute', 'deafen', 'go-live', 'voice-settings'])
    document.querySelector<HTMLElement>('[data-item="mute"]')!.click()
    expect(invoke).toHaveBeenCalledWith('toggle_mute')
    rightClick(q('[data-user="U1"]')!)
    document.querySelector<HTMLElement>('[data-item="voice-settings"]')!.click()
    expect(onVoiceSettings).toHaveBeenCalled()
  })

  it('a_click_outside_closes_it', async () => {
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    await new Promise((r) => setTimeout(r, 5))
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    flushSync()
    expect(menu()).toBeNull()
  })

  it('a_stream_tile_menu_offers_watch_for_a_live_person', async () => {
    app.state = applyReady(emptyState(), { ...ready, streams: [{ channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false }, status: 'live', viewers: [], started_at: '2026-10-26T10:00:00Z' }] })
    mountPanel()
    rightClick(q('[data-stream="U2"]')!)
    expect(items().map((i) => i.dataset.item)).toEqual(['volume', 'local-mute', 'message', 'watch'])
    document.querySelector<HTMLElement>('[data-item="watch"]')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('stream_watch', { channelId: 'V1', streamer: 'U2' })
  })

  it('channel_list_occupants_use_the_same_menu', () => {
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: 'V1', speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage, onVoiceSettings },
    })
    flushSync()
    const occ = [...document.querySelectorAll<HTMLElement>('.occupant')].find((o) => o.textContent?.includes('sam'))!
    expect(occ.tabIndex).toBe(0)
    rightClick(occ)
    expect(items().map((i) => i.dataset.item)).toEqual(['volume', 'local-mute', 'message'])
    key(menu()!, 'Escape')
    expect(document.activeElement).toBe(occ)
    key(occ, 'F10', { shiftKey: true })
    expect(menu()).not.toBeNull()
  })

  it('joining_re_applies_volumes_then_local_mutes', async () => {
    voice.volumes = { U2: 150 }
    voice.localMutes = { U2: true }
    await voiceApi.join('V1')
    const calls = invoke.mock.calls.filter(([c]) => c === 'set_peer_volume').map(([, a]) => a)
    expect(calls).toEqual([{ userId: 'U2', percent: 150 }, { userId: 'U2', percent: 0 }])
  })

  it('esc_closes_the_menu_without_leaving_full_view', async () => {
    app.state = applyReady(emptyState(), { ...ready, streams: [{ channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false }, status: 'live', viewers: [], started_at: '2026-10-26T10:00:00Z' }] })
    await streamApi.watch('V1', 'U2')
    callView.full = { channelId: 'V1', streamer: 'U2' }
    mountPanel()
    await settle()
    expect(q('.full-view')).not.toBeNull()
    rightClick(q('.strip [data-user="U1"]')!)
    expect(menu()).not.toBeNull()
    await new Promise((r) => setTimeout(r, 5))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(menu()).toBeNull()
    expect(q('.full-view')).not.toBeNull()
  })
})
