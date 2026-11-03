// @vitest-environment jsdom
// The right-click menu on a sidebar channel row (ChannelList + ChannelMenu + DeleteChannelDialog).
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

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
function rightClick(name: string) {
  row(name).dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }))
  flushSync()
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
    expect(labels()).toEqual([
      'Mark as read', 'For 15 minutes', 'For 1 hour', 'For 8 hours', 'For 24 hours', 'Until I turn it back on', 'Rename channel…', 'Delete channel…',
    ])
    expect(menu()?.textContent).toContain('Mute channel')
    expect(menu()!.style.left).toBe('40px')
  })

  it('mark_as_read_is_disabled_when_nothing_is_unread', () => {
    rightClick('general')
    expect(item('Mark as read')!.disabled).toBe(true)
  })

  it('mark_as_read_moves_the_read_point_to_the_newest_message', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [{ id: 'M5' }, { id: 'M9' }] : undefined))
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
    item('For 15 minutes')!.click()
    await settle()
    const [, args] = invoke.mock.calls.find(([c]) => c === 'set_mute')!
    expect(args.targetKind).toBe('channel')
    expect(args.targetId).toBe('T1')
    expect(Date.parse(args.until) - before).toBeGreaterThanOrEqual(15 * 60_000)
    expect(Date.parse(args.until) - before).toBeLessThan(16 * 60_000)

    render({ mutes: [{ target_kind: 'channel', target_id: 'T1', until: null }] })
    rightClick('general')
    expect(labels()).not.toContain('For 1 hour')
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
    expect(dialog.textContent).toContain('Delete #old-stuff? Its messages go too.')
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
    invoke.mockRejectedValue({ kind: 'rejected', message: 'A Quote of the Year vote is on' })
    rightClick('quotes')
    item('Delete channel…')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')?.textContent).toContain('Its quotes go too.')
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
    // Mark as read is disabled here: the first item is the first length
    expect(document.activeElement).toBe(item('For 15 minutes'))
    menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true }))
    expect(document.activeElement).toBe(item('Delete channel…'))
  })
})

describe('a voice channel', () => {
  it('offers_join_rename_and_delete_but_no_read_or_mute', () => {
    rightClick('Lounge')
    expect(labels()).toEqual(['Join', 'Rename channel…', 'Delete channel…'])
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
    expect(document.querySelector('[role=dialog]')?.textContent).toContain('Delete Lounge? Anyone in it is disconnected.')
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
