// @vitest-environment jsdom
// The call's action row (#221): Go Live and Soundboard, in the voice panel at the bottom of the sidebar and in a DM or
// group call's band.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
const pick = vi.hoisted(() => vi.fn(async () => {}))
vi.mock('../lib/stream/golive.svelte', () => ({ pickSource: pick, goLiveUi: { picked: null, channelId: null, error: '' } }))

import CallActions from './CallActions.svelte'
import { handleReady, resetState } from '../lib/store.svelte'
import { voice } from '../lib/voice.svelte'
import { stream } from '../lib/stream/stream.svelte'
import { testReady } from '../lib/testReady'

const DM = { id: 'DM1', server_id: null, kind: 'dm' as const, name: null, position: 0 }
const LOUNGE = { id: 'V1', server_id: 'S1', kind: 'voice' as const, name: 'Lounge', position: 0 }
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('.call-actions button')].find((b) => b.getAttribute('aria-label') === label)
const ua = navigator.userAgent

let cmp: ReturnType<typeof mount> | undefined
const show = (channelId: string) => {
  cmp = mount(CallActions, { target: document.body, props: { channelId } })
  flushSync()
}
const onWindows = (yes: boolean) => Object.defineProperty(navigator, 'userAgent', { value: yes ? 'Mozilla/5.0 (Windows NT 10.0)' : ua, configurable: true })

beforeEach(() => {
  resetState()
  handleReady(testReady([LOUNGE, DM]))
  voice.channelId = 'V1'
  voice.connection = 'connected'
  pick.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  voice.channelId = null
  voice.connection = 'disconnected'
  stream.live = null
  onWindows(false)
  document.body.innerHTML = ''
})

describe('CallActions', () => {
  it('is one row of the shared buttons: Go Live, then Soundboard', () => {
    show('V1')
    const row = document.querySelector('.call-actions')!
    expect(row.getAttribute('role')).toBe('group')
    // icon-only buttons (owner, 2026-11-06): the name is the label and the tooltip, no visible words
    expect([...row.querySelectorAll('button')].map((b) => b.getAttribute('aria-label'))).toEqual(['Go Live', 'Soundboard'])
    for (const b of row.querySelectorAll('button')) {
      expect(b.classList.contains('btn')).toBe(true)
      expect(b.textContent?.trim()).toBe('')
      expect(b.dataset.tip).toBeTruthy()
      expect(b.querySelector('svg')!.getAttribute('width')).toBe('16')
    }
  })

  it('Go Live opens the picker on Windows', () => {
    onWindows(true)
    show('V1')
    button('Go Live')!.click()
    expect(pick).toHaveBeenCalledWith('V1')
  })

  it('Go Live says why it is off, and does nothing: on Linux, and in a DM or group call', () => {
    show('V1')
    expect(button('Go Live')!.getAttribute('aria-disabled')).toBe('true')
    expect(button('Go Live')!.dataset.tip).toBe("Streaming from Linux isn't supported yet")
    button('Go Live')!.click()
    if (cmp) unmount(cmp)
    onWindows(true)
    voice.channelId = 'DM1'
    show('DM1')
    expect(button('Go Live')!.getAttribute('aria-disabled')).toBe('true')
    expect(button('Go Live')!.dataset.tip).toBe("Going live in DM and group calls isn't here yet")
    button('Go Live')!.click()
    expect(pick).not.toHaveBeenCalled()
  })

  it('while you are live it stops your stream instead', () => {
    onWindows(true)
    stream.live = { channelId: 'V1' } as never
    show('V1')
    const stop = button('Stop streaming')!
    expect(stop.textContent?.trim()).toBe('')
    expect(stop.dataset.tip).toBe('Stop streaming')
    expect(button('Go Live')).toBeUndefined()
  })

  it('Soundboard opens the soundboard popover and toggles it shut', () => {
    show('V1')
    const sb = button('Soundboard')!
    expect(sb.getAttribute('aria-haspopup')).toBe('dialog')
    sb.click()
    flushSync()
    expect(document.querySelector('.soundboard-popover')).not.toBeNull()
    expect(sb.getAttribute('aria-expanded')).toBe('true')
    sb.click()
    flushSync()
    expect(document.querySelector('.soundboard-popover')).toBeNull()
  })

  it('leaving the call closes the soundboard', () => {
    show('V1')
    button('Soundboard')!.click()
    flushSync()
    voice.channelId = null
    flushSync()
    expect(document.querySelector('.soundboard-popover')).toBeNull()
  })
})
