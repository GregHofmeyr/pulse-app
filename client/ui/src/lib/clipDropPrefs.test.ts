// @vitest-environment jsdom
// Clip drop's own save (#382): on by default, its folders kept, junk is the defaults; with no save yet, what Game
// activity's save had (on only if both its master and clips switches were), kept from then on.
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { parseSaved } from './clipDropPrefs.svelte'

describe('parseSaved', () => {
  it('on by default; junk is the defaults; junk folders go', () => {
    expect(parseSaved(null, null)).toEqual({ on: true, folders: null })
    expect(parseSaved('{oops', null)).toEqual({ on: true, folders: null })
    expect(parseSaved(JSON.stringify({ on: false, folders: ['D:\\Clips', '', 3, '  '] }), null)).toEqual({ on: false, folders: ['D:\\Clips'] })
    expect(parseSaved(JSON.stringify({ folders: 'D:\\Clips' }), null)).toEqual({ on: true, folders: null })
  })

  it('its own save wins over what Game activity had', () => {
    expect(parseSaved(JSON.stringify({ on: true, folders: null }), JSON.stringify({ enabled: false, clips: false }))).toEqual({ on: true, folders: null })
  })

  it('no save yet: taken over from Game activity, so nobody sees a change', () => {
    const games = (v: object) => parseSaved(null, JSON.stringify(v))
    expect(games({ enabled: true, clips: true, clip_folders: ['D:\\Clips'] })).toEqual({ on: true, folders: ['D:\\Clips'] })
    // Game activity off kept clip drop off before: still off (now its own switch turns it on)
    expect(games({ enabled: false, clips: true })).toEqual({ on: false, folders: null })
    expect(games({ enabled: true, clips: false })).toEqual({ on: false, folders: null })
    // an older save without the keys: each was on by default
    expect(games({ games: [] })).toEqual({ on: true, folders: null })
    expect(parseSaved(null, '{oops')).toEqual({ on: true, folders: null })
  })
})

describe('loading', () => {
  beforeEach(() => {
    vi.resetModules()
    localStorage.clear()
    invoke.mockReset()
  })

  it("takes Game activity's old setting over once and saves it as its own", async () => {
    localStorage.setItem('pulse.games', JSON.stringify({ enabled: false, clips: true, clip_folders: null }))
    const m = await import('./clipDropPrefs.svelte')
    expect(m.clipDropPrefs.on).toBe(false)
    expect(JSON.parse(localStorage.getItem('pulse.clipDrop')!)).toEqual({ on: false, folders: null })
  })

  it('a new device saves nothing until a change, then tells the core', async () => {
    const m = await import('./clipDropPrefs.svelte')
    expect(localStorage.getItem('pulse.clipDrop')).toBeNull()
    m.setClipFolders(['D:\\Clips', 'D:\\Clips', ' '])
    expect(invoke).toHaveBeenLastCalledWith('set_clip_drop', { settings: { on: true, folders: ['D:\\Clips'] } })
    m.setClipDropOn(false)
    expect(invoke).toHaveBeenLastCalledWith('set_clip_drop', { settings: { on: false, folders: ['D:\\Clips'] } })
    expect(JSON.parse(localStorage.getItem('pulse.clipDrop')!)).toEqual({ on: false, folders: ['D:\\Clips'] })
  })
})
