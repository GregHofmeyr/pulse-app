// @vitest-environment jsdom
// The inbox in the shell (#306): the top bar's button and its count, the popover it opens, the quick switcher's Inbox.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a), convertFileSrc: (s: string) => s }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true, minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }),
}))
vi.mock('../components/GoLiveDialog.svelte', () => ({ default: () => {} }))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => ({}), pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import Shell from './Shell.svelte'
import { handleReady, resetState } from '../lib/store.svelte'
import { closeInbox, noteInboxEvent } from '../lib/inbox.svelte'
import { testMe, testReady } from '../lib/testReady'
import { testItem } from '../lib/testInbox'

let cmp: ReturnType<typeof mount> | undefined
const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync() }
beforeEach(() => {
  localStorage.clear()
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : cmd === 'inbox' ? { items: [testItem('M1', {}, { channel_id: 'T1' })], unread: 3 } : undefined))
  Element.prototype.scrollTo ??= () => {}
  resetState()
  closeInbox()
  handleReady(testReady([{ id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } as never], { inbox_unread: 3 }))
  cmp = mount(Shell, { target: document.body, props: { user: testMe, onLogout: () => {} } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const button = () => document.querySelector<HTMLButtonElement>('header button.inbox')!
const popover = () => document.querySelector('[role="dialog"][aria-label="Inbox"]')

describe('the inbox in the shell (#306)', () => {
  it('the top bar button shows the unread count, live, and none at zero', () => {
    expect(button().getAttribute('aria-label')).toBe('Inbox, 3 unread')
    expect(button().querySelector('.pill')?.textContent).toBe('3')
    noteInboxEvent({ t: 'InboxUpdated', d: { user_id: 'U1', unread: 120, read: [], read_upto: null, removed: [] } })
    flushSync()
    expect(button().querySelector('.pill')?.textContent).toBe('99+')
    noteInboxEvent({ t: 'InboxUpdated', d: { user_id: 'U1', unread: 0, read: [], read_upto: null, removed: [] } })
    flushSync()
    expect(button().querySelector('.pill')).toBeNull()
    expect(button().getAttribute('aria-label')).toBe('Inbox')
  })

  it('opens and closes the popover; Esc closes it and gives the focus back', async () => {
    button().click()
    await settle()
    expect(popover()).not.toBeNull()
    expect(button().getAttribute('aria-expanded')).toBe('true')
    expect(invoke).toHaveBeenCalledWith('inbox', { before: null, mentions: false })
    button().click()
    flushSync()
    expect(popover()).toBeNull()
    button().click()
    await settle()
    await new Promise((r) => setTimeout(r)) // clickOutside listens from the next tick
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(popover()).toBeNull()
    expect(document.activeElement).toBe(button())
  })

  it('the quick switcher’s Inbox opens it', async () => {
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true, cancelable: true }))
    flushSync()
    const field = document.querySelector<HTMLInputElement>('input[role="combobox"]')!
    field.value = '>inbox'
    field.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
    await settle()
    expect(popover()).not.toBeNull()
  })
})
