// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

async function fresh() {
  vi.resetModules()
  return import('./appearance.svelte')
}
beforeEach(() => localStorage.clear())
afterEach(() => vi.restoreAllMocks())

describe('appearance', () => {
  it('defaults on, remembers off, survives a throwing localStorage', async () => {
    let m = await fresh()
    expect(m.appearance.showNameStyles).toBe(true)
    m.setShowNameStyles(false)
    expect(m.appearance.showNameStyles).toBe(false)
    expect(localStorage.getItem('pulse.showNameStyles')).toBe('false')
    m = await fresh()
    expect(m.appearance.showNameStyles).toBe(false)

    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied') })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('denied') })
    m = await fresh()
    expect(m.appearance.showNameStyles).toBe(true)
    expect(() => m.setShowNameStyles(false)).not.toThrow()
    expect(m.appearance.showNameStyles).toBe(false)
  })
})
