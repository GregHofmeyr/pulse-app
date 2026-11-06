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
function show(n: Notes = notes, previous: Notes | null = null) {
  onClose = vi.fn<() => void>()
  cmp = mount(PatchNotes, { target: document.body, props: { notes: n, previous, onClose } })
  flushSync()
}
const dialog = () => document.querySelector<HTMLElement>('[role=dialog][aria-label="Patch notes"]')!
const headings = () => [...dialog().querySelectorAll(':scope > section h2')].map((h) => h.textContent?.trim())
const older: Notes = {
  id: 't0',
  version: '0.1.9',
  date: '31 December 2025',
  new: ['An older shiny thing'],
  fixes: [],
  qol: ['Older nicety'],
  soon: ['A stale roadmap item'],
  message: 'Old sign-off',
}

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

  // #235: two releases can land before someone opens the notes, so the one before stays reachable.
  describe('the previous update (#235)', () => {
    const previous = () => dialog().querySelector<HTMLDetailsElement>('details.previous')

    it('sits below the current notes as one closed line naming its version', () => {
      show(notes, older)
      const p = previous()!
      expect(p).not.toBeNull()
      expect(p.open).toBe(false)
      expect(p.querySelector('summary')?.textContent?.trim()).toBe('Previous update (0.1.9)')
      // after the current notes' sign-off, before the Close button
      expect(dialog().querySelector('.message')!.compareDocumentPosition(p) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
      const close = [...dialog().querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Close')!
      expect(p.compareDocumentPosition(close) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
      // the current notes are unchanged
      expect(headings()).toEqual(['New', 'Bug fixes'])
    })

    it("opened, shows that release's date and changes, laid out like the current ones", () => {
      show(notes, older)
      const p = previous()!
      p.open = true
      flushSync()
      expect(p.querySelector('.date')?.textContent).toBe('31 December 2025')
      const sections = [...p.querySelectorAll('section')]
      expect(sections.map((s) => s.querySelector('h2')?.textContent)).toEqual(['New', 'Quality of life'])
      expect([...sections[0].querySelectorAll('li')].map((l) => l.textContent)).toEqual(['An older shiny thing'])
      // its roadmap and sign-off are out of date: left out
      expect(p.textContent).not.toContain('A stale roadmap item')
      expect(p.textContent).not.toContain('Old sign-off')
    })

    it('without one, nothing extra shows', () => {
      show(notes, null)
      expect(previous()).toBeNull()
      expect(dialog().textContent).not.toContain('Previous update')
    })
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
