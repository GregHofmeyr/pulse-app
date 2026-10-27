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
import { findWatch, stream, streamApi } from '../lib/stream/stream.svelte'
import { goLiveUi } from '../lib/stream/golive.svelte'
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
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
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

  it('arrows_move_and_wrap_esc_closes_and_restores_focus', () => {
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

  it('volumes_then_local_mutes_are_applied_before_the_join_without_a_blip', async () => {
    voice.volumes = { U2: 150, U3: 120 }
    voice.localMutes = { U2: true }
    voice.channelId = null
    await voiceApi.join('V1')
    const calls = invoke.mock.calls.filter(([c]) => c === 'set_peer_volume' || c === 'join_voice').map(([c, a]) => (c === 'join_voice' ? 'join' : a))
    // the core keeps them without a session: set before the join, nobody muted for you is heard on arrival,
    // and a locally muted person never gets their remembered volume first (no audible blip)
    expect(calls).toEqual([{ userId: 'U3', percent: 120 }, { userId: 'U2', percent: 0 }, 'join'])
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

  const live = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false }, status: 'live' as const, viewers: [], started_at: '2026-10-26T10:00:00Z' }
  const watchSam = async () => {
    app.state = applyReady(emptyState(), { ...ready, streams: [live] })
    await streamApi.watch('V1', 'U2')
    mountPanel()
    await settle()
  }
  const item = (id: string) => document.querySelector<HTMLElement>(`[data-item="${id}"]`)!

  it('enter_picks_the_focused_item_once', () => {
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    item('message').focus()
    key(item('message'), 'Enter')
    expect(onMessage).toHaveBeenCalledTimes(1)
    expect(onMessage).toHaveBeenCalledWith('U2')
    expect(menu()).toBeNull()
  })

  it('tab_closes_the_menu_and_returns_focus_to_the_opener', () => {
    mountPanel()
    const opener = q('[data-user="U2"] .avatar-btn')!
    opener.focus()
    key(opener, 'ContextMenu')
    expect(menu()).not.toBeNull()
    key(document.activeElement!, 'Tab')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(opener)
  })

  it('moving_the_user_slider_clears_a_local_mute', async () => {
    voice.localMutes = { U2: true }
    localStorage.setItem('pulse.localmutes', JSON.stringify({ U2: true }))
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    expect(item('local-mute').textContent).toContain('Unmute')
    const slider = q('[role=menu] input[aria-label="User volume"]') as HTMLInputElement
    slider.focus()
    key(slider, 'ArrowRight')
    await settle()
    expect(voice.localMutes).toEqual({})
    expect(JSON.parse(localStorage.getItem('pulse.localmutes')!)).toEqual({})
    expect(invoke.mock.calls.filter(([c]) => c === 'set_peer_volume').at(-1)![1]).toEqual({ userId: 'U2', percent: 105 })
    expect(item('local-mute').textContent).toContain('Mute for me')
  })

  it('a_muted_for_you_hint_shows_beside_the_user_volume', () => {
    voice.localMutes = { U2: true }
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    expect(q('[role=menu] .slider .hint')?.textContent).toBe('Muted for you')
    voice.localMutes = {}
    flushSync()
    expect(q('[role=menu] .slider .hint')).toBeNull()
  })

  it('on_linux_watch_and_go_live_are_disabled_with_their_reason', () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (X11; Linux x86_64)')
    app.state = applyReady(emptyState(), { ...ready, streams: [live] })
    mountPanel()
    rightClick(q('[data-stream="U2"]')!)
    expect(item('watch').getAttribute('aria-disabled')).toBe('true')
    expect(item('watch').textContent).toContain("Watching from Linux isn't supported yet")
    item('watch').click()
    flushSync()
    expect(invoke).not.toHaveBeenCalledWith('stream_watch', expect.anything())
    key(menu()!, 'Escape')
    rightClick(q('[data-user="U1"]')!)
    expect(item('go-live').getAttribute('aria-disabled')).toBe('true')
    expect(item('go-live').textContent).toContain("Streaming from Linux isn't supported yet")
    invoke.mockClear()
    item('go-live').click()
    flushSync()
    expect(invoke).not.toHaveBeenCalled()
    expect(goLiveUi.picked).toBeNull()
  })

  it('the_stream_volume_slider_uses_the_shared_stream_volumes', async () => {
    await watchSam()
    rightClick(q('[data-stream="U2"]')!)
    expect(items().map((i) => i.dataset.item)).toEqual(['volume', 'local-mute', 'message', 'stream-volume', 'stop-watching', 'full-view'])
    const slider = q('[role=menu] input[aria-label="Stream volume"]') as HTMLInputElement
    slider.focus()
    key(slider, 'ArrowRight')
    expect(stream.volumes.U2).toBe(105)
    expect(slider.closest('.slider')!.textContent).toContain('105%')
  })

  it('stop_watching_and_full_view_items_work', async () => {
    await watchSam()
    rightClick(q('[data-stream="U2"]')!)
    item('full-view').click()
    await settle()
    expect(callView.full).toEqual({ channelId: 'V1', streamer: 'U2' })
    expect(callView.goTo).toBe('V1')
    callView.goTo = null
    rightClick(q('.strip [data-user="U2"]')!)
    expect(items().map((i) => i.dataset.item)).not.toContain('full-view')
    item('stop-watching').click()
    await settle()
    expect(findWatch('V1', 'U2')).toBeUndefined()
  })

  it('stop_streaming_stops_your_stream', () => {
    stream.live = { channelId: 'V1', res: 720, fps: 30, audio: false, audioMuted: false, label: 'x', encoderWarning: false } as typeof stream.live
    const stop = vi.spyOn(streamApi, 'stop').mockResolvedValue()
    mountPanel()
    rightClick(q('[data-user="U1"]')!)
    expect(items().map((i) => i.dataset.item)).toEqual(['mute', 'deafen', 'stop-streaming', 'voice-settings'])
    item('stop-streaming').click()
    expect(stop).toHaveBeenCalledTimes(1)
    stream.live = null
  })

  it('clicking_the_opener_again_closes_its_menu', async () => {
    mountPanel()
    const btn = q('[data-user="U2"] .avatar-btn')!
    btn.click()
    flushSync()
    expect(menu()).not.toBeNull()
    await new Promise((r) => setTimeout(r, 5))
    btn.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    btn.click()
    flushSync()
    expect(menu()).toBeNull()
  })

  it('clicking_a_full_view_strip_opener_again_closes_its_menu', async () => {
    app.state = applyReady(emptyState(), { ...ready, streams: [{ channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false }, status: 'live', viewers: [], started_at: '2026-10-26T10:00:00Z' }] })
    await streamApi.watch('V1', 'U2')
    callView.full = { channelId: 'V1', streamer: 'U2' }
    mountPanel()
    await settle()
    const btn = q('.full-view .strip [data-user="U2"] .avatar-btn')!
    btn.click()
    flushSync()
    expect(menu()).not.toBeNull()
    await new Promise((r) => setTimeout(r, 5))
    btn.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    btn.click()
    flushSync()
    expect(menu()).toBeNull()
    expect(q('.full-view')).not.toBeNull()
  })

  it('clicking_a_channel_list_row_again_closes_its_menu', async () => {
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: 'V1', speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage, onVoiceSettings },
    })
    flushSync()
    const occ = [...document.querySelectorAll<HTMLElement>('.occupant')].find((o) => o.textContent?.includes('sam'))!
    occ.click()
    flushSync()
    expect(menu()).not.toBeNull()
    await new Promise((r) => setTimeout(r, 5))
    occ.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    occ.click()
    flushSync()
    expect(menu()).toBeNull()
  })

  it('the_menu_is_clamped_into_the_viewport', () => {
    mountPanel()
    q('[data-user="U2"]')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 99999, clientY: 99999 }))
    flushSync()
    expect(parseFloat(menu()!.style.left)).toBeLessThanOrEqual(window.innerWidth - 8)
    expect(parseFloat(menu()!.style.top)).toBeLessThanOrEqual(window.innerHeight - 8)
  })
})
