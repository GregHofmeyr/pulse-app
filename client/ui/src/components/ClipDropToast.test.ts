// @vitest-environment jsdom
// Auto clip drop's toast (#382): a recording the core offers shows "New clip: <file> — Share to #<channel>?"; Share
// opens that clips channel with the editor on it (the last one shared in, else the only one, else picked here) and
// Dismiss lets it go.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void>, unlistened: 0 }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.handlers[name] = cb
    return () => {
      h.unlistened++
      delete h.handlers[name]
    }
  },
}))

import ClipDropToast from './ClipDropToast.svelte'
import { clipDrop, clipsTarget, MAX_OFFERS } from '../lib/clipDrop.svelte'
import { app, ui } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testReady } from '../lib/testReady'
import type { Channel } from '../lib/protocol/Channel'

const ch = (id: string, name: string, kind: Channel['kind'] = 'clips', server_id = 'S1', position = 0): Channel => ({ id, server_id, kind, name, position }) as Channel
const settle = () => new Promise((r) => setTimeout(r, 0))
const offer = (name: string) => h.handlers['pulse://clip-drop']({ payload: { path: `C:\\Users\\alex\\Videos\\Apex\\${name}`, name } })
const toasts = () => [...document.querySelectorAll('.clip-offer')]
const button = (el: Element, text: string) => [...el.querySelectorAll('button')].find((b) => b.textContent?.trim() === text)!

let cmp: ReturnType<typeof mount> | undefined
async function show(channels: Channel[]) {
  app.state = applyReady(emptyState(), testReady(channels))
  cmp = mount(ClipDropToast, { target: document.body })
  flushSync()
  await settle()
}
beforeEach(() => {
  localStorage.clear()
  clipDrop.last = null
  clipDrop.offers = []
  ui.goTo = null
  ui.shareClip = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ClipDropToast', () => {
  it('offers a new recording to the only clips channel; Share opens it there', async () => {
    await show([ch('T1', 'general', 'text'), ch('C1', 'highlights')])
    offer('replay.mp4')
    flushSync()
    expect(toasts()).toHaveLength(1)
    expect(toasts()[0].textContent).toContain('New clip: replay.mp4 — Share to #highlights?')
    button(toasts()[0], 'Share').click()
    flushSync()
    expect(ui.shareClip).toEqual({ channelId: 'C1', path: 'C:\\Users\\alex\\Videos\\Apex\\replay.mp4' })
    expect(ui.goTo).toBe('C1')
    expect(toasts()).toHaveLength(0)
  })

  it('Dismiss lets it go and opens nothing', async () => {
    await show([ch('C1', 'highlights')])
    offer('replay.mp4')
    flushSync()
    button(toasts()[0], 'Dismiss').click()
    flushSync()
    expect(toasts()).toHaveLength(0)
    expect(ui.shareClip).toBeNull()
  })

  it('the last clips channel shared in wins; with several and none remembered, one is picked in the toast', async () => {
    await show([ch('C1', 'highlights', 'clips', 'S1', 0), ch('C2', 'fails', 'clips', 'S1', 1)])
    offer('a.mp4')
    flushSync()
    expect(toasts()[0].textContent).toContain('Share to a clips channel?')
    // the picker: the existing dropdown, listing both
    document.querySelector<HTMLButtonElement>('button[aria-label="Clips channel"]')!.click()
    flushSync()
    const opts = [...document.querySelectorAll('[role="option"]')]
    expect(opts.map((o) => o.textContent?.trim())).toEqual(['#highlights · Main', '#fails · Main'])
    opts[1].dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    flushSync()
    button(toasts()[0], 'Share').click()
    flushSync()
    expect(ui.shareClip?.channelId).toBe('C2')

    clipDrop.last = 'C1'
    offer('b.mp4')
    flushSync()
    expect(toasts()[0].textContent).toContain('Share to #highlights?')
  })

  it('no clips channel: says so, with no Share', async () => {
    await show([ch('T1', 'general', 'text')])
    offer('a.mp4')
    flushSync()
    expect(toasts()[0].textContent).toContain('Join a server with a clips channel')
    expect(button(toasts()[0], 'Share')).toBeUndefined()
  })

  it('the same file once; at most a few at a time; leaving clears them', async () => {
    await show([ch('C1', 'highlights')])
    offer('a.mp4')
    offer('a.mp4')
    for (let i = 0; i < MAX_OFFERS + 1; i++) offer(`n${i}.mp4`)
    flushSync()
    expect(toasts()).toHaveLength(MAX_OFFERS)
    expect(toasts()[0].textContent).toContain('n1.mp4')
    unmount(cmp!)
    cmp = undefined
    expect(clipDrop.offers).toEqual([])
    expect(h.handlers['pulse://clip-drop']).toBeUndefined()
  })
})

describe('clipsTarget', () => {
  const state = (channels: Channel[]) => applyReady(emptyState(), testReady(channels))
  it('a remembered channel that is gone falls back', () => {
    expect(clipsTarget(state([ch('C1', 'a')]), 'GONE')).toMatchObject({ kind: 'channel', channel: { id: 'C1' } })
    expect(clipsTarget(state([ch('C1', 'a'), ch('C2', 'b', 'clips', 'S1', 1)]), 'GONE')).toMatchObject({ kind: 'ask' })
    expect(clipsTarget(state([]), null)).toEqual({ kind: 'none' })
  })
  it("a clips channel of a server you're not in doesn't count", () => {
    expect(clipsTarget(state([ch('C1', 'a'), ch('CX', 'x', 'clips', 'S9')]), null)).toMatchObject({ kind: 'channel', channel: { id: 'C1' } })
  })
})
