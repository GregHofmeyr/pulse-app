// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem, { INLINE_ERROR_MS } from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: 'hello',
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [{ emoji: '👍', count: 1, me: false }], ...over,
})

let cmp: ReturnType<typeof mount> | undefined
const onEdit = vi.fn(async (_c: string) => {})
const onDelete = vi.fn(async () => {})
function show() {
  const m = msg()
  app.state = { ...emptyState(), messages: { C1: [m] } }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: app.state.messages['C1'][0], name: 'alex', nameOf: (id: string | null) => id ?? '', replyTo: null,
    grouped: false, mine: true, onReply: () => {}, onEdit, onDelete, meId: 'U1',
  } })
  flushSync()
}
const alert = () => document.querySelector('[role="alert"]')
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const box = () => document.querySelector<HTMLTextAreaElement>('textarea')
function type(text: string) {
  const t = box()!
  t.value = text
  t.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const press = (key: string) => {
  box()!.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
}
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  vi.useFakeTimers()
  invoke.mockReset(); invoke.mockResolvedValue(undefined)
  onEdit.mockReset(); onEdit.mockResolvedValue(undefined)
  onDelete.mockReset(); onDelete.mockResolvedValue(undefined)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('MessageItem errors', () => {
  it('failed_edit_keeps_the_box_and_text_with_the_error', async () => {
    onEdit.mockRejectedValue('edit refused')
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter')
    await settle()
    expect(box()).not.toBeNull()
    expect(box()!.value).toBe('changed text')
    expect(alert()?.textContent).toBe('edit refused')
    // The error stays (no timer) until the next save or Esc.
    vi.advanceTimersByTime(INLINE_ERROR_MS * 3)
    flushSync()
    expect(alert()?.textContent).toBe('edit refused')
    press('Escape'); flushSync()
    expect(box()).toBeNull()
    expect(alert()).toBeNull()
  })

  it('successful_edit_closes_the_box', async () => {
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter')
    await settle()
    expect(onEdit).toHaveBeenCalledWith('changed text')
    expect(box()).toBeNull()
    expect(alert()).toBeNull()
  })

  it('failed_delete_shows_inline_error_that_hides_after_5s', async () => {
    onDelete.mockRejectedValue('cannot delete')
    show()
    btn('Delete').click()
    await settle()
    expect(alert()?.textContent).toBe('cannot delete')
    vi.advanceTimersByTime(INLINE_ERROR_MS - 1); flushSync()
    expect(alert()).not.toBeNull()
    vi.advanceTimersByTime(1); flushSync()
    expect(alert()).toBeNull()
  })

  it('a_newer_error_restarts_the_clock', async () => {
    onDelete.mockRejectedValue('cannot delete')
    show()
    btn('Delete').click(); await settle()
    vi.advanceTimersByTime(INLINE_ERROR_MS - 1000)
    btn('Delete').click(); await settle()
    vi.advanceTimersByTime(INLINE_ERROR_MS - 1000); flushSync()
    expect(alert()).not.toBeNull()
    vi.advanceTimersByTime(1000); flushSync()
    expect(alert()).toBeNull()
  })

  it('unmounting_clears_the_timer', async () => {
    onDelete.mockRejectedValue('cannot delete')
    show()
    btn('Delete').click(); await settle()
    expect(vi.getTimerCount()).toBe(1)
    unmount(cmp!); cmp = undefined
    expect(vi.getTimerCount()).toBe(0)
  })

  it('a_reaction_failing_after_unmount_starts_no_timer', async () => {
    let refuse!: (e: unknown) => void
    invoke.mockImplementation((cmd: string) => (cmd === 'add_reaction' || cmd === 'remove_reaction' ? new Promise((_, rej) => (refuse = rej)) : Promise.resolve(undefined)))
    show()
    document.querySelector<HTMLButtonElement>('.chip:not(.add)')!.click()
    await settle()
    expect(refuse).toBeDefined()
    unmount(cmp!); cmp = undefined
    refuse('reaction refused')
    await settle()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('a_second_save_clears_the_old_error_while_in_flight', async () => {
    onEdit.mockRejectedValueOnce('edit refused')
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter'); await settle()
    expect(alert()?.textContent).toBe('edit refused')
    let finish!: () => void
    onEdit.mockImplementationOnce(() => new Promise<void>((done) => (finish = done)))
    press('Enter'); await settle(); flushSync()
    expect(alert()).toBeNull()
    finish(); await settle(); flushSync()
    expect(box()).toBeNull()
  })

  it('reopening_the_edit_box_shows_no_old_error', async () => {
    onEdit.mockRejectedValueOnce('edit refused')
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter'); await settle()
    expect(alert()?.textContent).toBe('edit refused')
    // the Edit button is hidden while the box is open: reopening goes through Esc
    press('Escape'); flushSync()
    btn('Edit').click(); flushSync()
    expect(alert()).toBeNull()
    expect(box()!.value).toBe('hello')
  })

  it('a_double_enter_while_saving_sends_one_edit', async () => {
    let finish!: () => void
    onEdit.mockImplementation(() => new Promise<void>((done) => (finish = done)))
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter')
    press('Enter')
    await settle()
    expect(onEdit).toHaveBeenCalledTimes(1)
    finish(); await settle(); flushSync()
    expect(box()).toBeNull()
  })

  it('failed_reaction_shows_inline_error_not_the_banner', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'add_reaction' || cmd === 'remove_reaction') throw 'reaction refused'
      return undefined
    })
    show()
    document.querySelector<HTMLButtonElement>('.chip:not(.add)')!.click()
    await settle()
    expect(alert()?.textContent).toBe('reaction refused')
    vi.advanceTimersByTime(INLINE_ERROR_MS); flushSync()
    expect(alert()).toBeNull()
  })
})
