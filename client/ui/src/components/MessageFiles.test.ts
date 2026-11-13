// @vitest-environment jsdom
// A message's files (#57): the image grid and viewer, players, file cards and the run-programs warning.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const calls: { cmd: string; args: unknown }[] = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: { messageId?: string }) => {
    calls.push({ cmd, args })
    if (cmd === 'attachment_urls') {
      return current.map((a) => ({ id: a.id, url: `https://store.test/${a.id}?X-Amz-Expires=3600`, expires_at: '2099-01-01T00:00:00.000Z' }))
    }
    if (cmd === 'save_attachment') return saveAnswer()
    return undefined
  },
}))

import MessageFiles from './MessageFiles.svelte'
import ImageViewer from './ImageViewer.svelte'
import { forgetAttachmentLinks } from '../lib/attachmentLinks'
import { downloads } from '../lib/downloads.svelte'
import type { Attachment } from '../lib/protocol/Attachment'
import type { Message } from '../lib/protocol/Message'

let current: Attachment[] = []
let saveAnswer: () => Promise<boolean> = async () => true
const att = (id: string, name: string, kind: Attachment['kind'], extra: Partial<Attachment> = {}): Attachment => ({
  id, name, kind, content_type: 'application/octet-stream', size: 2048, width: null, height: null, duration_ms: null, expires_at: null, expired: false, ...extra,
})
const message = (attachments: Attachment[]): Message => ({
  id: `M${Math.random()}`, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-11-11T00:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], attachments,
})

let cmp: ReturnType<typeof mount> | undefined
function show(atts: Attachment[]) {
  current = atts
  cmp = mount(MessageFiles, { target: document.body, props: { message: message(atts) } })
  flushSync()
}
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  flushSync()
}

beforeEach(() => {
  saveAnswer = async () => true
  calls.length = 0
  forgetAttachmentLinks()
})
afterEach(() => {
  downloads.reset()
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('MessageFiles', () => {
  it('shows one image large, its shape known before it loads', async () => {
    show([att('a', 'shot.webp', 'image', { width: 4096, height: 2048 })])
    const single = document.querySelector<HTMLElement>('.single')!
    expect(single.style.width).toBe('400px')
    expect(single.style.aspectRatio).toBe('400 / 200')
    await settle()
    expect(single.querySelector('img')!.getAttribute('src')).toBe('https://store.test/a?X-Amz-Expires=3600')
    // one request for the message's links
    expect(calls.filter((c) => c.cmd === 'attachment_urls')).toHaveLength(1)
  })

  it('shows several images in a grid and opens the viewer on the one clicked', async () => {
    show([att('a', '1.webp', 'image'), att('b', '2.webp', 'image'), att('c', '3.webp', 'image'), att('d', '4.webp', 'image')])
    await settle()
    const grid = document.querySelector<HTMLElement>('.grid')!
    expect(grid.style.getPropertyValue('--cols')).toBe('2')
    expect(grid.querySelectorAll('.tile')).toHaveLength(4)
    document.querySelector<HTMLButtonElement>('[aria-label="Open 3.webp"]')!.click()
    await settle()
    const dialog = document.querySelector('[role=dialog]')!
    expect(dialog.getAttribute('aria-label')).toBe('3.webp')
  })

  it('plays a video in the clip player, audio in an audio player', async () => {
    show([att('v', 'clip.mp4', 'video', { width: 1280, height: 720 }), att('s', 'song.mp3', 'audio')])
    await settle()
    expect(document.querySelector('[aria-label="Video player"]')).not.toBeNull()
    expect(document.querySelector('audio')!.getAttribute('src')).toBe('https://store.test/s?X-Amz-Expires=3600')
  })

  it('shows other files as cards with their size, and saves one by name', async () => {
    show([att('f', 'report.pdf', 'file', { size: 1.4 * 1024 * 1024 })])
    expect(document.querySelector('.card .name')!.textContent).toBe('report.pdf')
    expect(document.querySelector('.card .size')!.textContent).toBe('1.4 MB')
    expect(document.querySelector('.warn')).toBeNull()
    document.querySelector<HTMLButtonElement>('[aria-label="Download report.pdf"]')!.click()
    await settle()
    expect(calls.find((c) => c.cmd === 'save_attachment')!.args).toMatchObject({ attachmentId: 'f', name: 'report.pdf' })
  })

  it('a file card shows how far it has saved, cancels, and says Saved (#205)', async () => {
    let finish: (v: boolean) => void = () => {}
    saveAnswer = () => new Promise((r) => (finish = r))
    show([att('f', 'world.zip', 'file', { size: 300 * 1024 * 1024 })])
    document.querySelector<HTMLButtonElement>('[aria-label="Download world.zip"]')!.click()
    await settle()
    downloads.progress({ key: 'file:f', received: 42, total: 100 })
    flushSync()
    const button = document.querySelector<HTMLButtonElement>('.card .download')!
    expect(button.textContent?.trim()).toBe('42% · Cancel')
    expect(button.getAttribute('aria-label')).toBe('Cancel download of world.zip, 42%')
    expect(button.querySelector('.ring')).not.toBeNull()
    button.click()
    await settle()
    expect(calls.filter((c) => c.cmd === 'cancel_download').map((c) => c.args)).toEqual([{ key: 'file:f' }])
    finish(false)
    await settle()
    expect(button.textContent?.trim()).toBe('Download')
    // and again, to the end
    button.click()
    await settle()
    downloads.progress({ key: 'file:f', received: 0, total: null })
    flushSync()
    expect(button.textContent?.trim()).toBe('Cancel')
    expect(button.querySelector('.ring')!.classList.contains('spin')).toBe(true)
    finish(true)
    await settle()
    expect(button.textContent?.trim()).toBe('Saved')
    expect(button.getAttribute('aria-label')).toBe('Saved world.zip')
  })

  it('a video file shows its progress on the player\'s download icon (#205)', async () => {
    saveAnswer = () => new Promise(() => {})
    show([att('v', 'match.mp4', 'video')])
    const icon = document.querySelector<HTMLButtonElement>('button.clip-dl')!
    icon.click()
    await settle()
    downloads.progress({ key: 'file:v', received: 1, total: 4 })
    flushSync()
    expect(icon.getAttribute('aria-label')).toBe('Cancel download, 25%')
  })

  it('warns that a program can run before it is downloaded', () => {
    show([att('x', 'Setup.exe', 'file')])
    expect(document.querySelector('.warn')!.textContent).toContain('This file can run programs on your computer')
  })

  it('a temporary send says when it expires, then Expired with no Download', () => {
    const soon = new Date(Date.now() + 6 * 86_400_000 + 60_000).toISOString()
    show([att('t', 'world.zip', 'file', { expires_at: soon, size: 300 * 1024 * 1024 })])
    expect(document.querySelector('.card .size')!.textContent).toBe('300 MB · expires in 6 days')
    expect(document.querySelector('[aria-label="Download world.zip"]')).not.toBeNull()
    unmount(cmp!)
    show([att('t', 'world.zip', 'file', { expires_at: soon, expired: true })])
    expect(document.querySelector('.expired')!.textContent).toBe('Expired')
    expect(document.querySelector('[aria-label="Download world.zip"]')).toBeNull()
  })

  it('an image that won\'t load becomes a file card', async () => {
    show([att('a', 'broken.webp', 'image')])
    await settle()
    for (let i = 0; i < 2; i++) {
      document.querySelector('.single img')?.dispatchEvent(new Event('error'))
      await settle()
    }
    expect(document.querySelector('.single')).toBeNull()
    expect(document.querySelector('.card .name')!.textContent).toBe('broken.webp')
  })
})

describe('ImageViewer', () => {
  it('moves with the arrows, saves, and closes on Escape', async () => {
    const images = [att('a', '1.webp', 'image'), att('b', '2.webp', 'image')]
    current = images
    const onClose = vi.fn()
    const v = mount(ImageViewer, { target: document.body, props: { images, messageId: 'M1', onClose } })
    flushSync()
    const dialog = document.querySelector<HTMLElement>('[role=dialog]')!
    expect(dialog.getAttribute('aria-modal')).toBe('true')
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
    flushSync()
    expect(dialog.getAttribute('aria-label')).toBe('2.webp')
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
    flushSync()
    expect(dialog.getAttribute('aria-label')).toBe('1.webp')
    ;[...document.querySelectorAll('button')].find((b) => b.textContent?.includes('Save image'))!.click()
    await settle()
    expect(calls.find((c) => c.cmd === 'save_attachment')!.args).toMatchObject({ messageId: 'M1', attachmentId: 'a', name: '1.webp' })
    document.querySelector<HTMLButtonElement>('.modal-close')!.click()
    expect(onClose).toHaveBeenCalled()
    unmount(v)
  })
})
