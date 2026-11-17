// @vitest-environment jsdom
// A server membership line (#418): "alex left the server" / "sam was removed", named as they're called today (the
// stored text is only for older clients).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import MessageItem from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const line = (removed_by?: string): Message => ({
  id: 'M1', channel_id: 'C1', author_id: null, kind: 'system', content: 'old name left the server',
  reply_to_id: null, created_at: '2026-11-09T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  member_line: { user_id: 'U2', ...(removed_by ? { removed_by } : {}) },
})
const names: Record<string, string> = { U1: 'alex', U2: 'sam' }

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message) {
  app.state = {
    ...emptyState(),
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } },
    messages: { C1: [m] },
  } as never
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: 'S1', name: '', nameOf: (id: string | null) => (id ? names[id] : ''), replyTo: null,
    grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('a membership line', () => {
  it('says who left, by today\'s name', () => {
    show(line())
    const el = document.querySelector('.member-line')!
    expect(el.textContent).toContain('sam left the server')
    expect(el.textContent).not.toContain('old name')
  })

  it('says who was removed', () => {
    show(line('U1'))
    expect(document.querySelector('.member-line')!.textContent).toContain('sam was removed')
  })
})
