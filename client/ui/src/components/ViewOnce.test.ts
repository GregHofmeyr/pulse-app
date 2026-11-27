// @vitest-environment jsdom
// View-once files (#311): a recipient gets a card, never a link from the usual route; opening shows them once in a
// dialog that says it isn't screenshot-proof, then the card says Opened; the sender sees them as usual and who opened
// them (live); the composer marks pictures, videos and sounds view once, and the send carries the mark.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageFiles from './MessageFiles.svelte'
import ComposerFiles from './ComposerFiles.svelte'
import { forgetAttachmentLinks } from '../lib/attachmentLinks'
import { app, queueMessage, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { noteViewOnceEvent, resetViewOnce, viewOnce } from '../lib/viewOnce.svelte'
import type { Attachment } from '../lib/protocol/Attachment'
import type { Message } from '../lib/protocol/Message'
import { settle } from '../lib/testSettle'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const att = (id: string, kind: Attachment['kind'], extra: Partial<Attachment> = {}): Attachment => ({
  id, name: `${id}.bin`, kind, content_type: 'application/octet-stream', size: 2048, width: 100, height: 100, duration_ms: null, expires_at: null, expired: false, ...extra,
})
const message = (author: string, attachments: Attachment[]): Message => ({
  id: 'M1', channel_id: 'C1', author_id: author, kind: 'normal', content: '', reply_to_id: null, created_at: '2026-11-11T00:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], attachments,
})
const secret = att('A1', 'image', { view_once: true })
const plain = att('A2', 'image')

let cmp: ReturnType<typeof mount> | undefined
let opened = false
function show(author: string, atts: Attachment[] = [secret, plain]) {
  cmp = mount(MessageFiles, { target: document.body, props: { message: message(author, atts) } })
  flushSync()
}
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a)

beforeEach(() => {
  resetState()
  resetViewOnce()
  forgetAttachmentLinks()
  opened = false
  app.state = { ...emptyState(), me: user('U1', 'alex'), people: { U2: { user: user('U2', 'sam'), online: true, last_seen_at: null } } } as never
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'attachment_urls') return [{ id: 'A1', url: null, expires_at: null }, { id: 'A2', url: 'https://store.test/A2', expires_at: '2099-01-01T00:00:00.000Z' }]
    if (cmd === 'view_once_state') return { opened, viewers: [] }
    if (cmd === 'open_view_once') {
      if (opened) throw { kind: 'rejected', message: 'gone' }
      opened = true
      return [{ id: 'A1', url: 'https://store.test/A1?response-cache-control=private%2C%20no-store', expires_at: '2099-01-01T00:00:00.000Z' }]
    }
    return undefined
  })
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('a recipient', () => {
  it('gets a card, not the picture; opening shows it once with the warning; then it says Opened', async () => {
    show('U2')
    await settle(8)
    // only the plain picture is drawn
    expect([...document.querySelectorAll('img')].map((i) => i.getAttribute('src'))).toEqual(['https://store.test/A2'])
    const card = document.querySelector<HTMLButtonElement>('[data-view-once]')!
    expect(card.textContent).toContain('View once photo')
    card.click()
    await settle(8)
    expect(calls('open_view_once')).toEqual([{ messageId: 'M1' }])
    const dialog = document.querySelector<HTMLElement>('[role=dialog][aria-label="View once"]')!
    expect(dialog.textContent).toContain("isn't screenshot-proof")
    expect(dialog.querySelector('img')!.getAttribute('src')).toContain('A1')
    ;[...dialog.querySelectorAll('button')].find((b) => b.textContent === 'Close')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(document.querySelector('[data-view-once]')!.textContent).toContain('Opened')
    expect(document.querySelector('button[data-view-once]')).toBeNull()
  })

  it('opened before (another device): Opened from the start', async () => {
    opened = true
    show('U2')
    await settle(8)
    expect(document.querySelector('[data-view-once]')!.textContent).toContain('Opened')
  })
})

describe('the sender', () => {
  it('sees it as usual, and who opened it, live', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'attachment_urls') return [{ id: 'A1', url: 'https://store.test/A1', expires_at: '2099-01-01T00:00:00.000Z' }, { id: 'A2', url: 'https://store.test/A2', expires_at: '2099-01-01T00:00:00.000Z' }]
      if (cmd === 'view_once_state') return { opened: false, viewers: [] }
      return undefined
    })
    show('U1')
    await settle(8)
    expect(document.querySelectorAll('img')).toHaveLength(2)
    const line = () => document.querySelector('[data-view-once-status]')!.textContent
    expect(line()).toContain('Not opened yet')
    noteViewOnceEvent({ t: 'ViewOnceOpened', d: { user_id: 'U1', channel_id: 'C1', message_id: 'M1', viewer_id: 'U2', viewed_at: '2026-11-11T00:01:00.000Z' } })
    flushSync()
    expect(line()).toContain('Opened by sam')
    expect(viewOnce.states.M1.viewers).toHaveLength(1)
  })
})

describe('the composer', () => {
  it('offers View once on pictures, videos and sounds, not other files', () => {
    const toggled: string[] = []
    cmp = mount(ComposerFiles, { target: document.body, props: {
      files: [{ id: 'S1', name: 'a.webp', size: 1, kind: 'image', preview: null }, { id: 'S2', name: 'b.pdf', size: 1, kind: 'file', preview: null }],
      onToggleViewOnce: (id: string) => toggled.push(id),
    } })
    flushSync()
    const marks = [...document.querySelectorAll<HTMLButtonElement>('.once')]
    expect(marks).toHaveLength(1)
    marks[0].click()
    expect(toggled).toEqual(['S1'])
  })

  it('a send carries the marked files', () => {
    queueMessage('C1', '', null, null, [{ id: 'S1', name: 'a.webp', size: 1, kind: 'image', preview: null, width: null, height: null, viewOnce: true }, { id: 'S2', name: 'b.webp', size: 1, kind: 'image', preview: null, width: null, height: null }] as never)
    const sent = calls('send_with_attachments') as { viewOnce?: string[] }[]
    expect(sent[0].viewOnce).toEqual(['S1'])
  })
})
