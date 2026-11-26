// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { clipPanelOpen, setClipPanelOpen } from './clipPrefs'

afterEach(() => {
  localStorage.clear()
  vi.restoreAllMocks()
})

describe('clip panel open state (#663)', () => {
  it('open the first time, then remembered per channel', () => {
    expect(clipPanelOpen('C1')).toBe(true)
    setClipPanelOpen('C1', false)
    expect(clipPanelOpen('C1')).toBe(false)
    expect(clipPanelOpen('C2')).toBe(true)
    setClipPanelOpen('C1', true)
    expect(clipPanelOpen('C1')).toBe(true)
  })

  it('someone who had hidden the old filter chips starts with the panel closed, until they choose per channel', () => {
    localStorage.setItem('pulse.clipFilters', 'hidden')
    expect(clipPanelOpen('C1')).toBe(false)
    setClipPanelOpen('C1', true)
    expect(clipPanelOpen('C1')).toBe(true)
    expect(clipPanelOpen('C2')).toBe(false)
  })

  it('junk or blocked storage reads as open', () => {
    localStorage.setItem('pulse.clips', '[1,2]')
    expect(clipPanelOpen('C1')).toBe(true)
    localStorage.setItem('pulse.clips', '{nope')
    expect(clipPanelOpen('C1')).toBe(true)
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    expect(clipPanelOpen('C1')).toBe(true)
    expect(() => setClipPanelOpen('C1', false)).not.toThrow()
  })
})
