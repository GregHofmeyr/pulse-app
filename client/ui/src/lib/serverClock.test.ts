import { beforeEach, describe, expect, it } from 'vitest'
import { noteServerTime, resetServerClock, serverNow } from './serverClock'

const T = Date.parse('2026-11-09T10:00:00.000Z')
const iso = (ms: number) => new Date(ms).toISOString()
const MIN = 60_000

describe('server clock (#258)', () => {
  beforeEach(() => resetServerClock(null))

  it('is the local clock until the server says otherwise', () => {
    expect(serverNow(T)).toBe(T)
  })

  it("follows Ready's time when the local clock runs ahead or behind", () => {
    // the server's clock read 10:00 when we got the Ready at a local 10:05
    resetServerClock(iso(T), T + 5 * MIN)
    expect(serverNow(T + 6 * MIN)).toBe(T + MIN)
    resetServerClock(iso(T), T - 3 * MIN)
    expect(serverNow(T - 2 * MIN)).toBe(T + MIN)
  })

  it('takes the latest stamp as the best guess: a stamp is never later than the time it arrives', () => {
    resetServerClock(iso(T), T + 5 * MIN + 2_000) // a slow Ready
    noteServerTime(iso(T + MIN), T + 6 * MIN) // a live message, quicker
    expect(serverNow(T + 7 * MIN)).toBe(T + 2 * MIN)
    noteServerTime(iso(T + MIN), T + 6 * MIN + 5_000) // a slower one changes nothing
    expect(serverNow(T + 7 * MIN)).toBe(T + 2 * MIN)
  })

  it('forgets old guesses, so a local clock that is set right again is believed', () => {
    resetServerClock(iso(T + 10 * MIN), T) // 10 minutes behind
    for (let i = 1; i <= 20; i++) noteServerTime(iso(T + i * MIN), T + i * MIN) // then set right
    expect(serverNow(T + 30 * MIN)).toBe(T + 30 * MIN)
  })

  it('ignores a time it cannot read, and an older server sends none', () => {
    resetServerClock(undefined, T + MIN)
    noteServerTime('not a time', T)
    expect(serverNow(T)).toBe(T)
  })
})
