// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
const room = vi.hoisted(() => ({ connected: true, onLost: () => {}, disconnect: async () => {}, watch: async () => 0, unwatch: () => {}, setLayer: vi.fn(), remoteStats: async () => undefined }))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => room, pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import VoicePanel from './VoicePanel.svelte'
import ChannelList from './ChannelList.svelte'
import { app, ui } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { voice, voiceApi } from '../lib/voice.svelte'
import { findWatch, stream, streamApi } from '../lib/stream/stream.svelte'
import { goLiveUi } from '../lib/stream/golive.svelte'
import { callView } from '../lib/stream/viewer.svelte'
import type { Ready } from '../lib/protocol/Ready'
import { closeCard, profileUi } from '../lib/profileCard.svelte'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const flags = { muted: false, deafened: false }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [{ channel_id: 'V1', members: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }] }],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
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
  it('View profile opens the card', () => {
    closeCard()
    mountPanel()
    const opener = q('[data-user="U2"] .avatar-btn')!
    opener.focus()
    key(opener, 'ContextMenu')
    const first = items()[0]
    expect(first.dataset.item).toBe('profile')
    expect(first.textContent).toBe('View profile')
    first.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
    expect(menu()).toBeNull()
    closeCard()
  })

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
    expect(list.map((i) => i.dataset.item)).toEqual(['profile', 'volume', 'local-mute', 'sound-mute', 'message', 'server-mute', 'server-deafen', 'disconnect', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
    expect(document.activeElement).toBe(list[0])
    for (let i = 1; i < list.length; i++) {
      key(document.activeElement!, 'ArrowDown')
      expect(document.activeElement).toBe(list[i])
    }
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(list[0]) // wraps
    key(document.activeElement!, 'ArrowUp')
    expect(document.activeElement).toBe(list[list.length - 1])
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
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'mute', 'deafen', 'go-live', 'voice-settings', 'nick'])
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
    app.state = applyReady(emptyState(), { ...ready, streams: [{ channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false, allow_ghosts: true }, status: 'live', viewers: [], ghosts: 0, started_at: '2026-10-26T10:00:00Z' }] })
    mountPanel()
    rightClick(q('[data-stream="U2"]')!)
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'volume', 'local-mute', 'sound-mute', 'message', 'watch', 'server-mute', 'server-deafen', 'disconnect', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
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
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'volume', 'local-mute', 'sound-mute', 'message', 'server-mute', 'server-deafen', 'disconnect', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
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
    app.state = applyReady(emptyState(), { ...ready, streams: [{ channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false, allow_ghosts: true }, status: 'live', viewers: [], ghosts: 0, started_at: '2026-10-26T10:00:00Z' }] })
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

  const live = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false, allow_ghosts: true }, status: 'live' as const, viewers: [], ghosts: 0, started_at: '2026-10-26T10:00:00Z' }
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
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'volume', 'local-mute', 'sound-mute', 'message', 'stream-volume', 'stop-watching', 'full-view', 'server-mute', 'server-deafen', 'disconnect', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
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
    stream.live = { channelId: 'V1', res: 720, fps: 30, audio: false, audioMuted: false, label: 'x', encoderWarning: false, allowGhosts: true } as typeof stream.live
    const stop = vi.spyOn(streamApi, 'stop').mockResolvedValue()
    mountPanel()
    rightClick(q('[data-user="U1"]')!)
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'mute', 'deafen', 'stop-streaming', 'voice-settings', 'nick'])
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
    app.state = applyReady(emptyState(), { ...ready, streams: [{ channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false, allow_ghosts: true }, status: 'live', viewers: [], ghosts: 0, started_at: '2026-10-26T10:00:00Z' }] })
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

describe('server mute, deafen and disconnect (#140)', () => {
  const held = (muted_by: string | null, deafened_by: string | null) => {
    app.state = { ...app.state, serverVoice: { S1: { U2: { server_id: 'S1', user_id: 'U2', muted_by, deafened_by } } } }
    flushSync()
  }
  const item = (id: string) => q(`[role=menu] [data-item="${id}"]`)!
  const openSam = () => {
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
  }

  it('server_mute_and_deafen_ask_the_server', () => {
    openSam()
    expect(item('server-mute').textContent).toBe('Server mute')
    expect(item('server-deafen').textContent).toBe('Server deafen')
    item('server-mute').click()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('server_voice', { serverId: 'S1', userId: 'U2', mute: true, deafen: null })
    expect(menu()).toBeNull()
    rightClick(q('[data-user="U2"]')!)
    item('server-deafen').click()
    expect(invoke).toHaveBeenCalledWith('server_voice', { serverId: 'S1', userId: 'U2', mute: null, deafen: true })
  })

  it('anyone_can_undo_them', () => {
    held('U3', 'U3')
    openSam()
    expect(item('server-mute').textContent).toBe('Server unmute')
    expect(item('server-deafen').textContent).toBe('Server undeafen')
    item('server-mute').click()
    expect(invoke).toHaveBeenCalledWith('server_voice', { serverId: 'S1', userId: 'U2', mute: false, deafen: null })
  })

  it('disconnect_takes_them_out', () => {
    openSam()
    item('disconnect').click()
    expect(invoke).toHaveBeenCalledWith('disconnect_voice', { serverId: 'S1', userId: 'U2' })
  })

  it('a_refusal_is_a_toast_not_the_call_box_line (#147)', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'server_voice' || cmd === 'disconnect_voice') throw "they aren't in this server's voice"
    })
    openSam()
    item('server-mute').click()
    await settle()
    expect(ui.notice).toContain("they aren't in this server's voice")
    expect(voice.error).toBe('')
    ui.notice = null
    rightClick(q('[data-user="U2"]')!)
    item('disconnect').click()
    await settle()
    expect(ui.notice).toContain("they aren't in this server's voice")
  })

  it('a_held_mute_or_deafen_says_who_did_it (#144)', () => {
    held('U1', 'U1')
    openSam()
    expect(item('server-mute').getAttribute('data-tip')).toBe('Server muted by alex')
    expect(item('server-deafen').getAttribute('data-tip')).toBe('Server deafened by alex')
  })

  it('tiles_and_rows_show_who_did_it', () => {
    held('U1', null)
    mountPanel()
    expect(q('[data-user="U2"] .badge.server')?.getAttribute('data-tip')).toBe('Server muted by alex')
    unmount(cmp!)
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: 'V1', speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage, onVoiceSettings },
    })
    flushSync()
    const occ = q('.occupant[data-user="U2"]')!
    expect(occ.querySelector('.flag.server')?.getAttribute('data-tip')).toBe('Server muted by alex')
    expect(occ.getAttribute('aria-label')).toContain('server muted by alex')
    held(null, 'U1')
    expect(occ.querySelector('.flag.server')?.getAttribute('data-tip')).toBe('Server deafened by alex')
  })

  it('your_own_buttons_say_you_are_server_muted', () => {
    app.state = { ...app.state, serverVoice: { S1: { U1: { server_id: 'S1', user_id: 'U1', muted_by: 'U2', deafened_by: null } } } }
    voice.controls = { muted: false, deafened: false, server_muted: true, server_deafened: false }
    mountPanel()
    const mute = q('.controls .ctl')!
    expect(mute.getAttribute('data-tip')).toBe('Server muted by sam')
    expect(mute.classList.contains('server')).toBe(true)
  })
})

describe('ParticipantMenu nicknames (#113, #137)', () => {
  const labels = () => Object.fromEntries(items().map((i) => [i.dataset.item, i.textContent]))
  const openFor = (userId: string) => {
    mountPanel()
    rightClick(q(`[data-user="${userId}"]`)!)
  }
  afterEach(() => {
    profileUi.rename = null
  })

  it('in a server: Change server nickname, then your private one, after the voice items; no reset', () => {
    app.state = {
      ...app.state,
      members: { S1: [{ user: me, nickname: null }, { user: sam, nickname: 'Sammy' }] },
      privateNicks: { U2: 'Bestie' },
    }
    openFor('U2')
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'volume', 'local-mute', 'sound-mute', 'message', 'server-mute', 'server-deafen', 'disconnect', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
    expect(labels()).toMatchObject({ nick: 'Change server nickname', 'private-nick': 'Change private nickname' })
    q('[data-item="private-nick"]')!.click()
    flushSync()
    expect(profileUi.rename).toEqual({ kind: 'private', userId: 'U2' })
  })

  it('Change server nickname opens the dialog for the server nickname, and the menu closes', () => {
    openFor('U2')
    q('[data-item="nick"]')!.click()
    flushSync()
    expect(profileUi.rename).toEqual({ kind: 'server', userId: 'U2', serverId: 'S1' })
    expect(menu()).toBeNull()
  })

  it('your own menu: your server nickname', () => {
    openFor('U1')
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'mute', 'deafen', 'go-live', 'voice-settings', 'nick'])
    q('[data-item="nick"]')!.click()
    flushSync()
    expect(profileUi.rename).toEqual({ kind: 'server', userId: 'U1', serverId: 'S1' })
  })

  it('a group call (no server): Change nickname, your private one', () => {
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: null, onMessage, onVoiceSettings } })
    flushSync()
    rightClick(q('[data-user="U2"]')!)
    expect(items().map((i) => i.dataset.item)).toEqual(['profile', 'volume', 'local-mute', 'sound-mute', 'message', 'private-nick', 'live-notify', 'appear-offline'])
    expect(labels()).toMatchObject({ 'private-nick': 'Change nickname' })
    q('[data-item="private-nick"]')!.click()
    flushSync()
    expect(profileUi.rename).toEqual({ kind: 'private', userId: 'U2' })
  })

  it('the arrows reach the nickname item and wrap; Enter picks it', () => {
    mountPanel()
    const opener = q('[data-user="U2"] .avatar-btn')!
    opener.focus()
    key(opener, 'ContextMenu')
    const list = items()
    key(document.activeElement!, 'End')
    expect(document.activeElement).toBe(list[list.length - 1])
    expect((document.activeElement as HTMLElement).dataset.item).toBe('appear-offline')
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(list[0])
    // up past appear offline, live notify (#60) and the private nickname
    key(document.activeElement!, 'ArrowUp')
    key(document.activeElement!, 'ArrowUp')
    key(document.activeElement!, 'ArrowUp')
    key(document.activeElement!, 'ArrowUp')
    key(document.activeElement!, 'Enter')
    expect(profileUi.rename).toEqual({ kind: 'server', userId: 'U2', serverId: 'S1' })
    expect(menu()).toBeNull()
  })
})

// #121: Mute join sound, per person, on this device, in the voice row's (and call tile's) menu
describe('join sound mute', () => {
  const withJoinSound = (who: string[]) => {
    const sound = { id: 'A', server_id: 'S1', name: 'airhorn', emoji: null, emoji_id: null, media_hash: 'hA', volume: 100, duration_ms: 6000, added_by: null, created_at: '2026-11-10T00:00:00.000Z', plays: 0 }
    app.state = { ...app.state, sounds: { A: sound }, members: { S1: app.state.members.S1.map((m) => (who.includes(m.user.id) ? { ...m, join_sound: 'A' } : m)) } }
  }
  const item = () => q('[data-item="join-sound-mute"]')

  it('someone with a join sound: the item sits under Mute soundboard and toggles the per-device mute', async () => {
    withJoinSound(['U2'])
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    const ids = items().map((i) => i.dataset.item)
    expect(ids.indexOf('join-sound-mute')).toBe(ids.indexOf('sound-mute') + 1)
    expect(item()!.textContent?.trim()).toBe('Mute join sound')
    expect(item()!.className).toBe('menu-item')
    item()!.click()
    await settle()
    expect(JSON.parse(localStorage.getItem('pulse.joinSounds.mutes')!)).toEqual({ U2: true })
    rightClick(q('[data-user="U2"]')!)
    expect(item()!.textContent?.trim()).toBe('Unmute join sound')
    item()!.click()
    await settle()
    expect(JSON.parse(localStorage.getItem('pulse.joinSounds.mutes')!)).toEqual({})
  })

  it('nobody without a join sound gets it', () => {
    mountPanel()
    rightClick(q('[data-user="U2"]')!)
    expect(menu()).not.toBeNull()
    expect(item()).toBeNull()
  })

  it('never for yourself', () => {
    withJoinSound(['U1', 'U2'])
    mountPanel()
    rightClick(q('[data-user="U1"]')!)
    expect(menu()).not.toBeNull()
    expect(item()).toBeNull()
  })
})
