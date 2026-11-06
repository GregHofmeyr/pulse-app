// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import PatchNotes from './PatchNotes.svelte'
import { patchNotes, type PatchNotes as Notes } from '../patchNotes'

const notes: Notes = {
  id: 't1',
  version: '0.2.0',
  date: '1 January 2026',
  new: ['A shiny thing', 'Another'],
  fixes: ['A broken thing works'],
  qol: [],
  soon: [],
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
    expect(dialog().querySelector('.date')?.textContent).toBe('Pulse 0.2.0 · 1 January 2026')
    const sections = [...dialog().querySelectorAll('section')]
    expect(sections.map((s) => s.querySelector('h2')?.textContent)).toEqual(['New', 'Bug fixes'])
    expect([...sections[0].querySelectorAll('li')].map((l) => l.textContent)).toEqual(['A shiny thing', 'Another'])
    expect([...sections[1].querySelectorAll('li')].map((l) => l.textContent)).toEqual(['A broken thing works'])
    expect(dialog().querySelector('.message')?.textContent).toBe('Thanks, all\nsee you soon')
  })

  it('the notes show the version (#82)', () => {
    show({ ...notes, version: '0.3.1' })
    expect(dialog().querySelector('.date')?.textContent).toMatch(/Pulse 0\.3\.1/)
  })

  it('hides empty sections', () => {
    show({ ...notes, new: [], fixes: [], qol: ['Nicer'] })
    expect(headings()).toEqual(['Quality of life'])
  })

  it('shows Coming soon after Quality of life and before the sign-off, as a softer section (#224)', () => {
    show({ ...notes, qol: ['Nicer'], soon: ['A roadmap thing', 'Another one'] })
    expect(headings()).toEqual(['New', 'Bug fixes', 'Quality of life', 'Coming soon'])
    const soon = [...dialog().querySelectorAll('section')].at(-1)!
    expect(soon.classList.contains('soon')).toBe(true)
    expect([...soon.querySelectorAll('li')].map((l) => l.textContent)).toEqual(['A roadmap thing', 'Another one'])
    // The sign-off still comes last.
    expect(soon.compareDocumentPosition(dialog().querySelector('.message')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('hides Coming soon when it is empty or left out (#224)', () => {
    show({ ...notes, soon: [] })
    expect(headings()).not.toContain('Coming soon')
    unmount(cmp!)
    const { soon: _soon, ...withoutSoon } = notes
    show(withoutSoon)
    expect(headings()).not.toContain('Coming soon')
  })

  it('shows at most five Coming soon bullets (#224)', () => {
    show({ ...notes, soon: ['1', '2', '3', '4', '5', '6', '7'] })
    const soon = [...dialog().querySelectorAll('section')].at(-1)!
    expect([...soon.querySelectorAll('li')].map((l) => l.textContent)).toEqual(['1', '2', '3', '4', '5'])
  })

  it('the current notes keep Coming soon to five bullets or fewer (#224)', () => {
    expect((patchNotes.soon ?? []).length).toBeLessThanOrEqual(5)
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
