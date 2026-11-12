// @vitest-environment jsdom
// "Add to quotes…" in the message menu (#316): the message becomes a quote (its author speaking it) in a quotes
// channel of its server, the only one or one picked from a flyout; never in a DM or group.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Channel } from '../lib/protocol/Channel'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'the floor is lava',
  reply_to_id: null, created_at: '2026-11-09T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const quotes = (id: string, name: string, position: number): Channel => ({ id, server_id: 'S1', kind: 'quotes', name, position })

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message, opts: { dm?: boolean; extra?: Channel[] } = {}) {
  const here: Channel = { id: 'C1', server_id: opts.dm ? null : 'S1', kind: opts.dm ? 'dm' : 'text', name: 'general', position: 0 }
  app.state = {
    ...emptyState(),
    me: user('U1', 'alex'),
    channels: Object.fromEntries([here, ...(opts.extra ?? [])].map((c) => [c.id, c])),
    members: { S1: [{ user: user('U1', 'alex'), nickname: null }, { user: user('U2', 'sam'), nickname: null }] },
    messages: { C1: [m] },
  } as never
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: opts.dm ? null : 'S1', name: 'sam', nameOf: (id: string | null) => (id === 'U2' ? 'sam' : 'alex'), replyTo: null,
    grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const more = () => { document.querySelector<HTMLButtonElement>('button[aria-label="More"]')!.click(); flushSync() }
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Message actions"]')
const items = () => [...(menu()?.querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]') ?? [])].map((b) => b.textContent?.trim())
const item = (label: string) => [...(menu()?.querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]') ?? [])].find((b) => b.textContent?.trim() === label)
const hover = (el: Element) => { el.dispatchEvent(new Event('pointerover', { bubbles: true })); el.dispatchEvent(new Event('pointerenter')); flushSync() }
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  invoke.mockReset(); invoke.mockResolvedValue(undefined)
  ui.notice = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Add to quotes (#316)', () => {
  it('with one quotes channel: Add to quotes posts the message as a quote by its author there', async () => {
    show(msg(), { extra: [quotes('Q1', 'quotes', 1)] })
    more()
    expect(items()).toEqual(['Pin message', 'Add to quotes'])
    item('Add to quotes')!.click()
    await settle()
    const call = invoke.mock.calls.find(([c]) => c === 'post_quote')!
    expect(call[1]).toMatchObject({
      channelId: 'Q1',
      body: { lines: [{ id: null, speaker_id: 'U2', speaker_name: null, text: 'the floor is lava', media_hash: null, caption: null }] },
    })
    expect(menu()).toBeNull()
    expect(ui.notice).toBe('Added to #quotes')
  })

  it('with several: a flyout to pick which', async () => {
    show(msg(), { extra: [quotes('Q2', 'old-quotes', 5), quotes('Q1', 'quotes', 1)] })
    more()
    hover(item('Add to quotes…')!)
    const flyout = document.querySelector<HTMLElement>('[role=menu][aria-label="Add to quotes in"]')!
    const choices = [...flyout.querySelectorAll<HTMLButtonElement>('[role=menuitem]')]
    expect(choices.map((b) => b.textContent?.trim())).toEqual(['#quotes', '#old-quotes'])
    choices[1].click()
    await settle()
    expect(invoke.mock.calls.find(([c]) => c === 'post_quote')![1]).toMatchObject({ channelId: 'Q2' })
  })

  it('a refusal says why', async () => {
    invoke.mockImplementation(async (cmd) => { if (cmd === 'post_quote') throw 'a line must be 1-4000 characters' })
    show(msg(), { extra: [quotes('Q1', 'quotes', 1)] })
    more()
    item('Add to quotes')!.click()
    await settle()
    expect(document.body.textContent).toContain('a line must be 1-4000 characters')
  })

  it('not in a DM (no quotes channels), nor where the server has none, nor on a message with nothing to quote', () => {
    show(msg(), { dm: true })
    more()
    expect(items()).not.toContain('Add to quotes')
    unmount(cmp!); document.body.innerHTML = ''
    show(msg())
    more()
    expect(items()).toEqual(['Pin message'])
    unmount(cmp!); document.body.innerHTML = ''
    show(msg({ content: '', caption_is_fallback: true }), { extra: [quotes('Q1', 'quotes', 1)] })
    more()
    expect(items()).toEqual(['Pin message'])
  })
})
