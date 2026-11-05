// @vitest-environment jsdom
// Right-click a message author's name or avatar in a server channel (#137): View profile, Message, Change server nickname,
// Change private nickname.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import MessageItem from './MessageItem.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { closeCard, profileUi } from '../lib/profileCard.svelte'

const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'hello',
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})

let cmp: ReturnType<typeof mount> | undefined
function show(serverId: string | null, authorId = 'U2') {
  const m = msg({ author_id: authorId })
  app.state = {
    ...emptyState(), me: alex,
    people: { U1: { user: alex, online: true, last_seen_at: null }, U2: { user: sam, online: true, last_seen_at: null } },
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: 'Sammy' }] },
    privateNicks: { U2: 'Bestie' },
    messages: { C1: [m] },
  } as never
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId, name: 'Sammy', nameOf: (id: string | null) => id ?? '', replyTo: null,
    grouped: false, mine: authorId === 'U1', onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const menu = () => document.querySelector<HTMLElement>('[role=menu]')
const ids = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')].map((i) => i.dataset.item)
const label = (id: string) => document.querySelector(`[role=menu] [data-item="${id}"]`)!.textContent
const pick = (id: string) => {
  document.querySelector<HTMLElement>(`[role=menu] [data-item="${id}"]`)!.click()
  flushSync()
}
const rightClick = (el: Element) => {
  const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 })
  el.dispatchEvent(e)
  flushSync()
  return e
}
const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }))
  flushSync()
}
const authorName = () => document.querySelector<HTMLElement>('.meta .name')!
const avatar = () => document.querySelector<HTMLElement>('.avbtn')!

beforeEach(() => {
  resetState()
  closeCard()
  profileUi.rename = null
  ui.goTo = null
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('a message author menu (#137)', () => {
  it('right-click the name in a server channel: View profile, Message, Change server and private nickname', () => {
    show('S1')
    const e = rightClick(authorName())
    expect(e.defaultPrevented).toBe(true)
    expect(menu()).not.toBeNull()
    expect(ids()).toEqual(['profile', 'message', 'nick', 'private-nick'])
    expect(label('nick')).toBe('Change server nickname')
    expect(label('private-nick')).toBe('Change private nickname')
    expect(document.activeElement).toBe(document.querySelector('[role=menu] [data-item="profile"]'))
  })

  it('right-click the avatar opens it too', () => {
    show('S1')
    rightClick(avatar())
    expect(ids()).toEqual(['profile', 'message', 'nick', 'private-nick'])
  })

  it('Change server nickname opens the dialog for their server nickname', () => {
    show('S1')
    rightClick(authorName())
    pick('nick')
    expect(profileUi.rename).toEqual({ kind: 'server', userId: 'U2', serverId: 'S1' })
    expect(menu()).toBeNull()
  })

  it('Change private nickname opens the dialog for your private nickname (#137)', () => {
    show('S1')
    rightClick(authorName())
    pick('private-nick')
    expect(profileUi.rename).toEqual({ kind: 'private', userId: 'U2' })
    expect(menu()).toBeNull()
  })

  it('View profile opens their card in this server', () => {
    show('S1')
    rightClick(authorName())
    pick('profile')
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
  })

  it('Message opens your DM with them', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'create_dm' ? { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 } : undefined))
    show('S1')
    rightClick(authorName())
    pick('message')
    expect(menu()).toBeNull()
    await vi.waitFor(() => expect(ui.goTo).toBe('D1'))
  })

  it('your own message: no Message item', () => {
    show('S1', 'U1')
    rightClick(authorName())
    expect(ids()).toEqual(['profile', 'nick'])
  })

  it('ContextMenu / Shift+F10 on the name opens it; Esc closes and returns focus', () => {
    show('S1')
    authorName().focus()
    key(authorName(), 'F10', { shiftKey: true })
    expect(menu()).not.toBeNull()
    key(document.activeElement!, 'Escape')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(authorName())
  })

  it('no menu in a DM or group: the browser menu is left alone', () => {
    show(null)
    const e = rightClick(authorName())
    expect(e.defaultPrevented).toBe(false)
    expect(menu()).toBeNull()
  })
})
