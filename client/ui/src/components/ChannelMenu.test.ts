// @vitest-environment jsdom
// The right-click menu on a sidebar channel row (ChannelList + ChannelMenu + DeleteChannelDialog).
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SUBMENU_CLOSE_DELAY } from '../lib/hoverIntent'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { voice } from '../lib/voice.svelte'
import { testMe, testReady } from '../lib/testReady'
import type { Channel } from '../lib/protocol/Channel'
import type { Ready } from '../lib/protocol/Ready'

const channels: Channel[] = [
  { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
  { id: 'T2', server_id: 'S1', kind: 'text', name: 'old-stuff', position: 1 },
  { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 },
  { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
]
const unread = { read_states: [{ channel_id: 'T2', last_read_message_id: 'M1', unread: 3, mentions: 0 }] }

let cmp: ReturnType<typeof mount> | undefined
let onJoinVoice = vi.fn()
function render(over: Partial<Ready> = unread) {
  if (cmp) unmount(cmp)
  app.state = applyReady(emptyState(), testReady(channels, over))
  onJoinVoice = vi.fn()
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice, onMessage: () => {}, onVoiceSettings: () => {} },
  })
  flushSync()
}
const row = (name: string) => [...document.querySelectorAll<HTMLElement>('.row')].find((r) => r.textContent?.includes(name))!
const menu = () => document.querySelector<HTMLElement>('[role=menu]')
const items = () => [...(menu()?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])]
const item = (label: string) => items().find((b) => b.textContent?.trim() === label)
const labels = () => items().map((b) => b.textContent?.trim())
function rightClick(name: string, clientX = 40) {
  row(name).dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX, clientY: 50 }))
  flushSync()
}
/** The mute flyout (#131): its own menu, not inside the main one's items. */
const flyout = () => document.querySelector<HTMLElement>('[role=menu][aria-label^="Mute "]')
const flyoutLabels = () => [...(flyout()?.querySelectorAll('[role=menuitem]') ?? [])].map((b) => b.textContent?.trim())
const flyoutItem = (label: string) => [...(flyout()?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])].find((b) => b.textContent?.trim() === label)
const muteItem = () => item('Mute channel')!
// a pointer arriving: pointerover bubbles, pointerenter doesn't
const hover = (el: Element) => { el.dispatchEvent(new Event('pointerover', { bubbles: true })); el.dispatchEvent(new Event('pointerenter')); flushSync() }
const key = (el: Element, k: string) => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
  el.dispatchEvent(e)
  flushSync()
  return e
}
const settle = async () => {
  for (let i = 0; i < 4; i++) await tick()
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  Object.assign(voice, { channelId: null, joining: null, leaving: null })
  render()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('a text channel', () => {
  it('right_click_opens_its_menu_at_the_pointer_with_read_mute_rename_and_delete', () => {
    rightClick('old-stuff')
    expect(menu()?.getAttribute('aria-label')).toBe('Actions for #old-stuff')
    expect(labels()).toEqual(['Mark as read', 'Mute channel', 'Notifications', 'Rename channel…', 'Delete channel…'])
    expect(menu()!.style.left).toBe('40px')
  })

  it('mark_as_read_is_disabled_when_nothing_is_unread', () => {
    rightClick('general')
    expect(item('Mark as read')!.disabled).toBe(true)
  })

  it('mark_as_read_moves_the_read_point_to_the_newest_message', async () => {
    // the latest page comes newest first
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [{ id: 'M9' }, { id: 'M5' }] : undefined))
    rightClick('old-stuff')
    item('Mark as read')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('list_messages', { channelId: 'T2', before: null })
    expect(invoke).toHaveBeenCalledWith('mark_read', { channelId: 'T2', messageId: 'M9' })
    expect(menu()).toBeNull()
  })

  it('mutes_for_a_length_or_unmutes', async () => {
    const before = Date.now()
    rightClick('general')
    hover(muteItem())
    flyoutItem('For 15 minutes')!.click()
    await settle()
    const [, args] = invoke.mock.calls.find(([c]) => c === 'set_mute')!
    expect(args.targetKind).toBe('channel')
    expect(args.targetId).toBe('T1')
    expect(Date.parse(args.until) - before).toBeGreaterThanOrEqual(15 * 60_000)
    expect(Date.parse(args.until) - before).toBeLessThan(16 * 60_000)

    render({ mutes: [{ target_kind: 'channel', target_id: 'T1', until: null }] })
    rightClick('general')
    expect(labels()).not.toContain('Mute channel')
    expect(item('Unmute channel')!.hasAttribute('aria-haspopup')).toBe(false)
    item('Unmute channel')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('clear_mute', { targetKind: 'channel', targetId: 'T1' })
  })

  it('rename_edits_the_name_in_the_row_and_saves_it_like_a_new_channel_name', async () => {
    invoke.mockResolvedValue({ id: 'T1', server_id: 'S1', kind: 'text', name: 'chat-room', position: 0 })
    rightClick('general')
    item('Rename channel…')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename #general"]')!
    expect(input.value).toBe('general')
    expect(document.activeElement).toBe(input)
    input.value = 'Chat Room'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenCalledWith('rename_channel', { channelId: 'T1', name: 'chat-room' })
    expect(document.querySelector('input[aria-label="Rename #general"]')).toBeNull()
  })

  it('focus_is_back_on_the_row_after_a_rename', async () => {
    invoke.mockResolvedValue({ id: 'T1', server_id: 'S1', kind: 'text', name: 'chat', position: 0 })
    rightClick('general')
    item('Rename channel…')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename #general"]')!
    input.value = 'chat'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(document.activeElement).toBe(row('general'))
  })

  it('a_channel_deleted_while_its_menu_or_dialog_is_open_closes_them', async () => {
    const { applyEvent } = await import('../lib/state')
    const gone = (id: string) => {
      app.state = applyEvent(app.state, { t: 'ChannelDeleted', d: { channel_id: id, server_id: 'S1' } }, Date.now())
      flushSync()
    }
    rightClick('old-stuff')
    gone('T2')
    expect(menu()).toBeNull()
    rightClick('quotes')
    item('Delete channel…')!.click()
    flushSync()
    gone('Q1')
    expect(document.querySelector('[role=dialog]')).toBeNull()
  })

  it('escape_leaves_a_rename_without_saving', async () => {
    rightClick('general')
    item('Rename channel…')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename #general"]')!
    input.value = 'nope'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await settle()
    expect(invoke).not.toHaveBeenCalledWith('rename_channel', expect.anything())
    expect(row('general')).toBeTruthy()
  })

  it('a_refused_rename_says_why_and_keeps_the_field', async () => {
    invoke.mockRejectedValue({ kind: 'rejected', message: 'name must be 1-64 characters' })
    rightClick('general')
    item('Rename channel…')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename #general"]')!
    input.value = 'x'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(document.querySelector('input[aria-label="Rename #general"]')).not.toBeNull()
    expect(document.querySelector('.err')?.textContent).not.toBe('')
  })

  it('delete_asks_in_a_dialog_first_and_cancel_keeps_it', async () => {
    rightClick('old-stuff')
    item('Delete channel…')!.click()
    flushSync()
    const dialog = document.querySelector<HTMLElement>('[role=dialog]')!
    expect(dialog.getAttribute('aria-label')).toBe('Delete #old-stuff')
    expect(dialog.textContent).toContain('Delete #old-stuff? It moves to Recently deleted for 7 days, where the server owner or you can restore it.')
    expect(dialog.textContent).not.toContain("can't be undone")
    ;[...dialog.querySelectorAll('button')].find((b) => b.textContent === 'Cancel')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('delete_channel', expect.anything())
  })

  it('delete_deletes_once_confirmed', async () => {
    rightClick('old-stuff')
    item('Delete channel…')!.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=dialog] button')].find((b) => b.textContent === 'Delete channel')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('delete_channel', { channelId: 'T2' })
    expect(document.querySelector('[role=dialog]')).toBeNull()
  })

  it('a_refused_delete_stays_open_and_says_why', async () => {
    invoke.mockRejectedValue({ kind: 'rejected', message: 'Too many requests' })
    rightClick('quotes')
    item('Delete channel…')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')?.textContent).toContain('Delete #quotes? It moves to Recently deleted')
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=dialog] button')].find((b) => b.textContent === 'Delete channel')!.click()
    await settle()
    expect(document.querySelector('[role=dialog] [role=alert]')).not.toBeNull()
  })
})

describe('from the keyboard', () => {
  it('shift_f10_or_the_menu_key_on_a_focused_row_opens_it_under_the_row_with_focus_on_the_first_item', () => {
    const r = row('old-stuff')
    r.focus()
    r.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }))
    flushSync()
    expect(menu()).not.toBeNull()
    expect(document.activeElement).toBe(item('Mark as read'))
    menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(r)
    r.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }))
    flushSync()
    expect(menu()).not.toBeNull()
  })

  it('arrows_skip_a_disabled_item', () => {
    const r = row('general')
    r.focus()
    r.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }))
    flushSync()
    // Mark as read is disabled here: the first item is Mute channel
    expect(document.activeElement).toBe(item('Mute channel'))
    menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true }))
    expect(document.activeElement).toBe(item('Delete channel…'))
  })
})

describe('the mute flyout', () => {
  it('mute_channel_is_one_item_that_owns_a_closed_submenu', () => {
    rightClick('general')
    const m = muteItem()
    expect(m.getAttribute('aria-haspopup')).toBe('menu')
    expect(m.getAttribute('aria-expanded')).toBe('false')
    expect(flyout()).toBeNull()
  })

  it('hovering_it_opens_the_durations_to_the_left_and_another_item_closes_them_after_a_short_wait', () => {
    vi.useFakeTimers()
    rightClick('general', 600)
    hover(muteItem())
    expect(muteItem().getAttribute('aria-expanded')).toBe('true')
    expect(flyoutLabels()).toEqual(['For 15 minutes', 'For 1 hour', 'For 8 hours', 'For 24 hours', 'Until I turn it back on'])
    expect(flyout()!.dataset.side).toBe('left')
    expect(muteItem().getAttribute('aria-controls')).toBe(flyout()!.id)
    // hover only: focus stays where it was
    expect(flyout()!.contains(document.activeElement)).toBe(false)
    hover(item('Rename channel…')!)
    // hover intent (#173): crossing another item on the way to the flyout doesn't close it at once
    expect(flyout()).not.toBeNull()
    vi.advanceTimersByTime(SUBMENU_CLOSE_DELAY); flushSync()
    expect(flyout()).toBeNull()
    expect(muteItem().getAttribute('aria-expanded')).toBe('false')
    vi.useRealTimers()
  })

  it('crossing_another_item_into_the_flyout_keeps_it_open', () => {
    vi.useFakeTimers()
    rightClick('general', 600)
    hover(muteItem())
    hover(item('Rename channel…')!)
    vi.advanceTimersByTime(SUBMENU_CLOSE_DELAY / 2)
    flyout()!.dispatchEvent(new Event('pointerenter'))
    vi.advanceTimersByTime(SUBMENU_CLOSE_DELAY * 2); flushSync()
    expect(flyout()).not.toBeNull()
    vi.useRealTimers()
  })

  it('near_the_left_edge_it_opens_to_the_right', () => {
    rightClick('general', 40)
    hover(muteItem())
    expect(flyout()!.dataset.side).toBe('right')
  })

  it('in_a_window_too_narrow_for_either_side_it_is_pulled_back_inside_the_right_edge', () => {
    // #134: a 400px window, the menu 230px wide at 40: 122px to its right, short of the flyout's 204
    const w = window.innerWidth
    Object.assign(window, { innerWidth: 400 })
    vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('menu') ? 230 : 0 })
    try {
      rightClick('general', 40)
      hover(muteItem())
      expect(flyout()!.dataset.side).toBe('right')
      expect(flyout()!.style.marginLeft).toBe(`${-(204 - 122)}px`)
    } finally {
      vi.restoreAllMocks()
      Object.assign(window, { innerWidth: w })
    }
  })

  it('arrow_right_or_enter_opens_it_with_focus_on_the_first_duration_and_arrows_move_inside_it', () => {
    rightClick('general')
    muteItem().focus()
    key(muteItem(), 'ArrowRight')
    expect(document.activeElement).toBe(flyoutItem('For 15 minutes'))
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(flyoutItem('For 1 hour'))
    key(document.activeElement!, 'ArrowUp')
    key(document.activeElement!, 'ArrowUp')
    expect(document.activeElement).toBe(flyoutItem('Until I turn it back on'))
    key(document.activeElement!, 'Home')
    expect(document.activeElement).toBe(flyoutItem('For 15 minutes'))
    key(document.activeElement!, 'End')
    expect(document.activeElement).toBe(flyoutItem('Until I turn it back on'))
    key(document.activeElement!, 'ArrowLeft')
    expect(flyout()).toBeNull()
    expect(document.activeElement).toBe(muteItem())
    expect(menu()).not.toBeNull()
    // Enter is the button's click
    muteItem().click(); flushSync()
    expect(document.activeElement).toBe(flyoutItem('For 15 minutes'))
  })

  it('escape_in_the_flyout_closes_only_the_flyout', () => {
    rightClick('general')
    muteItem().focus()
    key(muteItem(), 'ArrowRight')
    const e = key(document.activeElement!, 'Escape')
    expect(e.defaultPrevented).toBe(true)
    expect(flyout()).toBeNull()
    expect(menu()).not.toBeNull()
    expect(document.activeElement).toBe(muteItem())
  })

  it('picking_a_duration_from_the_keyboard_mutes_and_puts_focus_back_on_the_row', async () => {
    rightClick('general')
    muteItem().focus()
    key(muteItem(), 'ArrowRight')
    key(document.activeElement!, 'End')
    ;(document.activeElement as HTMLButtonElement).click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_mute', { targetKind: 'channel', targetId: 'T1', until: null })
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(row('general'))
  })

  it('hovering_another_item_while_focus_is_in_the_flyout_keeps_focus_in_the_menu', () => {
    vi.useFakeTimers()
    rightClick('general')
    muteItem().focus()
    key(muteItem(), 'ArrowRight')
    hover(item('Rename channel…')!)
    vi.advanceTimersByTime(SUBMENU_CLOSE_DELAY); flushSync()
    vi.useRealTimers()
    expect(flyout()).toBeNull()
    expect(document.activeElement).toBe(muteItem())
  })

  it('escape_with_a_hover_opened_flyout_closes_only_the_flyout', () => {
    rightClick('general')
    hover(muteItem())
    const e = key(item('Mark as read') ?? muteItem(), 'Escape')
    expect(e.defaultPrevented).toBe(true)
    expect(flyout()).toBeNull()
    expect(menu()).not.toBeNull()
    key(muteItem(), 'Escape')
    expect(menu()).toBeNull()
  })

  it('arrow_down_from_mute_channel_moves_on_in_the_main_menu_and_closes_the_flyout', () => {
    rightClick('general')
    hover(muteItem())
    muteItem().focus()
    key(muteItem(), 'ArrowDown')
    expect(document.activeElement).toBe(item('Notifications'))
    expect(flyout()).toBeNull()
  })
})

describe('a voice channel', () => {
  it('offers_join_then_its_chats_read_mute_and_notifications_then_rename_and_delete', () => {
    // #226: its chat has unreads, a mute and a notification level like a text channel
    rightClick('Lounge')
    expect(labels()).toEqual(['Join', 'Mark as read', 'Mute channel', 'Notifications', 'Channel settings…', 'Rename channel…', 'Delete channel…'])
    item('Join')!.click()
    flushSync()
    expect(onJoinVoice).toHaveBeenCalledWith('V1')
  })

  it('offers_leave_while_you_are_in_it', async () => {
    voice.channelId = 'V1'
    rightClick('Lounge')
    expect(labels()[0]).toBe('Leave')
    item('Leave')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('leave_voice')
  })

  it('renames_keeping_its_case', async () => {
    invoke.mockResolvedValue({ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Game Night', position: 0 })
    rightClick('Lounge')
    item('Rename channel…')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename Lounge"]')!
    input.value = 'Game Night'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenCalledWith('rename_channel', { channelId: 'V1', name: 'Game Night' })
  })

  it('delete_says_anyone_in_it_is_disconnected', () => {
    rightClick('Lounge')
    item('Delete channel…')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')?.textContent).toContain('Delete Lounge? Anyone in it is disconnected. It moves to Recently deleted for 7 days')
  })
})

describe('a voice channel’s settings (#75)', () => {
  beforeEach(() => {
    invoke.mockReset()
    render()
  })
  const dialog = () => document.querySelector<HTMLElement>('[role=dialog][aria-label="Channel settings"]')
  const trigger = () => dialog()!.querySelector<HTMLButtonElement>('button[aria-label=Bitrate]')!
  const option = (label: string) => [...dialog()!.querySelectorAll<HTMLElement>('[role=option]')].find((o) => o.textContent?.trim() === label)!

  it('opens_from_the_menu_at_default', () => {
    rightClick('Lounge')
    item('Channel settings…')!.click()
    flushSync()
    expect(menu()).toBeNull()
    expect(dialog()?.textContent).toContain('Lounge')
    expect(trigger().textContent?.trim()).toBe('Default (64 kbps)')
    expect(dialog()?.textContent).toContain('Higher sounds better, music especially, but uses more data for everyone in the call.')
  })

  it('offers_default_and_the_presets_and_a_pick_saves_it', async () => {
    invoke.mockResolvedValue({ ...channels[3], bitrate: 256 })
    rightClick('Lounge')
    item('Channel settings…')!.click()
    flushSync()
    trigger().click()
    flushSync()
    expect([...dialog()!.querySelectorAll('[role=option]')].map((o) => o.textContent?.trim())).toEqual([
      'Default (64 kbps)', '96 kbps', '128 kbps', '192 kbps', '256 kbps (max)',
    ])
    option('256 kbps (max)').dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }))
    flushSync()
    expect(invoke).toHaveBeenCalledWith('set_channel_bitrate', { channelId: 'V1', bitrate: 256 })
    expect(trigger().textContent?.trim()).toBe('256 kbps (max)')
    await tick()
  })

  it('back_to_default_sends_null_and_a_failure_puts_it_back', async () => {
    render({ ...unread })
    app.state = { ...app.state, channels: { ...app.state.channels, V1: { ...app.state.channels.V1, bitrate: 128 } } }
    rightClick('Lounge')
    item('Channel settings…')!.click()
    flushSync()
    expect(trigger().textContent?.trim()).toBe('128 kbps')
    invoke.mockRejectedValueOnce('Too many requests')
    trigger().click()
    flushSync()
    option('Default (64 kbps)').dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }))
    flushSync()
    expect(invoke).toHaveBeenCalledWith('set_channel_bitrate', { channelId: 'V1', bitrate: null })
    await tick()
    await tick()
    flushSync()
    expect(dialog()?.querySelector('[role=alert]')?.textContent).toBeTruthy()
    expect(trigger().textContent?.trim()).toBe('128 kbps')
  })

  it('someone_elses_change_shows_while_it_is_open', () => {
    rightClick('Lounge')
    item('Channel settings…')!.click()
    flushSync()
    app.state = { ...app.state, channels: { ...app.state.channels, V1: { ...app.state.channels.V1, bitrate: 192 } } }
    flushSync()
    expect(trigger().textContent?.trim()).toBe('192 kbps')
  })

  it('text_channels_have_none', () => {
    rightClick('general')
    expect(labels()).not.toContain('Channel settings…')
  })
})

describe('the notifications flyout (#195)', () => {
  const levels = () => document.querySelector<HTMLElement>('[role=menu][aria-label^="Notifications for "]')
  const choices = () => [...(levels()?.querySelectorAll<HTMLButtonElement>('[role=menuitemradio]') ?? [])]
  const choice = (label: string) => choices().find((b) => b.textContent?.trim() === label)!
  const ticked = () => choices().filter((b) => b.getAttribute('aria-checked') === 'true').map((b) => b.textContent?.trim())

  it('follows_its_server_by_default_and_the_pick_is_ticked', () => {
    rightClick('general', 600)
    hover(item('Notifications')!)
    expect(levels()!.getAttribute('aria-label')).toBe('Notifications for #general')
    expect(choices().map((b) => b.textContent?.trim())).toEqual(['Follow server', 'All messages', 'Only @mentions', 'Nothing'])
    expect(ticked()).toEqual(['Follow server'])
    expect(choice('Follow server').querySelector('svg')).not.toBeNull()
    expect(choice('Nothing').querySelector('svg')).toBeNull()

    render({ ...unread, notify_levels: [{ target_kind: 'channel', target_id: 'T1', level: 'nothing' }] })
    rightClick('general', 600)
    hover(item('Notifications')!)
    expect(ticked()).toEqual(['Nothing'])
  })

  it('picking_a_level_sets_it_and_follow_server_clears_it', async () => {
    rightClick('general', 600)
    hover(item('Notifications')!)
    choice('Only @mentions').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'channel', targetId: 'T1', level: 'mentions' })
    expect(menu()).toBeNull()
    rightClick('general', 600)
    hover(item('Notifications')!)
    choice('Follow server').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'channel', targetId: 'T1', level: null })
  })

  it('sits_under_mute_in_quotes_and_voice_too', () => {
    rightClick('quotes')
    expect(labels()).toContain('Notifications')
    rightClick('Lounge')
    expect(labels()).toContain('Notifications') // a voice channel's chat (#226)
  })

  it('opening_one_flyout_closes_the_other_after_the_hover_wait', () => {
    vi.useFakeTimers()
    rightClick('general', 600)
    hover(muteItem())
    hover(item('Notifications')!)
    expect(levels()).not.toBeNull()
    vi.advanceTimersByTime(SUBMENU_CLOSE_DELAY); flushSync()
    expect(flyout()).toBeNull()
    expect(levels()).not.toBeNull()
    vi.useRealTimers()
  })

  it('the_keyboard_opens_it_and_arrow_down_leaves_it', () => {
    rightClick('general', 600)
    key(muteItem(), 'ArrowDown')
    expect(document.activeElement).toBe(item('Notifications'))
    key(item('Notifications')!, 'ArrowRight')
    expect(document.activeElement).toBe(choice('Follow server'))
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(choice('All messages'))
    key(document.activeElement!, 'Escape')
    expect(levels()).toBeNull()
    expect(document.activeElement).toBe(item('Notifications'))
  })
})

describe('outside the server', () => {
  it('only_reading_and_muting_are_offered', () => {
    render({ ...unread, members: [{ server_id: 'S1', members: [{ user: { ...testMe, id: 'U9' }, nickname: null }] }] })
    rightClick('old-stuff')
    expect(labels()).not.toContain('Rename channel…')
    expect(labels()).not.toContain('Delete channel…')
    expect(labels()).toContain('Mark as read')
    rightClick('Lounge')
    expect(menu()).toBeNull()
  })
})
