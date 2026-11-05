// "Notify me when they go live" (#60): the list of people, as the state keeps it (from Ready and your own events).
import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState } from './state'
import { testReady } from './testReady'
import type { Ready } from './protocol/Ready'

const changed = (user_id: string, target_id: string, on: boolean) => ({ t: 'LiveNotifyChanged' as const, d: { user_id, target_id, on } })

describe('the live-notify list', () => {
  it('comes with Ready; an older server sends none: nobody', () => {
    expect(applyReady(emptyState(), testReady([], { live_notify: ['U2', 'U3'] })).liveNotify).toEqual(['U2', 'U3'])
    const r = testReady([]) as Partial<Ready>
    delete r.live_notify
    expect(applyReady({ ...emptyState(), liveNotify: ['U9'] }, r as Ready).liveNotify).toEqual([])
  })

  it('LiveNotifyChanged adds at the end (once) and takes off', () => {
    let s = applyReady(emptyState(), testReady([]))
    s = applyEvent(s, changed('U1', 'U2', true), 0)
    s = applyEvent(s, changed('U1', 'U3', true), 0)
    s = applyEvent(s, changed('U1', 'U2', true), 0)
    expect(s.liveNotify).toEqual(['U2', 'U3'])
    s = applyEvent(s, changed('U1', 'U2', false), 0)
    expect(s.liveNotify).toEqual(['U3'])
  })

  it("someone else's list is never kept", () => {
    const s = applyEvent(applyReady(emptyState(), testReady([])), changed('U2', 'U1', true), 0)
    expect(s.liveNotify).toEqual([])
  })
})
