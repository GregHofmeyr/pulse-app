// @vitest-environment jsdom
// Message sounds per person (#312): a different "New message" sound for someone, on this device.
import { beforeEach, describe, expect, it } from 'vitest'
import { loadMessageSounds, messageSoundChoices, messageSoundSet, saveMessageSounds, withMessageSound } from './messageSounds'

beforeEach(() => localStorage.clear())

describe('message sounds per person', () => {
  it('someone with their own sound plays that set; anyone else the set you picked', () => {
    const m = { U2: 'goofy' as const }
    expect(messageSoundSet(m, 'U2', 'classic')).toBe('goofy')
    expect(messageSoundSet(m, 'U3', 'minimal-wood')).toBe('minimal-wood')
  })

  it('setting and clearing one person leaves the others', () => {
    const m = withMessageSound({ U2: 'goofy' }, 'U3', 'twostep-deep')
    expect(m).toEqual({ U2: 'goofy', U3: 'twostep-deep' })
    expect(withMessageSound(m, 'U2', null)).toEqual({ U3: 'twostep-deep' })
  })

  it('kept on this device; anything odd in storage is dropped', () => {
    saveMessageSounds({ U2: 'goofy' })
    expect(loadMessageSounds()).toEqual({ U2: 'goofy' })
    localStorage.setItem('pulse.messageSounds', JSON.stringify({ U2: 'goofy', U3: 'nope', U4: 7 }))
    expect(loadMessageSounds()).toEqual({ U2: 'goofy' })
    localStorage.setItem('pulse.messageSounds', '[1]')
    expect(loadMessageSounds()).toEqual({})
    localStorage.setItem('pulse.messageSounds', '{not json')
    expect(loadMessageSounds()).toEqual({})
  })

  it('the menu: Default first (naming the set you use), then every set, the current one ticked', () => {
    const c = messageSoundChoices(null, 'minimal-wood')
    expect(c[0]).toEqual({ label: 'Default (Minimal · Wood)', set: null, checked: true })
    expect(c.slice(1).map((x) => x.label)).toEqual([
      'Classic', 'Two-step · Bright', 'Two-step · Warm', 'Two-step · Deep', 'Minimal · Soft', 'Minimal · Wood', 'Minimal · Glass', 'Goofy',
    ])
    expect(c.filter((x) => x.checked).length).toBe(1)
    const g = messageSoundChoices('goofy', 'classic')
    expect(g.find((x) => x.checked)!.set).toBe('goofy')
  })
})
