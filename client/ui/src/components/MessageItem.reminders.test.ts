// @vitest-environment jsdom
// Remind me (#300) in a message's menu: a flyout of times (and Custom…, a day and a time) that sets a reminder.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { MessageReminder } from '../lib/protocol/MessageReminder'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { reminders } from '../lib/reminders.svelte'

const NOW = new Date(2026, 10, 16, 10, 0).getTime()
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'the plan for friday',
  reply_to_id: null, created_at: '2026-11-16T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const made = (remind_at: string): MessageReminder => ({
  id: 'R1', message_id: 'M1', channel_id: 'C1', thread_root_id: null, remind_at, created_at: new Date(NOW).toISOString(),
  due: false, seen: false, message: { message_id: 'M1', author_id: 'U2', content: 'the plan for friday', deleted: false },
})

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message) {
  app.state = {
    ...emptyState(),
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } },
    messages: { C1: [m] },
  }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: 'sam', nameOf: (id: string | null) => (id === 'U2' ? 'sam' : 'alex'), replyTo: null,
    grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Message actions"]')
const items = () => [...(menu()?.querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]') ?? [])]
const item = (label: string) => items().find((b) => b.textContent?.trim() === label)
const flyout = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Remind me about this"]')
const choices = () => [...(flyout()?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])]
const hover = (el: Element) => { el.dispatchEvent(new Event('pointerover', { bubbles: true })); el.dispatchEvent(new Event('pointerenter')); flushSync() }
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => (cmd === 'create_reminder' ? made((args as { remindAt: string }).remindAt) : undefined))
  reminders.list = []
  ui.notice = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('Remind me (#300)', () => {
  it('offers 20 minutes, an hour, three hours, tomorrow and Custom…, and a pick sets the reminder', async () => {
    show(msg())
    btn('More')!.click(); flushSync()
    hover(item('Remind me')!)
    expect(choices().map((b) => b.textContent?.trim())).toEqual(['In 20 minutes', 'In 1 hour', 'In 3 hours', 'Tomorrow at 09:00', 'Custom…'])
    choices()[0].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('create_reminder', { messageId: 'M1', remindAt: new Date(NOW + 20 * 60_000).toISOString() })
    expect(reminders.list.map((r) => r.id)).toEqual(['R1'])
    expect(ui.notice).toBe("I'll remind you today at 10:20")
    expect(menu()).toBeNull()
  })

  it('Custom… asks for a day and a time, in your own time', async () => {
    show(msg())
    btn('More')!.click(); flushSync()
    hover(item('Remind me')!)
    choices().at(-1)!.click()
    flushSync()
    const picker = document.querySelector<HTMLElement>('[role=dialog][aria-label="Remind me about this"]')!
    expect(picker).not.toBeNull()
    const [day, time] = [...picker.querySelectorAll<HTMLInputElement>('input')]
    day.value = '2026-11-20'; day.dispatchEvent(new Event('input', { bubbles: true }))
    time.value = '18:30'; time.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    const set = [...picker.querySelectorAll('button')].find((b) => b.textContent === 'Set reminder')!
    expect(set.disabled).toBe(false)
    set.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('create_reminder', { messageId: 'M1', remindAt: new Date(2026, 10, 20, 18, 30).toISOString() })
    flushSync()
    expect(document.querySelector('[aria-label="Remind me about this"][role=dialog]')).toBeNull()
  })

  it("a time that has passed can't be picked", () => {
    show(msg())
    btn('More')!.click(); flushSync()
    hover(item('Remind me')!)
    choices().at(-1)!.click()
    flushSync()
    const picker = document.querySelector<HTMLElement>('[role=dialog][aria-label="Remind me about this"]')!
    const [day, time] = [...picker.querySelectorAll<HTMLInputElement>('input')]
    day.value = '2026-11-16'; day.dispatchEvent(new Event('input', { bubbles: true }))
    time.value = '09:00'; time.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect([...picker.querySelectorAll('button')].find((b) => b.textContent === 'Set reminder')!.disabled).toBe(true)
    expect(picker.textContent).toContain('That time has passed.')
  })
})
