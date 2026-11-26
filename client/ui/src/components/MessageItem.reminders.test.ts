// @vitest-environment jsdom
// Remind me (#300) in a message's menu: a flyout of times (and Custom…, a day and a time) that sets a reminder. Also
// (#412) in a quote's line menu, a clip's menu, and the menu of a reply in a quote's or clip's thread.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { MessageReminder } from '../lib/protocol/MessageReminder'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import QuoteItem from './QuoteItem.svelte'
import ClipActions from './ClipActions.svelte'
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
    // the day comes from the shared DatePicker (#643)
    picker.querySelector<HTMLButtonElement>('button.date-field')!.click(); flushSync()
    document.querySelector<HTMLButtonElement>('.date-picker [data-day="2026-11-20"]')!.click(); flushSync()
    const time = picker.querySelector<HTMLInputElement>('input[type=time]')!
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
    // today, the day it starts on (#643: past days can't be picked at all)
    expect(picker.querySelector('button.date-field')!.textContent).toContain('16 Nov 2026')
    const time = picker.querySelector<HTMLInputElement>('input[type=time]')!
    time.value = '09:00'; time.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect([...picker.querySelectorAll('button')].find((b) => b.textContent === 'Set reminder')!.disabled).toBe(true)
    expect(picker.textContent).toContain('That time has passed.')
  })
})

describe('Remind me in quotes and clips (#412)', () => {
  const quoteMsg = msg({ content: '“hi” — sam', quote: { lines: [{ id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hi', media: null, caption: null, awards: 0, awarders: [], awarded: false }], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] } })
  const clipMsg = msg({ content: 'nice shot', clip: { game_id: null } as unknown as Message['clip'] })

  it('a reply in a quote’s thread has a menu with Remind me alone', async () => {
    const reply = msg({ id: 'M1', thread_root_id: 'Q1' })
    app.state = {
      ...emptyState(),
      channels: { C1: { id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 } },
      messages: { C1: [reply] },
    }
    cmp = mount(MessageItem, { target: document.body, props: {
      message: reply, serverId: 'S1', name: 'sam', nameOf: (id: string | null) => (id === 'U2' ? 'sam' : 'alex'), replyTo: null,
      grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
    } })
    flushSync()
    btn('More')!.click(); flushSync()
    expect(items().map((b) => b.textContent?.trim())).toEqual(['Remind me'])
    hover(item('Remind me')!)
    choices()[1].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('create_reminder', { messageId: 'M1', remindAt: new Date(NOW + 60 * 60_000).toISOString() })
  })

  it('a quote’s line menu reminds you about the whole quote', async () => {
    app.state = {
      ...emptyState(),
      channels: { C1: { id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 } },
      messages: { C1: [quoteMsg] },
    }
    cmp = mount(QuoteItem, { target: document.body, props: {
      message: quoteMsg, serverId: 'S1', meId: 'U1', nameOf: (id: string | null) => (id === 'U2' ? 'sam' : 'alex'),
      onReply: () => {}, onEdit: () => {}, onDelete: async () => {},
    } })
    flushSync()
    document.querySelector<HTMLElement>('.line')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
    flushSync()
    const remind = [...document.querySelectorAll<HTMLButtonElement>('[role=menu][aria-label="Line actions"] > [role=menuitem]')].find((b) => b.textContent?.trim() === 'Remind me')!
    hover(remind)
    choices()[0].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('create_reminder', { messageId: 'M1', remindAt: new Date(NOW + 20 * 60_000).toISOString() })
    expect(ui.notice).toBe("I'll remind you today at 10:20")
  })

  it('a clip’s menu has Remind me, and Custom… asks for a time', async () => {
    app.state = {
      ...emptyState(),
      channels: { C1: { id: 'C1', server_id: 'S1', kind: 'clips', name: 'clips', position: 0 } },
      messages: { C1: [clipMsg] },
    }
    cmp = mount(ClipActions, { target: document.body, props: { message: clipMsg, serverId: 'S1', mine: false, onDownload: () => {} } })
    flushSync()
    btn('Clip actions')!.click(); flushSync()
    const remind = [...document.querySelectorAll<HTMLButtonElement>('.clip-actions [role=menu] > [role=menuitem]')].find((b) => b.textContent?.trim() === 'Remind me')!
    hover(remind)
    expect(choices().map((b) => b.textContent?.trim())).toEqual(['In 20 minutes', 'In 1 hour', 'In 3 hours', 'Tomorrow at 09:00', 'Custom…'])
    choices().at(-1)!.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Remind me about this"]')).not.toBeNull()
  })
})
