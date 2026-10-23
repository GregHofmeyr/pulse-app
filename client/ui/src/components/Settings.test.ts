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
  it('has a Chat section with Animate emoji, Always by default', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {} } })
    flushSync()
    const chat = [...document.querySelectorAll('nav button')].find((b) => b.textContent === 'Chat') as HTMLButtonElement
    chat.click()
    flushSync()
    expect(options().map((o) => o.textContent)).toEqual(['Always', 'On hover', 'Never'])
    expect(options().map((o) => o.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false'])
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
