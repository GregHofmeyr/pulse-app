// @vitest-environment jsdom
import { flushSync } from 'svelte'
import { describe, expect, it } from 'vitest'
import { app } from './store.svelte'
import { applyEvent, emptyState } from './state'

describe('app store', () => {
  it('holds the reducer output as-is (no deep proxy)', () => {
    const s = emptyState()
    app.state = s
    expect(app.state).toBe(s)
    expect(app.state.channels).toBe(s.channels)
  })

  it('still re-runs effects when a new state is assigned', () => {
    app.state = emptyState()
    let runs = 0
    const stop = $effect.root(() => {
      $effect(() => {
        void app.state.conn
        runs++
      })
    })
    flushSync()
    expect(runs).toBe(1)
    app.state = { ...app.state, conn: 'reconnecting' }
    flushSync()
    expect(runs).toBe(2)
    stop()
  })

  it('an unchanged slice keeps its identity across events, so slice deriveds stop there', () => {
    app.state = emptyState()
    const people = app.state.people
    app.state = applyEvent(app.state, { t: 'Typing', d: { channel_id: 'c1', user_id: 'u1' } } as never, 0)
    expect(app.state.people).toBe(people)
  })
})
