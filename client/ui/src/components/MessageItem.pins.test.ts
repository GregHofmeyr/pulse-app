// @vitest-environment jsdom
// Pins on a message (#178): the message menu's Pin message flyout and Unpin message, the marker by the time, and
// the pin's system line.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { resetServerClock } from '../lib/serverClock'

const NOW = Date.parse('2026-11-09T10:00:00.000Z')
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: 'plan for tonight',
  reply_to_id: null, created_at: '2026-11-09T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const names: Record<string, string> = { U1: 'alex', U2: 'sam' }

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message, kind: 'text' | 'quotes' | 'dm' = 'text') {
  app.state = {
    ...emptyState(),
    channels: { C1: { id: 'C1', server_id: kind === 'dm' ? null : 'S1', kind, name: 'general', position: 0 } },
    messages: { C1: [m] },
  }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: 'alex', nameOf: (id: string | null) => (id ? names[id] : ''), replyTo: null,
    grouped: false, mine: true, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Message actions"]')
const items = () => [...(menu()?.querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]') ?? [])]
const item = (label: string) => items().find((b) => b.textContent?.trim() === label)
const flyout = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Pin message for"]')
const choices = () => [...(flyout()?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])]
const hover = (el: Element) => { el.dispatchEvent(new Event('pointerover', { bubbles: true })); el.dispatchEvent(new Event('pointerenter')); flushSync() }
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  invoke.mockReset(); invoke.mockResolvedValue(undefined)
  ui.goTo = null; ui.jump = null; ui.openPins = null
  resetServerClock(null)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('the message menu (#178)', () => {
  it('More on the action bar opens it with Pin message, whose flyout offers the four lengths', async () => {
    show(msg())
    btn('More')!.click(); flushSync()
    expect(items().map((b) => b.textContent?.trim())).toEqual(['Pin message'])
    hover(item('Pin message')!)
    expect(choices().map((b) => b.textContent?.trim())).toEqual(['1 hour', '1 day', '1 week', 'Forever'])
    choices()[1].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('pin_message', { messageId: 'M1', duration: 'day' })
    expect(menu()).toBeNull()
  })

  it('a right-click on the message opens it too; on the author it is still their person menu', () => {
    show(msg())
    document.querySelector('.content')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 50, clientY: 50 }))
    flushSync()
    expect(menu()).not.toBeNull()
    if (cmp) unmount(cmp)
    document.body.innerHTML = ''
    show(msg())
    document.querySelector('.meta strong')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }))
    flushSync()
    expect(menu()).toBeNull()
  })

  it('a right-click on a link, an image or selected text keeps the system menu (Copy, Copy link)', () => {
    show(msg({ content: 'see https://example.com' }))
    const content = document.querySelector<HTMLElement>('.content')!
    const a = document.createElement('a')
    a.href = 'https://example.com'
    content.append(a)
    const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 50, clientY: 50 })
    a.dispatchEvent(e)
    flushSync()
    expect(menu()).toBeNull()
    expect(e.defaultPrevented).toBe(false)
    // text selected in the message
    const r = document.createRange()
    r.selectNodeContents(content)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    const e2 = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 50, clientY: 50 })
    content.dispatchEvent(e2)
    flushSync()
    expect(menu()).toBeNull()
    expect(e2.defaultPrevented).toBe(false)
    getSelection()!.removeAllRanges()
  })

  it('the action bar stays up while its menu is open', () => {
    show(msg())
    btn('More')!.click(); flushSync()
    expect(document.querySelector('.actions')?.classList.contains('open')).toBe(true)
  })

  it('a pinned message offers Unpin message instead', async () => {
    show(msg({ pin: { pinned_by: 'U2', pinned_at: '2026-11-09T08:00:00.000Z', expires_at: null } }))
    btn('More')!.click(); flushSync()
    expect(items().map((b) => b.textContent?.trim())).toEqual(['Unpin message'])
    item('Unpin message')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('unpin_message', { messageId: 'M1' })
  })

  it('a refused pin says why under the message', async () => {
    invoke.mockImplementation(async (c: string) => { if (c === 'pin_message') throw 'This chat already has 50 pins. Unpin something first.' })
    show(msg())
    btn('More')!.click(); flushSync()
    hover(item('Pin message')!)
    choices()[0].click()
    await settle()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('This chat already has 50 pins. Unpin something first.')
  })

  it("a refused pin fetches the chat's pins, so the message shows the pin the server holds (#258)", async () => {
    const held = { pinned_by: 'U2', pinned_at: '2026-11-09T09:00:00.000Z', expires_at: null }
    invoke.mockImplementation(async (c: string) => {
      if (c === 'pin_message') throw 'That message is already pinned.'
      if (c === 'list_pins') return [{ message: msg({ pin: held }), pin: held }]
    })
    show(msg())
    btn('More')!.click(); flushSync()
    hover(item('Pin message')!)
    choices()[0].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('list_pins', { channelId: 'C1' })
    expect(app.state.messages['C1'][0].pin).toEqual(held)
  })

  it('is in DMs too, but not in a quotes channel', () => {
    show(msg(), 'dm')
    expect(btn('More')).not.toBeNull()
    if (cmp) unmount(cmp)
    document.body.innerHTML = ''
    show(msg(), 'quotes')
    expect(btn('More')).toBeNull()
  })
})

describe('the pin marker (#178)', () => {
  it('sits by the time with who pinned it and how long is left', () => {
    show(msg({ pin: { pinned_by: 'U2', pinned_at: '2026-11-09T08:00:00.000Z', expires_at: '2026-11-11T13:00:00.000Z' } }))
    const mark = document.querySelector<HTMLElement>('.meta .pin-mark')
    expect(mark?.dataset.tip).toBe('Pinned by sam · 2 days left')
    expect(mark?.querySelector('svg')?.getAttribute('width')).toBe('14')
  })

  it('reads without a time for Forever, and shows nowhere once it has run out', () => {
    show(msg({ pin: { pinned_by: 'U2', pinned_at: '2026-11-09T08:00:00.000Z', expires_at: null } }))
    expect(document.querySelector<HTMLElement>('.pin-mark')?.dataset.tip).toBe('Pinned by sam')
    if (cmp) unmount(cmp)
    document.body.innerHTML = ''
    show(msg({ pin: { pinned_by: 'U2', pinned_at: '2026-11-09T08:00:00.000Z', expires_at: '2026-11-09T09:59:00.000Z' } }))
    expect(document.querySelector('.pin-mark')).toBeNull()
  })

  it('an unpinned message has none', () => {
    show(msg())
    expect(document.querySelector('.pin-mark')).toBeNull()
  })

  it("runs out by the server's clock, not a local one running ahead (#258)", () => {
    // the server's clock is 5 minutes behind ours: a pin ending a minute ago here has 4 minutes left there
    resetServerClock(new Date(NOW - 5 * 60_000).toISOString(), NOW)
    show(msg({ pin: { pinned_by: 'U2', pinned_at: '2026-11-09T08:59:00.000Z', expires_at: '2026-11-09T09:59:00.000Z' } }))
    expect(document.querySelector<HTMLElement>('.meta .pin-mark')?.dataset.tip).toBe('Pinned by sam · 4 minutes left')
    btn('More')!.click(); flushSync()
    expect(items().map((b) => b.textContent?.trim())).toEqual(['Unpin message'])
  })
})

describe('the pin line (#178)', () => {
  const line = (duration: 'day' | 'forever') => msg({
    id: 'L1', kind: 'system', author_id: null, content: 'alex pinned a message for 1 day',
    pin_line: { message_id: 'M0', by: 'U1', duration },
  })

  it('reads with the pinner, the length and See pins', () => {
    show(line('day'))
    const el = document.querySelector<HTMLElement>('.system.pin-line')!
    expect(el.textContent?.replace(/\s+/g, ' ')).toContain('alex pinned a message for 1 day · See pins')
    if (cmp) unmount(cmp)
    document.body.innerHTML = ''
    show(line('forever'))
    expect(document.querySelector('.pin-line')?.textContent?.replace(/\s+/g, ' ')).toContain('alex pinned a message · See pins')
  })

  it('its message jumps to the pinned message; See pins opens the pins', async () => {
    show(line('day'))
    app.state = { ...app.state, messages: { C1: [msg({ id: 'M0' }), app.state.messages['C1'][0]] } }
    const [jump, see] = [...document.querySelectorAll<HTMLButtonElement>('.pin-line button')]
    expect(jump.textContent).toBe('a message')
    jump.click()
    await settle()
    expect(ui.jump).toMatchObject({ channelId: 'C1', messageId: 'M0' })
    expect(see.textContent).toBe('See pins')
    see.click()
    expect(ui.openPins).toBe('C1')
  })

  it('pins a quote in a quotes channel (#257)', () => {
    show(line('day'), 'quotes')
    expect(document.querySelector('.pin-line')?.textContent?.replace(/\s+/g, ' ')).toContain('alex pinned a quote for 1 day · See pins')
  })
})

describe('a stage share line (#234)', () => {
  it('reads with today\'s name; older clients read its content as it is', () => {
    const line = msg({ id: 'L1', author_id: null, kind: 'system', content: '🎵 sam shared a Jam to the stage', stage_share_line: { by: 'U2', kind: 'jam' } })
    names.U2 = 'samantha'
    show(line)
    expect(document.querySelector('.system')!.textContent).toContain('🎵 samantha shared a Jam to the stage')
    names.U2 = 'sam'
  })
})
