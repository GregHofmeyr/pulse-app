// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import NotificationSettings from './NotificationSettings.svelte'
import Settings from './Settings.svelte'
import { notifyPrefs, setNotificationsEnabled, setNotificationText } from '../lib/notifyPrefs.svelte'
import { reloadToastPrefs, toastPrefs } from '../lib/toastPrefs.svelte'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  setNotificationsEnabled(true)
  setNotificationText(true)
  localStorage.clear()
  reloadToastPrefs()
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
    expect(switches().slice(0, 2).map((s) => s.checked)).toEqual([true, true])
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

describe('Settings → Notifications → in-app toasts (#166)', () => {
  const toggle = (label: string) => [...document.querySelectorAll('label')].find((l) => l.textContent?.trim() === label)!.querySelector<HTMLInputElement>('input[role=switch]')!

  it('a master switch and one per kind, all on by default', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    for (const l of ['Show in-app toasts', 'Someone goes live', 'Badge unlocked', 'Stream updates']) expect(toggle(l).checked, l).toBe(true)
  })

  it('switching a kind off saves it per device', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    toggle('Badge unlocked').click()
    flushSync()
    expect(toastPrefs.badge).toBe(false)
    expect(JSON.parse(localStorage.getItem('pulse.toasts')!).badge).toBe(false)
  })

  it('the kinds are disabled while the master switch is off, and keep their own settings', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    toggle('Someone goes live').click()
    toggle('Show in-app toasts').click()
    flushSync()
    expect(toastPrefs.enabled).toBe(false)
    for (const l of ['Someone goes live', 'Badge unlocked', 'Stream updates']) expect(toggle(l).disabled, l).toBe(true)
    expect(toggle('Someone goes live').checked).toBe(false)
    expect(toggle('Badge unlocked').checked).toBe(true)
  })
})
