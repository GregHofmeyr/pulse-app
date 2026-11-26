// @vitest-environment jsdom
// Clip drop's own save (#382): on by default, its folders kept, junk is the defaults; Game activity's old clip keys
// are ignored.
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { parseSaved } from './clipDropPrefs.svelte'

describe('parseSaved', () => {
  it('on by default; junk is the defaults; junk folders go', () => {
    expect(parseSaved(null)).toEqual({ on: true, folders: null })
    expect(parseSaved('{oops')).toEqual({ on: true, folders: null })
    expect(parseSaved(JSON.stringify({ on: false, folders: ['D:\\Clips', '', 3, '  '] }))).toEqual({ on: false, folders: ['D:\\Clips'] })
    expect(parseSaved(JSON.stringify({ folders: 'D:\\Clips' }))).toEqual({ on: true, folders: null })
  })
})

describe('loading', () => {
  beforeEach(() => {
    vi.resetModules()
    localStorage.clear()
    invoke.mockReset()
  })

  it("ignores Game activity's old clip keys: the defaults, nothing saved", async () => {
    localStorage.setItem('pulse.games', JSON.stringify({ enabled: false, clips: false, clip_folders: ['D:\\Old'] }))
    const m = await import('./clipDropPrefs.svelte')
    expect({ ...m.clipDropPrefs }).toEqual({ on: true, folders: null })
    expect(localStorage.getItem('pulse.clipDrop')).toBeNull()
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
