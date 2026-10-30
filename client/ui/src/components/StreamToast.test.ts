// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import StreamToast from './StreamToast.svelte'
import { voice } from '../lib/voice.svelte'
import { toasts } from '../lib/stream/viewer.svelte'

let cmp: ReturnType<typeof mount> | undefined
const watchButton = () => document.querySelector<HTMLButtonElement>('.toast .watch')!

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  voice.channelId = 'V1'
  toasts.list = [{ channelId: 'V1', streamer: 'U2', name: 'sam' }]
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  toasts.list = []
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('StreamToast', () => {
  it('on_linux_watch_explains_why_it_is_unavailable', () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (X11; Linux x86_64)')
    cmp = mount(StreamToast, { target: document.body })
    flushSync()
    const b = watchButton()
    // aria-disabled, not disabled: a disabled button swallows the hover that shows the reason
    expect(b.disabled).toBe(false)
    expect(b.getAttribute('aria-disabled')).toBe('true')
    expect(b.dataset.tip).toBe("Watching from Linux isn't supported yet")
    b.click()
    flushSync()
    expect(invoke).not.toHaveBeenCalled()
  })

  it('on_windows_watch_is_a_plain_button', () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Windows NT 10.0)')
    cmp = mount(StreamToast, { target: document.body })
    flushSync()
    const b = watchButton()
    expect(b.getAttribute('aria-disabled')).toBeNull()
    expect(b.dataset.tip).toBeUndefined()
    expect(b.textContent).toBe('Watch')
  })

  it('dismiss is the X icon (not a "×" glyph) and still dismisses', () => {
    cmp = mount(StreamToast, { target: document.body })
    flushSync()
    const x = document.querySelector<HTMLButtonElement>('.toast button[aria-label="Dismiss"]')!
    expect(x.querySelector('svg[aria-hidden="true"]')).not.toBeNull()
    expect(x.textContent?.trim()).toBe('')
    x.click()
    flushSync()
    expect(toasts.list).toEqual([])
  })
})
