// @vitest-environment jsdom
// Block follow-ups (#508), the app side: your DM with someone you've blocked shows a quiet notice instead of the
// composer, with Unblock in their right-click menu; and reaction events keep who reacted, so Hide completely can leave
// them out of the count.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'list_messages' ? [] : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
Element.prototype.scrollTo ??= () => {}

import TextChannel from './TextChannel.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState, type AppState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'
import type { Message } from '../lib/protocol/Message'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
let cmp: ReturnType<typeof mount> | undefined
const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve()
  flushSync()
}

beforeEach(() => {
  resetState()
  invoke.mockClear()
  app.state = applyReady(emptyState(), testReady([{ id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 }], {
    dm_members: [{ channel_id: 'D1', user_ids: [testMe.id, 'U2'] }],
    people: [{ user: sam, online: true, last_seen_at: null, status: 'online' }],
  } as never))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe("your DM with someone you've blocked", () => {
  const open = () => {
    cmp = mount(TextChannel, { target: document.body, props: { channelId: 'D1', serverId: null } })
    flushSync()
  }
  const note = () => document.querySelector<HTMLElement>('.blocked-note')

  it('says so instead of the composer, and comes back on unblock', () => {
    app.state = { ...app.state, blocks: [{ user_id: 'U2', mode: 'line' }] }
    open()
    expect(note()!.textContent).toBe('You blocked sam. Unblock to message.')
    expect(document.querySelector('.composer-input')).toBeNull()
    app.state = applyEvent(app.state, { t: 'BlocksChanged', d: { user_id: testMe.id, list: [] } }, 0)
    flushSync()
    expect(note()).toBeNull()
    expect(document.querySelector('.composer-input')).not.toBeNull()
  })

  it('Unblock is in their right-click menu on the notice (no button)', async () => {
    app.state = { ...app.state, blocks: [{ user_id: 'U2', mode: 'hide' }] }
    open()
    expect(note()!.querySelector('button')).toBeNull()
    note()!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    await settle()
    const unblock = document.querySelector<HTMLButtonElement>('[role=menu] [data-item="unblock"]')!
    expect(unblock).not.toBeNull()
    invoke.mockResolvedValueOnce([])
    unblock.click()
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'set_block').map(([, a]) => a)).toEqual([{ userId: 'U2', mode: null }])
  })
})

describe('reaction events keep who reacted', () => {
  it('adds and removes the reactor, and a new chip starts with them', () => {
    const m = {
      id: 'M1', channel_id: 'D1', author_id: 'U2', kind: 'normal', content: 'hi', reply_to_id: null, created_at: '2026-11-19T10:00:00.000Z',
      edited_at: null, deleted: false, mentions: [], reactions: [{ emoji: '👍', count: 1, me: false, users: ['U2'] }],
    } as unknown as Message
    let s: AppState = { ...app.state, messages: { D1: [m] } }
    const ev = (t: 'ReactionAdded' | 'ReactionRemoved', user_id: string, count: number, emoji = '👍') =>
      ({ t, d: { channel_id: 'D1', message_id: 'M1', emoji, user_id, count } }) as const
    s = applyEvent(s, ev('ReactionAdded', 'U3', 2), 0)
    s = applyEvent(s, ev('ReactionAdded', 'U3', 1, '😂'), 0)
    expect(s.messages.D1[0].reactions.map((r) => r.users)).toEqual([['U2', 'U3'], ['U3']])
    s = applyEvent(s, ev('ReactionRemoved', 'U2', 1), 0)
    expect(s.messages.D1[0].reactions[0].users).toEqual(['U3'])
  })
})
