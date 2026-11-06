// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import Updating from './Updating.svelte'
import { updates } from '../lib/updates.svelte'

let cmp: ReturnType<typeof mount> | undefined
function show() {
  cmp = mount(Updating, { target: document.body })
  flushSync()
}
const bar = () => document.querySelector<HTMLElement>('[role=progressbar]')!

afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  updates.progress = null
  document.body.innerHTML = ''
})

describe('Updating (#82)', () => {
  it('says Updating Pulse… on the app background, as a window drag region', () => {
    show()
    expect(document.body.textContent).toContain('Updating Pulse…')
    expect(document.querySelector('.updating')?.hasAttribute('data-tauri-drag-region')).toBe(true)
  })

  it('a known size shows how far the download is', () => {
    updates.progress = { downloaded: 50, total: 100 }
    show()
    expect(bar().getAttribute('aria-valuenow')).toBe('50')
    expect(bar().getAttribute('aria-valuemax')).toBe('100')
    updates.progress = { downloaded: 75, total: 100 }
    flushSync()
    expect(bar().getAttribute('aria-valuenow')).toBe('75')
  })

  it('an unknown size (or no progress yet) is indeterminate: no aria-valuenow', () => {
    show()
    expect(bar().hasAttribute('aria-valuenow')).toBe(false)
    updates.progress = { downloaded: 10, total: null }
    flushSync()
    expect(bar().hasAttribute('aria-valuenow')).toBe(false)
    expect(bar().classList.contains('indeterminate')).toBe(true)
  })
})
