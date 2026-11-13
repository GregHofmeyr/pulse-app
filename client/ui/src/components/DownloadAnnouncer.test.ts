// @vitest-environment jsdom
// The downloads' live region (#205): polite, always there, says the start and the finish but not each percent.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import DownloadAnnouncer from './DownloadAnnouncer.svelte'
import { downloads } from '../lib/downloads.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  downloads.reset()
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('DownloadAnnouncer', () => {
  it('is a polite region, there before anything is said', () => {
    cmp = mount(DownloadAnnouncer, { target: document.body })
    const region = document.querySelector<HTMLElement>('[data-downloads]')!
    expect(region.getAttribute('aria-live')).toBe('polite')
    expect(region.classList.contains('sr-only')).toBe(true)
    expect(region.textContent).toBe('')
  })

  it('says a download started and finished, not each percent', async () => {
    cmp = mount(DownloadAnnouncer, { target: document.body })
    const region = document.querySelector<HTMLElement>('[data-downloads]')!
    let finish: (v: boolean) => void = () => {}
    const done = downloads.toggle('file:A1', 'notes.txt', () => new Promise((r) => (finish = r)), () => {})
    downloads.progress({ key: 'file:A1', received: 0, total: 100 })
    flushSync()
    expect(region.textContent).toBe('Downloading notes.txt')
    const changes: string[] = []
    const watch = new MutationObserver(() => changes.push(region.textContent ?? ''))
    watch.observe(region, { childList: true, characterData: true, subtree: true })
    for (let i = 1; i < 100; i += 10) {
      downloads.progress({ key: 'file:A1', received: i, total: 100 })
      flushSync()
    }
    await Promise.resolve()
    expect(changes).toEqual([])
    finish(true)
    await done
    flushSync()
    expect(region.textContent).toBe('Saved notes.txt')
    watch.disconnect()
  })
})
