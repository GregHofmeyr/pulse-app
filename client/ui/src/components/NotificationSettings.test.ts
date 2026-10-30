// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import NotificationSettings from './NotificationSettings.svelte'
import Settings from './Settings.svelte'
import { notifyPrefs, setNotificationsEnabled, setNotificationText } from '../lib/notifyPrefs.svelte'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  setNotificationsEnabled(true)
  setNotificationText(true)
  localStorage.clear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  localStorage.clear()
})

const switches = () => [...document.querySelectorAll<HTMLInputElement>('input[type=checkbox][role=switch]')]

describe('Settings → Notifications', () => {
  it('both switches are on by default (nothing stored)', async () => {
    vi.resetModules()
    const fresh = await import('../lib/notifyPrefs.svelte')
    expect(fresh.notifyPrefs).toEqual({ enabled: true, showText: true })
  })

  it('a stored "false" is off', async () => {
    localStorage.setItem('pulse.notifications', 'false')
    localStorage.setItem('pulse.notificationText', 'false')
    vi.resetModules()
    const fresh = await import('../lib/notifyPrefs.svelte')
    expect(fresh.notifyPrefs).toEqual({ enabled: false, showText: false })
  })

  it('shows "Desktop notifications" and "Show message text", both on', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    expect(document.body.textContent).toContain('Desktop notifications')
    expect(document.body.textContent).toContain('Show message text')
    expect(switches().map((s) => s.checked)).toEqual([true, true])
  })

  it('switching them off is remembered on this device', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    switches()[1].click()
    flushSync()
    expect(notifyPrefs.showText).toBe(false)
    expect(localStorage.getItem('pulse.notificationText')).toBe('false')
    switches()[0].click()
    flushSync()
    expect(notifyPrefs.enabled).toBe(false)
    expect(localStorage.getItem('pulse.notifications')).toBe('false')
  })

  it('"Show message text" is disabled while notifications are off', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    expect(switches()[1].disabled).toBe(false)
    switches()[0].click()
    flushSync()
    expect(switches()[1].disabled).toBe(true)
  })

  it('Settings has a Notifications section', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {} } })
    flushSync()
    const nav = [...document.querySelectorAll('nav button')].map((b) => b.textContent)
    expect(nav).toContain('Notifications')
    const btn = [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Notifications')!
    btn.click()
    flushSync()
    expect(document.body.textContent).toContain('Desktop notifications')
  })
})
