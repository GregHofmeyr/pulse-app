// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import PatchNotes from './PatchNotes.svelte'
import type { PatchNotes as Notes } from '../patchNotes'

const notes: Notes = {
  id: 't1',
  date: '1 January 2026',
  new: ['A shiny thing', 'Another'],
  fixes: ['A broken thing works'],
  qol: [],
  message: 'Thanks, all\nsee you soon',
}

let cmp: ReturnType<typeof mount> | undefined
let onClose: ReturnType<typeof vi.fn<() => void>>
function show(n: Notes = notes) {
  onClose = vi.fn<() => void>()
  cmp = mount(PatchNotes, { target: document.body, props: { notes: n, onClose } })
  flushSync()
}
const dialog = () => document.querySelector<HTMLElement>('[role=dialog][aria-label="Patch notes"]')!
const headings = () => [...dialog().querySelectorAll('h2')].map((h) => h.textContent?.trim())

afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('PatchNotes (#207)', () => {
  it('shows the title, the date beneath it, each section as a heading and a list, and the sign-off', () => {
    show()
    expect(dialog().querySelector('h1')?.textContent).toBe('Patch notes')
    expect(dialog().querySelector('.date')?.textContent).toBe('1 January 2026')
    const sections = [...dialog().querySelectorAll('section')]
    expect(sections.map((s) => s.querySelector('h2')?.textContent)).toEqual(['New', 'Bug fixes'])
    expect([...sections[0].querySelectorAll('li')].map((l) => l.textContent)).toEqual(['A shiny thing', 'Another'])
    expect([...sections[1].querySelectorAll('li')].map((l) => l.textContent)).toEqual(['A broken thing works'])
    expect(dialog().querySelector('.message')?.textContent).toBe('Thanks, all\nsee you soon')
  })

  it('hides empty sections', () => {
    show({ ...notes, new: [], fixes: [], qol: ['Nicer'] })
    expect(headings()).toEqual(['Quality of life'])
  })

  it('Close closes it', () => {
    show()
    const close = [...dialog().querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Close')!
    close.click()
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('Escape closes it', () => {
    show()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('a backdrop click closes it', () => {
    show()
    document.querySelector<HTMLElement>('[data-backdrop]')!.click()
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
