// @vitest-environment jsdom
// A timed DND flips back to Online exactly when it ends (#45), by itself: the shared clock moves to the end time, so
// every dot drawn from presenceOf(…, clock.nowIso) updates with no round trip to the server.
import { flushSync } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { app, clock, handleReady, resetState } from './store.svelte'
import { applyEvent } from './state'
import { presenceOf } from './presence'
import { testReady } from './testReady'

const START = Date.UTC(2026, 10, 5, 10)
const mine = () => presenceOf(app.state, 'U1', clock.nowIso).kind

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
  vi.setSystemTime(START)
  clock.nowIso = new Date(START).toISOString()
  resetState()
})
afterEach(() => {
  resetState()
  flushSync()
  vi.useRealTimers()
})

describe('the DND timer', () => {
  it('flips your dot back to Online at the end time, not before', () => {
    handleReady(testReady([], { status: { status: 'dnd', dnd_until: new Date(START + 3_600_000).toISOString() } }))
    flushSync()
    expect(mine()).toBe('dnd')
    vi.advanceTimersByTime(3_600_000 - 1)
    flushSync()
    expect(mine()).toBe('dnd')
    vi.advanceTimersByTime(1)
    flushSync()
    expect(mine()).toBe('online')
  })

  it('a new status replaces the old timer', () => {
    handleReady(testReady([], { status: { status: 'dnd', dnd_until: new Date(START + 60_000).toISOString() } }))
    flushSync()
    // changed on another device: DND until turned off
    app.state = applyEvent(app.state, { t: 'StatusChanged', d: { user_id: 'U1', status: { status: 'dnd', dnd_until: null } } }, Date.now())
    flushSync()
    vi.advanceTimersByTime(120_000)
    flushSync()
    expect(mine()).toBe('dnd')
  })
})
