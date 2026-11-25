// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { commentsShown, panelOpen, setCommentsShown, setPanelOpen, setView, viewOf } from './buildPrefs'

beforeEach(() => localStorage.clear())

describe('buildPrefs', () => {
  it('a channel starts in the gallery with its panel open', () => {
    expect(viewOf('B1')).toBe('gallery')
    expect(panelOpen('B1')).toBe(true)
  })

  it('remembers the view and the panel per channel', () => {
    setView('B1', 'list')
    setPanelOpen('B1', false)
    expect(viewOf('B1')).toBe('list')
    expect(panelOpen('B1')).toBe(false)
    expect(viewOf('B2')).toBe('gallery')
    expect(panelOpen('B2')).toBe(true)
    expect(JSON.parse(localStorage.getItem('pulse.builds')!)).toEqual({ B1: { view: 'list', panel: false } })
  })

  it('junk in storage, or storage that throws, is the defaults', () => {
    localStorage.setItem('pulse.builds', '{nope')
    expect(viewOf('B1')).toBe('gallery')
    localStorage.setItem('pulse.builds', JSON.stringify({ B1: { view: 'wall', panel: 'yes' } }))
    expect(viewOf('B1')).toBe('gallery')
    expect(panelOpen('B1')).toBe(true)
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked') })
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked') })
    expect(viewOf('B1')).toBe('gallery')
    expect(() => setView('B1', 'list')).not.toThrow()
    get.mockRestore()
    set.mockRestore()
  })

  // #618 testing: a build page's Comments column, shown or hidden on this device (every builds channel alike)
  it('the build page shows its comments until they are hidden, and remembers that', () => {
    expect(commentsShown()).toBe(true)
    setCommentsShown(false)
    expect(commentsShown()).toBe(false)
    expect(localStorage.getItem('pulse.buildComments')).toBe('hidden')
    setCommentsShown(true)
    expect(commentsShown()).toBe(true)
  })

  it('comments read as shown when storage holds junk or throws', () => {
    localStorage.setItem('pulse.buildComments', 'nope')
    expect(commentsShown()).toBe(true)
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked') })
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked') })
    expect(commentsShown()).toBe(true)
    expect(() => setCommentsShown(false)).not.toThrow()
    get.mockRestore()
    set.mockRestore()
  })
})
