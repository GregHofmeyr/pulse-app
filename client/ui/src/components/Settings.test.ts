// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import Settings from './Settings.svelte'
import { closeProfileEditor, profileEditor } from '../lib/profileEditor.svelte'
import { motion, setAnimateMode } from '../lib/animate.svelte'
import { previews, setShowPreviews } from '../lib/linkPreviews.svelte'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  localStorage.clear()
  setAnimateMode('always')
  localStorage.clear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const options = () => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Animate emoji"] [role=radio]')]

describe('Settings: Chat', () => {
  it('has a Chat section with Animate emoji, Always by default (nothing stored)', async () => {
    // A fresh setting module reads the empty storage itself (beforeEach's choice doesn't count).
    localStorage.clear()
    vi.resetModules()
    const svelte = await import('svelte')
    const { default: FreshSettings } = await import('./Settings.svelte')
    const fresh = svelte.mount(FreshSettings, { target: document.body, props: { onClose: () => {} } })
    try {
      svelte.flushSync()
      const chat = [...document.querySelectorAll('nav button')].find((b) => b.textContent === 'Chat') as HTMLButtonElement
      chat.click()
      svelte.flushSync()
      expect(options().map((o) => o.textContent)).toEqual(['Always', 'On hover', 'Never'])
      expect(options().map((o) => o.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false'])
      expect(localStorage.getItem('pulse.animateEmoji')).toBeNull()
    } finally {
      svelte.unmount(fresh)
    }
  })

  it('choosing an option applies and persists it', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {}, section: 'chat' } })
    flushSync()
    options()[1].click()
    flushSync()
    expect(motion.mode).toBe('hover')
    expect(localStorage.getItem('pulse.animateEmoji')).toBe('hover')
    expect(options()[1].getAttribute('aria-checked')).toBe('true')
    options()[2].click()
    flushSync()
    expect(motion.mode).toBe('never')
    expect(localStorage.getItem('pulse.animateEmoji')).toBe('never')
  })

  it('Show link previews: On by default, Off hides them and persists', () => {
    setShowPreviews(true)
    localStorage.clear()
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {}, section: 'chat' } })
    flushSync()
    const opts = () => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Show link previews"] [role=radio]')]
    expect(opts().map((o) => o.textContent)).toEqual(['On', 'Off'])
    expect(opts()[0].getAttribute('aria-checked')).toBe('true')
    opts()[1].click()
    flushSync()
    expect(previews.show).toBe(false)
    expect(localStorage.getItem('pulse.linkPreviews')).toBe('off')
    expect(opts()[1].getAttribute('aria-checked')).toBe('true')
    setShowPreviews(true)
  })
})

describe('Settings dialog', () => {
  it('focuses into the dialog, closes on Esc, and gives focus back', () => {
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose } })
    flushSync()
    expect(document.querySelector('[role=dialog]')!.contains(document.activeElement)).toBe(true)
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(onClose).toHaveBeenCalledTimes(1)
    unmount(cmp)
    cmp = undefined
    expect(document.activeElement).toBe(opener)
  })

  it('Esc on an open Select closes the dropdown only', () => {
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose } })
    flushSync()
    const trigger = document.querySelector<HTMLButtonElement>('button[aria-label="Input device"]')!
    const key = (k: string) => trigger.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
    key('ArrowDown')
    flushSync()
    expect(document.querySelector('[role=listbox]')).not.toBeNull()
    key('Escape')
    flushSync()
    expect(document.querySelector('[role=listbox]')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
  })
})

describe('Settings sections', () => {
  it('Profile is listed first (it opens the editor), the default is still Voice & Audio', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {} } })
    flushSync()
    const labels = [...document.querySelectorAll('nav button')].map((b) => b.textContent)
    expect(labels.slice(0, 2)).toEqual(['Profile', 'Voice & Audio'])
    expect(document.querySelector('nav button[aria-current=page]')!.textContent).toBe('Voice & Audio')
  })
})

describe('Settings: Log out (#168)', () => {
  it('is the last item of the navigation, under the sections, as a danger item, and logs out', () => {
    const onLogout = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {}, onLogout } })
    flushSync()
    const nav = [...document.querySelectorAll<HTMLButtonElement>('nav button')]
    const out = nav.at(-1)!
    expect(out.textContent?.trim()).toBe('Log out')
    expect(nav.at(-2)!.textContent).toBe('About')
    expect(out.classList.contains('danger')).toBe(true)
    // set apart from the sections, not one of them
    expect(out.getAttribute('aria-current')).toBeNull()
    out.click()
    flushSync()
    expect(onLogout).toHaveBeenCalledTimes(1)
  })
})

describe('Settings: Profile and Appearance (#727)', () => {
  const navLabels = () => [...document.querySelectorAll('nav button')].map((b) => b.textContent?.trim())

  it('no_appearance_entry', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {}, section: 'voice' } })
    flushSync()
    expect(navLabels()).not.toContain('Appearance')
    expect(navLabels()).toContain('Profile')
  })

  it('profile_entry_opens_editor_and_closes_settings', async () => {
    closeProfileEditor()
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose, section: 'voice' } })
    flushSync()
    ;([...document.querySelectorAll('nav button')].find((b) => b.textContent?.trim() === 'Profile') as HTMLButtonElement).click()
    flushSync()
    await Promise.resolve()
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(profileEditor.open).toBe(true)
    expect(profileEditor.section).toBe('looks')
    closeProfileEditor()
  })
})

describe('Settings: closing (#727)', () => {
  // the profile guard moved to the profile editor with the Profile page: every way out of Settings just closes
  const ways: [string, () => void][] = [
    ['close button', () => document.querySelector<HTMLButtonElement>('button[aria-label="Close settings"]')!.click()],
    ['backdrop', () => document.querySelector<HTMLElement>('[data-backdrop]')!.click()],
    ['Escape', () => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))],
  ]
  it.each(ways)('%s closes without asking', (_what, leave) => {
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose, section: 'voice' } })
    flushSync()
    leave()
    flushSync()
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
  })
})

