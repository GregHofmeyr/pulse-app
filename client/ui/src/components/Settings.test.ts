// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import Settings from './Settings.svelte'
import { motion, setAnimateMode } from '../lib/animate.svelte'

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
