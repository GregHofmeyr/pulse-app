// @vitest-environment jsdom
// In-app toast prefs (#166): stored per device in their own key, checked once where each kind of toast shows.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import { reloadToastPrefs, setToastPref, toastPrefs, toastShown } from './toastPrefs.svelte'
import { removeToast, showToast, toasts } from './stream/viewer.svelte'
import { badgeStore } from './badges.svelte'
import { stream } from './stream/stream.svelte'
import StreamToast from '../components/StreamToast.svelte'

beforeEach(() => {
  localStorage.clear()
  reloadToastPrefs()
})
afterEach(() => {
  removeToast('V1', 'U2')
  badgeStore.reset()
  stream.notice = ''
  stream.watchError = ''
  document.body.innerHTML = ''
  localStorage.clear()
})

describe('toast prefs', () => {
  it('everything is on with nothing stored', () => {
    expect({ ...toastPrefs }).toEqual({ enabled: true, live: true, badge: true, stream: true })
    expect(['live', 'badge', 'stream'].every((k) => toastShown(k as 'live'))).toBe(true)
  })

  it('a switch is remembered on this device under its own key', () => {
    setToastPref('badge', false)
    expect(JSON.parse(localStorage.getItem('pulse.toasts')!)).toMatchObject({ badge: false, live: true })
    toastPrefs.badge = true
    reloadToastPrefs()
    expect(toastPrefs.badge).toBe(false)
    expect(localStorage.getItem('pulse.notifications')).toBeNull()
  })

  it('the master switch silences every kind, and each kind keeps its own setting under it', () => {
    setToastPref('live', false)
    setToastPref('enabled', false)
    expect([toastShown('live'), toastShown('badge'), toastShown('stream')]).toEqual([false, false, false])
    setToastPref('enabled', true)
    expect([toastShown('live'), toastShown('badge'), toastShown('stream')]).toEqual([false, true, true])
  })

  it('a garbled value turns everything back on', () => {
    localStorage.setItem('pulse.toasts', '[1')
    reloadToastPrefs()
    expect(toastShown('badge')).toBe(true)
  })
})

describe('where toasts show', () => {
  const sam = { channelId: 'V1', streamer: 'U2', name: 'sam' }

  it('"is live" toasts follow the live switch', () => {
    setToastPref('live', false)
    showToast(sam)
    expect(toasts.list).toEqual([])
    setToastPref('live', true)
    showToast(sam)
    expect(toasts.list).toEqual([sam])
  })

  it('badge unlock toasts follow the badge switch', () => {
    setToastPref('badge', false)
    badgeStore.pushToast({ name: 'The Yapper', icon: '💬', rarity: 'common', tier: 1 })
    expect(badgeStore.toasts).toEqual([])
    setToastPref('badge', true)
    badgeStore.pushToast({ name: 'The Yapper', icon: '💬', rarity: 'common', tier: 1 })
    expect(badgeStore.toasts).toHaveLength(1)
  })

  it('stream updates follow the stream switch, but a watch error always shows', () => {
    const cmp = mount(StreamToast, { target: document.body })
    try {
      setToastPref('stream', false)
      stream.notice = 'sam stopped streaming'
      flushSync()
      expect(document.body.textContent).not.toContain('sam stopped streaming')
      stream.watchError = "Couldn't watch"
      flushSync()
      expect(document.body.textContent).toContain("Couldn't watch")
      stream.watchError = ''
      setToastPref('stream', true)
      flushSync()
      expect(document.body.textContent).toContain('sam stopped streaming')
    } finally {
      unmount(cmp)
    }
  })
})
