// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Only the profile pane asks for anything: your profile.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string) =>
    cmd === 'get_profile'
      ? {
        user: { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } },
        card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
        stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
        badges: [],
        name_archive: [],
      }
      : undefined,
}))

import Settings from './Settings.svelte'
import { motion, setAnimateMode } from '../lib/animate.svelte'
import { app, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

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
  it('Profile and Appearance sections are listed first', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {} } })
    flushSync()
    const labels = [...document.querySelectorAll('nav button')].map((b) => b.textContent)
    expect(labels.slice(0, 2)).toEqual(['Profile', 'Appearance'])
    // the default is still Voice & Audio
    expect(document.querySelector('nav button[aria-current=page]')!.textContent).toBe('Voice & Audio')
    ;([...document.querySelectorAll('nav button')][1] as HTMLButtonElement).click()
    flushSync()
    expect(document.querySelector('input[role=switch]')).not.toBeNull()
  })
})

describe('Settings: leaving the profile editor', () => {
  const alex = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const confirmText = 'Discard your profile changes?'
  const byText = (t: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === t)!
  const esc = () => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))

  async function openDirty(onClose = vi.fn()) {
    resetState()
    app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
    cmp = mount(Settings, { target: document.body, props: { onClose, section: 'profile' } })
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.preview .card')) throw new Error('not loaded')
    })
    const berry = [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Card theme"] [role=radio]')].find((b) => b.getAttribute('aria-label') === 'Berry')!
    berry.click()
    flushSync()
    return onClose
  }
  const ways: [string, () => void][] = [
    ['close button', () => document.querySelector<HTMLButtonElement>('button[aria-label="Close settings"]')!.click()],
    ['backdrop', () => document.querySelector<HTMLElement>('[data-backdrop]')!.click()],
    ['Escape', esc],
    ['another section', () => byText('Chat').click()],
  ]

  it('every way out asks when dirty', async () => {
    for (const [what, leave] of ways) {
      const onClose = await openDirty()
      leave()
      flushSync()
      expect(document.body.textContent, what).toContain(confirmText)
      expect(onClose, what).not.toHaveBeenCalled()
      // Keep editing stays, draft and all
      byText('Keep editing').click()
      flushSync()
      expect(document.body.textContent, what).not.toContain(confirmText)
      expect(document.querySelector('nav button[aria-current=page]')!.textContent, what).toBe('Profile')
      expect(document.body.textContent, what).toContain('Unsaved changes')
      // asked again, Discard leaves
      leave()
      flushSync()
      byText('Discard').click()
      flushSync()
      if (what === 'another section') {
        expect(onClose, what).not.toHaveBeenCalled()
        expect(document.querySelector('nav button[aria-current=page]')!.textContent, what).toBe('Chat')
        // nothing left to ask about: Escape now just closes
        esc()
        expect(onClose, what).toHaveBeenCalledTimes(1)
      } else {
        expect(onClose, what).toHaveBeenCalledTimes(1)
      }
      unmount(cmp!)
      cmp = undefined
      document.body.innerHTML = ''
    }
  })

  it('a clean profile pane leaves without asking', async () => {
    resetState()
    app.state = { ...emptyState(), me: alex } as never
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose, section: 'profile' } })
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.preview .card')) throw new Error('not loaded')
    })
    esc()
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(document.body.textContent).not.toContain(confirmText)
  })
})
