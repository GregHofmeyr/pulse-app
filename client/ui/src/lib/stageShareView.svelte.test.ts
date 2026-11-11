// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { modeFor, reloadStageView, setMode } from './stageShareView.svelte'

describe('stage share view (#234)', () => {
  beforeEach(() => {
    localStorage.clear()
    reloadStageView()
  })

  it('a share starts as the strip, and keeps what you chose', () => {
    expect(modeFor('a')).toBe('strip')
    setMode('a', 'pill')
    expect(modeFor('a')).toBe('pill')
  })

  it('a new share shows the strip again, unless you hid the one before', () => {
    setMode('a', 'pill')
    expect(modeFor('b')).toBe('strip')
    setMode('b', 'hidden')
    expect(modeFor('c')).toBe('hidden')
  })

  it('remembered on this device', () => {
    setMode('a', 'pill')
    reloadStageView()
    expect(modeFor('a')).toBe('pill')
    localStorage.setItem('pulse.stageShare', '{"mode":"sideways"}')
    reloadStageView()
    expect(modeFor('a')).toBe('strip')
  })

  it('the corner card opens from the pill and is never kept (#288): one saved open comes back as the pill', () => {
    localStorage.setItem('pulse.stageShare', '{"mode":"corner","shareId":"a"}')
    reloadStageView()
    expect(modeFor('a')).toBe('pill')
  })
})
