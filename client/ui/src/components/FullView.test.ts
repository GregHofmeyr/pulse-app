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
import LivePopover from './LivePopover.svelte'
import { clickOutside } from '../lib/actions'
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
  channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [{ channel_id: 'V1', members: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }] }],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [samStream], custom_emoji: [], frequent_emoji: [],
}
const SAM = { channelId: 'V1', streamer: 'U2' }

let cmp: ReturnType<typeof mount> | undefined
const q = (sel: string) => document.querySelector<HTMLElement>(sel)
const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text || b.getAttribute('aria-label') === text)
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  await new Promise((r) => setTimeout(r, 0))
  flushSync()
}
/** The call view on V1, watching sam's stream in full view. */
async function fullOnSam() {
  cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1' } })
  await streamApi.watch('V1', 'U2')
  openFull(SAM)
  await settle()
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
  voice.speaking = new Set()
  stream.live = null
  stream.watching = []
  stream.volumes = {}
  callView.full = null
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

describe('full view', () => {
  it('takes_the_grid_area_with_the_stream_on_the_high_layer', async () => {
    await fullOnSam()
    expect(q('.full-view')).not.toBeNull()
    expect(q('.grid')).toBeNull()
    expect(q('.full-view .stage video')).not.toBeNull()
    expect(q('.full-view .bar')!.textContent).toContain('sam')
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'high')
  })

  it('back_and_esc_return_to_the_grid_and_keep_watching', async () => {
    await fullOnSam()
    button('Back')!.click()
    flushSync()
    expect(callView.full).toBeNull()
    expect(q('.full-view')).toBeNull()
    expect(q('[data-stream="U2"] video')).not.toBeNull()
    expect(stream.watching).toEqual([{ channelId: 'V1', streamer: 'U2', quality: 'auto', view: 'preview' }])
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'low')

    openFull(SAM)
    flushSync()
    room.setLayer.mockClear()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    flushSync()
    expect(callView.full).toBeNull()
    expect(q('.full-view')).toBeNull()
    expect(stream.watching).toHaveLength(1)
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'low')
  })

  it('esc_while_typing_is_left_alone', async () => {
    await fullOnSam()
    const input = document.createElement('input')
    document.body.append(input)
    input.focus()
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(callView.full).toEqual(SAM)
    expect(q('.full-view')).not.toBeNull()
  })

  it('strip_shows_speaking_rings', async () => {
    await fullOnSam()
    const strip = q('.full-view .strip')!
    expect(strip.querySelectorAll('.tile.compact')).toHaveLength(2)
    expect(strip.querySelector('[data-user="U2"]')!.classList.contains('speaking')).toBe(false)
    voice.speaking = new Set(['U2'])
    flushSync()
    expect(strip.querySelector('[data-user="U2"]')!.classList.contains('speaking')).toBe(true)
    expect(strip.querySelector('[data-user="U1"]')!.classList.contains('speaking')).toBe(false)
  })

  it('stream_volume_is_the_remembered_one_and_saves', async () => {
    stream.volumes = { U2: 150 }
    await fullOnSam()
    const slider = q('.full-view input[aria-label="Stream volume"]') as HTMLInputElement
    expect(slider.value).toBe('150')
    slider.value = '60'
    slider.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(invoke).toHaveBeenCalledWith('set_stream_volume', { userId: 'U2', pct: 60 })
    expect(JSON.parse(localStorage.getItem('pulse.stream.volumes')!)).toEqual({ U2: 60 })
    expect(q('.full-view .bar')!.textContent).toContain('60%')
  })

  it('stop_watching_ends_the_watch_and_the_full_view', async () => {
    await fullOnSam()
    button('Stop watching')!.click()
    await settle()
    expect(stream.watching).toEqual([])
    expect(callView.full).toBeNull()
    expect(q('.full-view')).toBeNull()
    expect(invoke).toHaveBeenCalledWith('stream_unwatch', { channelId: 'V1', streamer: 'U2' })
  })

  it('the_tiles_full_view_button_opens_it', async () => {
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1' } })
    await streamApi.watch('V1', 'U2')
    await settle()
    button('Full view')!.click()
    flushSync()
    expect(callView.full).toEqual(SAM)
    expect(q('.full-view')).not.toBeNull()
  })

  it('esc_closes_a_popover_over_full_view_not_the_full_view', async () => {
    await fullOnSam()
    const anchor = document.createElement('span')
    document.body.append(anchor)
    const onClose = vi.fn()
    const pop = mount(LivePopover, { target: document.body, props: { channelId: 'V1', userId: 'U2', anchor, onClose } })
    flushSync()
    let seen: KeyboardEvent | undefined
    const last = (e: KeyboardEvent) => (seen = e)
    window.addEventListener('keydown', last)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    window.removeEventListener('keydown', last)
    flushSync()
    expect(onClose).toHaveBeenCalled()
    expect(seen?.defaultPrevented).toBe(true)
    expect(callView.full).toEqual(SAM)
    unmount(pop)
  })

  it('esc_closes_a_menu_over_full_view_not_the_full_view', async () => {
    await fullOnSam()
    const menu = document.createElement('div')
    menu.setAttribute('role', 'menu')
    document.body.append(menu)
    const close = vi.fn(() => menu.remove())
    const a = clickOutside(menu, close)
    await new Promise((r) => setTimeout(r, 0)) // armed after the opening click
    let seen: KeyboardEvent | undefined
    const last = (e: KeyboardEvent) => (seen = e)
    window.addEventListener('keydown', last)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    window.removeEventListener('keydown', last)
    expect(close).toHaveBeenCalled()
    expect(seen?.defaultPrevented).toBe(true)
    expect(callView.full).toEqual(SAM)
    a.destroy()
    // nothing over it any more: Esc is the full view's again
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    flushSync()
    expect(callView.full).toBeNull()
  })

  it('opening_another_stream_in_full_view_sets_the_old_one_back_to_preview', () => {
    stream.watching = [
      { channelId: 'V1', streamer: 'U2', quality: 'auto', view: 'full' },
      { channelId: 'V1', streamer: 'U3', quality: 'auto', view: 'preview' },
    ]
    callView.full = SAM
    openFull({ channelId: 'V1', streamer: 'U3' })
    expect(stream.watching.map((w) => w.view)).toEqual(['preview', 'full'])
    expect(callView.full).toEqual({ channelId: 'V1', streamer: 'U3' })
    // the same stream again changes nothing
    openFull({ channelId: 'V1', streamer: 'U3' })
    expect(stream.watching.map((w) => w.view)).toEqual(['preview', 'full'])
    stream.watching = []
  })

  it('stream_volume_is_one_value_the_tile_and_full_view_both_follow', async () => {
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1' } })
    await streamApi.watch('V1', 'U2')
    await settle()
    // set from elsewhere (e.g. a menu): the tile shows it
    streamApi.setStreamVolume('U2', 40)
    flushSync()
    expect((q('[data-stream="U2"] input[aria-label="Stream volume"]') as HTMLInputElement).value).toBe('40')
    openFull(SAM)
    flushSync()
    expect((q('.full-view input[aria-label="Stream volume"]') as HTMLInputElement).value).toBe('40')
    streamApi.setStreamVolume('U2', 170)
    flushSync()
    expect((q('.full-view input[aria-label="Stream volume"]') as HTMLInputElement).value).toBe('170')
    expect(stream.volumes).toEqual({ U2: 170 })
  })
})
