// @vitest-environment jsdom
// Forward a message (#302): Forward… in its menu opens a picker of chats you can post in, with a comment; a forward
// shows where it came from, who said it and what; its header opens the original only where you can see it.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Forwarded } from '../lib/protocol/Forwarded'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'movie night at 8',
  reply_to_id: null, created_at: '2026-11-16T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const fwd = (over: Partial<Forwarded> = {}): Forwarded => ({
  message_id: 'M0', channel_id: 'C1', channel_kind: 'text', channel_name: 'general', server_id: 'S1', thread_root_id: null,
  author_id: 'U2', created_at: '2026-11-16T08:00:00.000Z', content: 'movie night at 8', ...over,
})

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message, mine = false) {
  app.state = {
    ...emptyState(),
    me: me as never,
    servers: [{ id: 'S1', name: 'Main' } as never],
    members: { S1: [{ user: me, nickname: null } as never] },
    channels: {
      C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      C2: { id: 'C2', server_id: 'S1', kind: 'text', name: 'plans', position: 1 },
      D1: { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
    },
    dmMembers: { D1: ['U1', 'U2'] },
    people: { U2: { user: { ...me, id: 'U2', username: 'sam', display_name: 'sam' } } as never },
    messages: { C1: [m] },
  }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: 'S1', name: 'sam', nameOf: (id: string | null) => (id === 'U2' ? 'sam' : 'alex'), replyTo: null,
    grouped: false, mine, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Message actions"]')
const items = () => [...(menu()?.querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]') ?? [])]
const item = (label: string) => items().find((b) => b.textContent?.trim() === label)
const picker = () => document.querySelector<HTMLElement>('[role=dialog][aria-label="Forward message"]')
const options = () => [...(picker()?.querySelectorAll<HTMLButtonElement>('[role=option]') ?? [])]
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  localStorage.clear()
  invoke.mockReset()
  invoke.mockImplementation(async () => undefined)
  ui.notice = null
  ui.goTo = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Forward… (#302)', () => {
  it('opens a picker of chats you can post in; picking one with a comment forwards it there', async () => {
    show(msg())
    btn('More')!.click(); flushSync()
    item('Forward…')!.click(); flushSync()
    expect(picker()).not.toBeNull()
    expect(picker()!.querySelector('.preview')!.textContent).toContain('movie night at 8')
    // conversations first, then the server's channels
    expect(options().map((o) => o.querySelector('.name')!.textContent)).toEqual(['sam', '#general', '#plans'])
    const search = picker()!.querySelector<HTMLInputElement>('input[type=search]')!
    search.value = 'plan'
    search.dispatchEvent(new Event('input', { bubbles: true })); flushSync()
    expect(options().map((o) => o.querySelector('.name')!.textContent)).toEqual(['#plans'])
    options()[0].click(); flushSync()
    const box = picker()!.querySelector<HTMLTextAreaElement>('textarea')!
    box.value = 'you in?'
    box.dispatchEvent(new Event('input', { bubbles: true }))
    const go = [...picker()!.querySelectorAll<HTMLButtonElement>('.foot button')].find((b) => b.textContent!.startsWith('Forward'))!
    expect(go.textContent).toBe('Forward to #plans')
    go.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('forward_message', { messageId: 'M1', channelId: 'C2', comment: 'you in?', nonce: expect.any(String) })
    expect(ui.notice).toBe('Forwarded to #plans')
    expect(picker()).toBeNull()
  })

  it('Forward stays off until a chat is picked; a refusal says why and keeps it open', async () => {
    invoke.mockImplementation(async () => { throw { kind: 'Forbidden', message: 'no' } })
    show(msg())
    btn('More')!.click(); flushSync()
    item('Forward…')!.click(); flushSync()
    const go = () => [...picker()!.querySelectorAll<HTMLButtonElement>('.foot button')].find((b) => b.textContent!.startsWith('Forward'))!
    expect(go().disabled).toBe(true)
    options()[0].click(); flushSync()
    go().click()
    await settle()
    expect(picker()!.querySelector('[role=alert]')).not.toBeNull()
  })

  it('no Forward… on a poll', () => {
    show(msg({ poll: { question: 'q', options: [], multiple: false, anonymous: false, ends_at: null, ended: false, votes: 0, results: null, my_votes: [] } as never }))
    btn('More')!.click(); flushSync()
    expect(item('Forward…')).toBeUndefined()
  })
})

describe('a forward (#302)', () => {
  it('shows the comment, then where from, who and what; the header opens the original', async () => {
    show(msg({ id: 'M5', author_id: 'U1', content: 'you in?', forwarded: fwd() }), true)
    expect(document.querySelector('.msg .content')!.textContent).toBe('you in?')
    const card = document.querySelector<HTMLElement>('.fwd')!
    expect(card.querySelector('.from')!.textContent).toBe('Forwarded from #general in Main')
    expect(card.querySelector('.by')!.textContent).toContain('sam')
    expect(card.querySelector('.content')!.textContent).toBe('movie night at 8')
    // your own forward can't be edited
    expect(btn('Edit')).toBeNull()
    card.querySelector<HTMLButtonElement>('.head')!.click()
    await settle()
    expect(ui.goTo).toBe('C1')
  })

  it('no comment: the stand-in is hidden; a DM is never named; one you can\'t see doesn\'t open', async () => {
    show(msg({ id: 'M6', content: '↪ Forwarded a message', caption_is_fallback: true, forwarded: fwd({ channel_id: 'D9', channel_kind: 'dm', channel_name: null, server_id: null }) }))
    expect(document.querySelector('.msg > .row .body > .content')).toBeNull()
    expect(document.body.textContent).not.toContain('↪ Forwarded a message')
    const card = document.querySelector<HTMLElement>('.fwd')!
    expect(card.querySelector('.from')!.textContent).toBe('Forwarded from a DM')
    card.querySelector<HTMLButtonElement>('.head')!.click()
    await settle()
    expect(ui.goTo).toBeNull()
    expect(ui.notice).toContain("can't see")
  })

  it('a deleted original shows only that it is gone', () => {
    show(msg({ id: 'M7', content: '↪ Forwarded a message', caption_is_fallback: true, forwarded: fwd({ content: '', unavailable: true, author_id: null }) }))
    const card = document.querySelector<HTMLElement>('.fwd')!
    expect(card.querySelector('.unavailable')!.textContent).toContain('no longer available')
    expect(card.querySelector('.content')).toBeNull()
  })
})
