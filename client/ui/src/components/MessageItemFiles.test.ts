// @vitest-environment jsdom
// A message with files in the list (#57): the stand-in text older versions show stays hidden; a caption shows.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Attachment } from '../lib/protocol/Attachment'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => [] }))

import MessageItem from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const pdf: Attachment = {
  id: 'A1', name: 'report.pdf', kind: 'file', content_type: 'application/octet-stream', size: 2048, width: null, height: null,
  duration_ms: null, expires_at: null, expired: false,
}
const msg = (over: Partial<Message>): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  attachments: [pdf], ...over,
})

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message) {
  app.state = { ...emptyState(), messages: { C1: [m] } }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: 'alex', nameOf: (id: string | null) => id ?? '', replyTo: null,
    grouped: false, mine: true, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('MessageItem with files', () => {
  it('hides the stand-in text and shows the file', () => {
    show(msg({ content: '📎 report.pdf', caption_is_fallback: true }))
    expect(document.querySelector('.content')).toBeNull()
    expect(document.querySelector('.card .name')!.textContent).toBe('report.pdf')
  })

  it('shows a caption above the files', () => {
    show(msg({ content: 'the numbers' }))
    expect(document.querySelector('.content')!.textContent).toBe('the numbers')
    expect(document.querySelector('.card')).not.toBeNull()
  })

  it('editing a message with only files starts with an empty box', () => {
    show(msg({ content: '📎 report.pdf', caption_is_fallback: true }))
    document.querySelector<HTMLButtonElement>('button[aria-label="Edit"]')!.click()
    flushSync()
    expect(document.querySelector('[role="textbox"][aria-label="Edit message"]')!.textContent).toBe('')
  })
})
