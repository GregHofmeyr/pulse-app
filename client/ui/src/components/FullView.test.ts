// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
const room = vi.hoisted(() => ({
  connected: true,
  onLost: () => {},
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
import { fullPanelsContext, type PanelSide } from '../lib/stream/fullPanels'
import type { Ready } from '../lib/protocol/Ready'
import { reloadVoiceChat } from '../lib/voiceChat.svelte'
import { reloadColumns } from '../lib/columns.svelte'
import { motion } from '../lib/animate.svelte'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const flags = { muted: false, deafened: false }
const samStream = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false, allow_ghosts: true }, status: 'live' as const, viewers: [], ghosts: 0, started_at: '2026-10-26T10:00:00Z' }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [{ channel_id: 'V1', members: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }] }],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [samStream], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
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
  cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1', onMessage: () => {}, onVoiceSettings: () => {} } })
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
    // the side panel toggles are the shell's: on its own, there's nothing to toggle
    expect(q('.full-view .panel-toggle')).toBeNull()
  })

  it('with_no_member_list_only_the_left_toggle_shows_named_for_what_the_left_panel_holds', async () => {
    const toggled: PanelSide[] = []
    const panels = { left: false, right: false, leftName: 'Conversations', hasRight: false, toggle: (s: PanelSide) => void toggled.push(s) }
    cmp = mount(VoicePanel, { target: document.body, context: fullPanelsContext(panels),
      props: { channelId: 'V1', serverId: 'S1', onMessage: () => {}, onVoiceSettings: () => {} } })
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    const toggles = [...document.querySelectorAll<HTMLButtonElement>('.full-view .panel-toggle')]
    expect(toggles.map((t) => t.getAttribute('aria-label'))).toEqual(['Conversations'])
    expect(toggles[0].dataset.tip).toBe('Show conversations')
    expect(q('.full-view .bar')!.lastElementChild!.classList.contains('controls')).toBe(true)
    toggles[0].click()
    expect(toggled).toEqual(['left'])
  })

  it('the_right_toggle_is_named_for_the_voice_chat_when_that_is_what_the_right_panel_holds', async () => {
    // #226: beside a voice stage with its chat open, the right panel is the chat
    const panels = { left: false, right: false, leftName: 'Channels', hasRight: true, rightName: 'Chat', toggle: () => {} }
    cmp = mount(VoicePanel, { target: document.body, context: fullPanelsContext(panels),
      props: { channelId: 'V1', serverId: 'S1', onMessage: () => {}, onVoiceSettings: () => {} } })
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    const right = q('.full-view .panel-toggle#full-toggle-members')!
    expect(right.getAttribute('aria-label')).toBe('Chat')
    expect(right.dataset.tip).toBe('Show chat')
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

  // the focused stream itself, the way a mouse uses it: press, then click (detail = click count)
  const unfocus = () => q('.full-view .unfocus')!
  function clickStream(opts: { detail?: number; moveBy?: number; el?: HTMLElement } = {}) {
    const el = opts.el ?? unfocus()
    el.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientX: 100, clientY: 100, button: 0 }))
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: opts.detail ?? 1, clientX: 100 + (opts.moveBy ?? 0), clientY: 100 }))
    flushSync()
  }

  it('clicking_the_focused_stream_again_returns_to_the_grid_of_all_tiles', async () => {
    await fullOnSam()
    expect(q('[data-stream="U2"]')).toBeNull() // focused: the other tiles are hidden
    clickStream()
    expect(callView.full).toBeNull()
    expect(q('.full-view')).toBeNull()
    expect(q('[data-stream="U2"] video')).not.toBeNull() // the grid, still watching
    expect(stream.watching).toHaveLength(1)
  })

  it('the_stream_is_a_mouse_shortcut_not_a_tab_stop', async () => {
    await fullOnSam()
    expect(unfocus().tagName).toBe('DIV')
    expect(unfocus().hasAttribute('tabindex')).toBe(false)
    expect(unfocus().getAttribute('role')).toBeNull()
  })

  it('a_double_click_neither_flickers_back_nor_opens_another_stream', async () => {
    // into full view from the grid by double-clicking a tile: the second click is ignored
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1', onMessage: () => {}, onVoiceSettings: () => {} } })
    await streamApi.watch('V1', 'U2')
    await settle()
    const host = q('[data-stream="U2"] .host')!
    host.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }))
    flushSync()
    expect(callView.full).toEqual(SAM)
    clickStream({ detail: 2 })
    expect(callView.full).toEqual(SAM) // still in full view
    // out by double-clicking the stream: the second click lands on the grid and does nothing
    clickStream({ detail: 1 })
    expect(callView.full).toBeNull()
    q('[data-stream="U2"] .host')!.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 2 }))
    flushSync()
    expect(callView.full).toBeNull()
  })

  it('a_click_that_closes_a_menu_or_list_does_not_also_leave_full_view', async () => {
    await fullOnSam()
    const listbox = document.createElement('div')
    listbox.setAttribute('role', 'listbox')
    document.body.append(listbox)
    unfocus().dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0 }))
    listbox.remove() // the outside press closed it
    unfocus().dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }))
    flushSync()
    expect(callView.full).toEqual(SAM)
    clickStream() // the next click does
    expect(callView.full).toBeNull()
  })

  it('a_drag_or_the_click_that_brings_the_window_to_the_front_does_not_leave', async () => {
    await fullOnSam()
    clickStream({ moveBy: 20 })
    expect(callView.full).toEqual(SAM)
    // the window was in the background; this click brings it to the front
    window.dispatchEvent(new FocusEvent('blur'))
    flushSync()
    window.dispatchEvent(new FocusEvent('focus'))
    flushSync()
    clickStream()
    expect(callView.full).toEqual(SAM)
    await new Promise((r) => setTimeout(r, 350))
    clickStream()
    expect(callView.full).toBeNull()
  })

  it('the_activation_click_is_ignored_even_if_the_window_focus_event_comes_late', async () => {
    await fullOnSam()
    // the DOM's own focus event (in the page, before the press) is enough on its own
    window.dispatchEvent(new FocusEvent('focus'))
    clickStream()
    expect(callView.full).toEqual(SAM)
    await new Promise((r) => setTimeout(r, 350))
    // and the click is checked again: activation noticed between the press and the click
    unfocus().dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientX: 100, clientY: 100, button: 0 }))
    window.dispatchEvent(new FocusEvent('focus'))
    unfocus().dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1, clientX: 100, clientY: 100 }))
    flushSync()
    expect(callView.full).toEqual(SAM)
  })

  it('a_menu_left_open_in_a_hidden_panel_does_not_block_esc_or_the_click', async () => {
    await fullOnSam()
    const panel = document.createElement('aside')
    panel.hidden = true
    panel.innerHTML = '<div role="menu"></div>'
    document.body.append(panel)
    await new Promise((r) => setTimeout(r, 350))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    flushSync()
    expect(callView.full).toBeNull()
    panel.remove()
  })

  it('the_stream_menu_can_hand_focus_back_to_the_stage', async () => {
    await fullOnSam()
    expect(q('.full-view .stage')!.getAttribute('tabindex')).toBe('-1')
  })

  it('keyboard_focus_lands_on_the_streams_grid_tile_after_leaving', async () => {
    await fullOnSam()
    button('Back')!.focus()
    button('Back')!.click()
    await settle()
    expect(document.activeElement).toBe(q('[data-stream="U2"] .host'))
  })

  it('right_click_on_the_stream_opens_the_streamers_menu', async () => {
    await fullOnSam()
    const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 50, clientY: 60 })
    unfocus().dispatchEvent(e)
    flushSync()
    expect(e.defaultPrevented).toBe(true)
    expect(q('[role="menu"]')).not.toBeNull()
    expect(callView.full).toEqual(SAM)
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
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1', onMessage: () => {}, onVoiceSettings: () => {} } })
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
    cmp = mount(VoicePanel, { target: document.body, props: { channelId: 'V1', serverId: 'S1', onMessage: () => {}, onVoiceSettings: () => {} } })
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

// #63: the channel's chat as a side strip beside the stream, and the stream in full screen with the strip inside it
describe('chat strip and full screen (#63)', () => {
  const chatToggle = () => q('.full-view #stream-chat-toggle') as HTMLButtonElement | null
  const strip = () => q('.full-view aside#stream-chat')
  const screenBtn = () => (button('Full screen') ?? button('Exit full screen')) as HTMLButtonElement | undefined
  const esc = (target: EventTarget = window) => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
  }
  const fullscreenCalls = () => invoke.mock.calls.filter((c) => c[0] === 'set_fullscreen').map((c) => (c[1] as { on: boolean }).on)
  const openChatFromStorage = () => {
    localStorage.setItem('pulse.voiceChat', JSON.stringify({ open: true }))
    reloadVoiceChat()
  }

  beforeEach(() => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : undefined))
    Element.prototype.scrollTo ??= () => {}
    reloadVoiceChat()
    reloadColumns()
  })
  afterEach(() => {
    motion.reducedMotion = false
  })

  it('a labelled chat toggle in the bar opens the same chat beside the stream, and remembers it on this device', async () => {
    await fullOnSam()
    const t = chatToggle()!
    expect(t.closest('.trail')).not.toBeNull()
    expect(t.getAttribute('aria-label')).toBe('Chat')
    expect(t.getAttribute('aria-pressed')).toBe('false')
    expect(t.getAttribute('aria-controls')).toBe('stream-chat')
    expect(t.dataset.tip).toBe('Show chat')
    expect(t.querySelector('svg')).not.toBeNull()
    expect(strip()).toBeNull()
    t.click()
    await settle()
    const s = strip()!
    expect(s.getAttribute('aria-label')).toBe('Chat')
    // the voice chat panel's own view: its Chat header, close button and composer
    expect(s.querySelector('.chan.panel .head .title')?.textContent).toBe('Chat')
    expect(s.querySelector('[aria-label="Close chat"]')).not.toBeNull()
    expect(s.querySelector('[role="textbox"]')).not.toBeNull()
    // beside the stream, not instead of it
    expect(q('.full-view .stage video')).not.toBeNull()
    expect(t.getAttribute('aria-pressed')).toBe('true')
    expect(t.dataset.tip).toBe('Hide chat')
    expect(JSON.parse(localStorage.getItem('pulse.voiceChat')!)).toEqual({ open: true })
    t.click()
    await settle()
    expect(strip()).toBeNull()
    expect(JSON.parse(localStorage.getItem('pulse.voiceChat')!)).toEqual({ open: false })
  })

  it('opens already open when the chat was left open, fading in', async () => {
    openChatFromStorage()
    await fullOnSam()
    expect(strip()).not.toBeNull()
    // app.css: no animation under reduced motion
    expect(strip()!.classList.contains('fade-in')).toBe(true)
  })

  it("the strip's close button closes it and hands the focus to the toggle", async () => {
    await fullOnSam()
    chatToggle()!.click()
    await settle()
    const close = strip()!.querySelector<HTMLButtonElement>('[aria-label="Close chat"]')!
    close.focus()
    close.click()
    await settle()
    expect(document.activeElement).toBe(chatToggle())
    expect(chatToggle()!.getAttribute('aria-pressed')).toBe('false')
  })

  it('under reduced motion the strip goes at once', async () => {
    motion.reducedMotion = true
    await fullOnSam()
    chatToggle()!.click()
    await settle()
    chatToggle()!.click()
    flushSync()
    expect(strip()).toBeNull()
  })

  it('new messages while it is closed: a dot on the toggle (a count for mentions); none while open', async () => {
    await fullOnSam()
    const read = { channel_id: 'V1', last_read_message_id: null, unread: 3, mentions: 0 }
    app.state = { ...app.state, reads: { ...app.state.reads, V1: read } }
    flushSync()
    expect(chatToggle()!.querySelector('.chat-dot')).not.toBeNull()
    expect(chatToggle()!.getAttribute('aria-label')).toBe('Chat, unread messages')
    app.state = { ...app.state, reads: { ...app.state.reads, V1: { ...read, mentions: 2 } } }
    flushSync()
    expect(chatToggle()!.querySelector('.chat-dot')).toBeNull()
    expect(chatToggle()!.querySelector('.chat-pill')?.textContent).toBe('2')
    expect(chatToggle()!.getAttribute('aria-label')).toBe('Chat, 2 mentions')
    chatToggle()!.click()
    await settle()
    expect(chatToggle()!.querySelector('.chat-dot, .chat-pill')).toBeNull()
    expect(chatToggle()!.getAttribute('aria-label')).toBe('Chat')
  })

  it('resizes like the other columns, with its own remembered width', async () => {
    await fullOnSam()
    chatToggle()!.click()
    await settle()
    const h = q('.full-view [role=separator][aria-label="Resize the chat"]')!
    expect(h.getAttribute('aria-controls')).toBe('stream-chat')
    expect(h.getAttribute('aria-valuenow')).toBe('300')
    expect(strip()!.style.width).toBe('300px')
    h.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
    flushSync()
    expect(strip()!.style.width).toBe('316px')
    expect(JSON.parse(localStorage.getItem('pulse.columns')!).strip).toBe(316)
    // the stage's chat panel keeps its own
    expect(JSON.parse(localStorage.getItem('pulse.columns')!).chat).toBeNull()
  })

  it('full screen: the window goes full screen with the stream over the whole app, the strip inside it', async () => {
    openChatFromStorage()
    await fullOnSam()
    const b = screenBtn()!
    expect(b.getAttribute('aria-label')).toBe('Full screen')
    expect(b.dataset.tip).toBe('Full screen')
    expect(b.closest('.trail')).not.toBeNull()
    expect(q('.full-view')!.classList.contains('screen')).toBe(false)
    b.click()
    await settle()
    expect(fullscreenCalls()).toEqual([true])
    expect(q('.full-view')!.classList.contains('screen')).toBe(true)
    expect(screenBtn()!.getAttribute('aria-label')).toBe('Exit full screen')
    expect(q('.full-view.screen aside#stream-chat')).not.toBeNull()
    expect(q('.full-view.screen #stream-chat-toggle')).not.toBeNull()
    screenBtn()!.click()
    await settle()
    expect(fullscreenCalls()).toEqual([true, false])
    expect(q('.full-view')!.classList.contains('screen')).toBe(false)
  })

  it('a window already full screen stays full screen after', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_fullscreen' ? true : cmd === 'list_messages' ? [] : undefined))
    await fullOnSam()
    screenBtn()!.click()
    await settle()
    screenBtn()!.click()
    await settle()
    expect(fullscreenCalls()).toEqual([true, true])
  })

  it('Esc leaves full screen first, then full view; Esc in the message box does neither', async () => {
    await fullOnSam()
    chatToggle()!.click()
    await settle()
    screenBtn()!.click()
    await settle()
    const box = strip()!.querySelector<HTMLElement>('[role="textbox"]')!
    box.focus()
    esc(box)
    expect(q('.full-view.screen')).not.toBeNull()
    box.blur()
    esc()
    await settle()
    expect(fullscreenCalls()).toEqual([true, false])
    expect(q('.full-view')).not.toBeNull()
    expect(q('.full-view.screen')).toBeNull()
    esc()
    expect(q('.full-view')).toBeNull()
  })

  it('leaving full view while full screen gives the window back', async () => {
    await fullOnSam()
    screenBtn()!.click()
    await settle()
    button('Back')!.click()
    await settle()
    expect(q('.full-view')).toBeNull()
    expect(fullscreenCalls()).toEqual([true, false])
  })

  it('in full screen the hidden side panels have no toggles, and a click on the stream stays', async () => {
    const panels = { left: false, right: false, leftName: 'Channels', hasRight: true, toggle: () => {} }
    cmp = mount(VoicePanel, { target: document.body, context: fullPanelsContext(panels),
      props: { channelId: 'V1', serverId: 'S1', onMessage: () => {}, onVoiceSettings: () => {} } })
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    expect(document.querySelectorAll('.full-view .panel-toggle')).toHaveLength(2)
    screenBtn()!.click()
    await settle()
    expect(document.querySelectorAll('.full-view .panel-toggle')).toHaveLength(0)
    await new Promise((r) => setTimeout(r, 350))
    const u = q('.full-view .unfocus')!
    u.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientX: 100, clientY: 100, button: 0 }))
    u.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1, clientX: 100, clientY: 100 }))
    flushSync()
    expect(callView.full).toEqual(SAM)
  })

  it("the stage header's chat toggle points at the strip while a stream is in full view", async () => {
    await fullOnSam()
    expect(q('#voice-chat-toggle')!.getAttribute('aria-controls')).toBe('stream-chat')
    q('#voice-chat-toggle')!.click()
    await settle()
    expect(strip()).not.toBeNull()
  })
})
