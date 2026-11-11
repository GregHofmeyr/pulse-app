// @vitest-environment jsdom
// Files in the composer (#57): the attach button, pasted files, chips and sending with only files.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const calls: { cmd: string; args: unknown; opts?: unknown }[] = []
let answers: Record<string, (args: unknown) => unknown> = {}
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: unknown, opts?: unknown) => {
    calls.push({ cmd, args, opts })
    return answers[cmd]?.(args)
  },
}))

import Composer from './Composer.svelte'
import ComposerFiles from './ComposerFiles.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { StagedFile } from '../lib/tauri'

const file = (id: string, name: string, size = 1000, extra: Partial<StagedFile> = {}): StagedFile => ({
  id, name, size, content_type: 'application/octet-stream', kind: 'file', width: null, height: null, preview: null, ...extra,
})

let cmp: ReturnType<typeof mount> | undefined
const onSend = vi.fn()
function show(extra: Record<string, unknown> = { files: true }) {
  app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } } }
  cmp = mount(Composer, {
    target: document.body,
    props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend, onTyping: () => {}, ...extra },
  })
  flushSync()
  return document.querySelector<HTMLElement>('[role="textbox"]')!
}
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  flushSync()
}
const chips = () => [...document.querySelectorAll('.chip .name')].map((n) => n.textContent)
const enter = (box: HTMLElement) => {
  box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
  flushSync()
}

beforeEach(() => {
  calls.length = 0
  answers = {}
  onSend.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ComposerFiles', () => {
  it('shows each file with its size, and removes one', () => {
    const onRemove = vi.fn()
    const c = mount(ComposerFiles, {
      target: document.body,
      props: { files: [file('a', 'report.pdf', 1.4 * 1024 * 1024), file('b', 'shot.webp', 2048, { kind: 'image', preview: 'data:image/png;base64,AA' })], onRemove },
    })
    flushSync()
    expect(chips()).toEqual(['report.pdf', 'shot.webp'])
    expect(document.querySelector('.chip .size')!.textContent).toBe('1.4 MB')
    expect(document.querySelector<HTMLImageElement>('.chip img.thumb')!.src).toContain('data:image/png')
    document.querySelector<HTMLButtonElement>('[aria-label="Remove report.pdf"]')!.click()
    expect(onRemove).toHaveBeenCalledWith('a')
    unmount(c)
  })

  it('shows upload progress without remove buttons', () => {
    const c = mount(ComposerFiles, { target: document.body, props: { files: [file('a', 'world.zip')], progress: 0.5 } })
    flushSync()
    expect(document.querySelector('[role=progressbar]')!.getAttribute('aria-valuenow')).toBe('50')
    expect(document.querySelector('.remove')).toBeNull()
    unmount(c)
  })
})

describe('Composer files', () => {
  it('has no attach button where files can\'t go', () => {
    show({})
    expect(document.querySelector('[aria-label="Attach files"]')).toBeNull()
  })

  it('picks files with the button and sends them with no text', async () => {
    answers.pick_attachment_files = () => ['/home/alex/report.pdf']
    answers.stage_attachment_paths = () => [{ status: 'staged', file: file('s1', 'report.pdf') }]
    const box = show()
    document.querySelector<HTMLButtonElement>('[aria-label="Attach files"]')!.click()
    await settle()
    expect(calls.find((c) => c.cmd === 'stage_attachment_paths')!.args).toEqual({ paths: ['/home/alex/report.pdf'] })
    expect(chips()).toEqual(['report.pdf'])
    enter(box)
    expect(onSend).toHaveBeenCalledWith('', [file('s1', 'report.pdf')])
    expect(chips()).toEqual([])
  })

  it('takes a pasted screenshot as a file, not as text', async () => {
    answers.stage_attachment_bytes = () => file('p1', 'image.webp', 500, { kind: 'image' })
    const box = show()
    const png = new File([new Uint8Array([137, 80, 78, 71])], 'image.png', { type: 'image/png' })
    const e = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent
    Object.defineProperty(e, 'clipboardData', { value: { files: [png], getData: () => '' } })
    box.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
    await settle()
    const staged = calls.find((c) => c.cmd === 'stage_attachment_bytes')!
    expect(staged.opts).toEqual({ headers: { 'x-name': 'image.png' } })
    expect(chips()).toEqual(['image.webp'])
  })

  it('says why a folder or an eleventh file can\'t be added', async () => {
    answers.stage_attachment_paths = (a) =>
      (a as { paths: string[] }).paths.map((p, i) =>
        p.endsWith('/mods') ? { status: 'refused', name: 'mods', error: 'Folders can\'t be sent as they are. Zip the folder first.' } : { status: 'staged', file: file(`f${i}`, p) },
      )
    answers.pick_attachment_files = () => ['/home/alex/mods']
    show()
    document.querySelector<HTMLButtonElement>('[aria-label="Attach files"]')!.click()
    await settle()
    expect(document.querySelector('[role=alert]')!.textContent).toContain('Zip the folder first.')

    answers.pick_attachment_files = () => Array.from({ length: 11 }, (_, i) => `f${i}.txt`)
    document.querySelector<HTMLButtonElement>('[aria-label="Attach files"]')!.click()
    await settle()
    expect(chips()).toHaveLength(10)
    expect(document.querySelector('[role=alert]')!.textContent).toBe('Up to 10 files per message.')
    // the one that didn't fit is let go
    expect(calls.filter((c) => c.cmd === 'unstage_attachment')).toHaveLength(1)
  })

  it('refuses a file over 50 MB', async () => {
    answers.pick_attachment_files = () => ['/big.zip']
    answers.stage_attachment_paths = () => [{ status: 'staged', file: file('b', 'big.zip', 51 * 1024 * 1024) }]
    show()
    document.querySelector<HTMLButtonElement>('[aria-label="Attach files"]')!.click()
    await settle()
    expect(chips()).toEqual([])
    expect(document.querySelector('[role=alert]')!.textContent).toBe('Files can be up to 50 MB.')
  })

  it('a removed chip is let go in the core', async () => {
    answers.pick_attachment_files = () => ['/a.txt']
    answers.stage_attachment_paths = () => [{ status: 'staged', file: file('s1', 'a.txt') }]
    show()
    document.querySelector<HTMLButtonElement>('[aria-label="Attach files"]')!.click()
    await settle()
    document.querySelector<HTMLButtonElement>('[aria-label="Remove a.txt"]')!.click()
    flushSync()
    expect(chips()).toEqual([])
    expect(calls.find((c) => c.cmd === 'unstage_attachment')!.args).toEqual({ id: 's1' })
  })
})
