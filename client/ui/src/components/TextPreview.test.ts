// @vitest-environment jsdom
// A small text file's inline preview (#318): first lines, expandable; Markdown rendered safely; HTML only ever as text.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

let answer: () => Promise<unknown> = async () => ''
const invoke = vi.fn(async (cmd: string, _args?: unknown) => (cmd === 'attachment_text' ? answer() : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import TextPreview from './TextPreview.svelte'
import MessageFiles from './MessageFiles.svelte'
import type { Attachment } from '../lib/protocol/Attachment'
import type { Message } from '../lib/protocol/Message'
import { settle } from '../lib/testSettle'

const att = (name: string, extra: Partial<Attachment> = {}): Attachment => ({
  id: 'A1', name, kind: 'file', content_type: 'text/plain', size: 600, width: null, height: null, duration_ms: null, expires_at: null, expired: false, ...extra,
})
let cmp: ReturnType<typeof mount> | undefined
function show(file: Attachment, kind: 'markdown' | 'text', text: string | null) {
  answer = async () => text
  cmp = mount(TextPreview, { target: document.body, props: { file, messageId: 'M1', kind } })
  flushSync()
}
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  invoke.mockClear()
})

describe('TextPreview', () => {
  it('reads the file through the core and shows its first 15 lines, then all of it', async () => {
    const text = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join('\n')
    show(att('log.txt'), 'text', text)
    await settle()
    expect(invoke).toHaveBeenCalledWith('attachment_text', { messageId: 'M1', attachmentId: 'A1' })
    const pre = document.querySelector('pre')!
    expect(pre.textContent).toBe(text.split('\n').slice(0, 15).join('\n'))
    const more = document.querySelector<HTMLButtonElement>('.toggle')!
    expect(more.textContent?.trim()).toBe('Show all 20 lines')
    more.click()
    flushSync()
    expect(document.querySelector('pre')!.textContent).toBe(text)
    expect(document.querySelector('.body.scroll')).not.toBeNull()
    expect(more.getAttribute('aria-expanded')).toBe('true')
  })

  it('shows HTML as its source text, never as markup', async () => {
    show(att('page.html'), 'text', '<img src=x onerror="alert(1)"><b>bold</b>')
    await settle()
    expect(document.querySelector('.preview img')).toBeNull()
    expect(document.querySelector('.preview b')).toBeNull()
    expect(document.querySelector('pre')!.textContent).toBe('<img src=x onerror="alert(1)"><b>bold</b>')
  })

  it('renders Markdown through the safe renderer (raw HTML in it stays text)', async () => {
    show(att('notes.md'), 'markdown', '# Plan\n\n- **alex** brings snacks\n\n<script>alert(1)</script>')
    await settle()
    // the message renderer's allow-list: no headings, their words stay
    expect(document.querySelector('.md')?.textContent).toContain('Plan')
    expect(document.querySelector('.md strong')?.textContent).toBe('alex')
    expect(document.querySelector('.md script')).toBeNull()
  })

  it("says it can't preview a file that isn't UTF-8 text", async () => {
    show(att('odd.txt'), 'text', null)
    await settle()
    expect(document.querySelector('.note')?.textContent).toBe("Can't preview this file.")
    expect(document.querySelector('pre')).toBeNull()
  })
})

describe('a text file in a message', () => {
  const message = (attachments: Attachment[]): Message => ({
    id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-11-11T00:00:00.000Z',
    edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], attachments,
  })
  it('previews under its card, and a spoiler only once shown', async () => {
    answer = async () => 'hello'
    cmp = mount(MessageFiles, { target: document.body, props: { message: message([att('hi.txt', { spoiler: true })]) } })
    flushSync()
    await settle()
    expect(document.querySelector('[data-text-preview]')).toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('attachment_text', expect.anything())
    document.querySelector<HTMLButtonElement>('.spoiler-cover')!.click()
    await settle()
    expect(document.querySelector('[data-text-preview] pre')?.textContent).toBe('hello')
  })
  it('leaves other files and big text files as cards only', async () => {
    cmp = mount(MessageFiles, { target: document.body, props: { message: message([att('a.zip', { id: 'A2' }), att('big.txt', { id: 'A3', size: 300 * 1024 })]) } })
    flushSync()
    await settle()
    expect(document.querySelectorAll('.card')).toHaveLength(2)
    expect(document.querySelector('[data-text-preview]')).toBeNull()
  })
})
