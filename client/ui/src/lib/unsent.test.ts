// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { loadUnsent, saveUnsent } from './unsent'
import type { Pending } from './state'

const p = (nonce: string): Pending => ({ nonce, content: 'hello', reply_to_id: null, status: 'failed', created_at: '2026-10-01T00:00:00.000Z' })

beforeEach(() => localStorage.clear())

describe('unsent storage', () => {
  it('round_trips_per_user', () => {
    saveUnsent('U1', { C1: [p('a')] })
    saveUnsent('U2', { C2: [p('b')] })
    expect(loadUnsent('U1')).toEqual({ C1: [p('a')] })
    expect(loadUnsent('U2')).toEqual({ C2: [p('b')] })
    expect(loadUnsent('U3')).toEqual({})
  })

  it('malformed_storage_is_empty', () => {
    for (const raw of ['{nope', 'null', '[]', '"x"', '{"C1":"x"}', '{"C1":[{"nonce":1}]}']) {
      localStorage.setItem('pulse.unsent.U1', raw)
      expect(loadUnsent('U1')).toEqual({})
    }
  })

  it('empty_channels_are_not_saved', () => {
    saveUnsent('U1', { C1: [], C2: [p('a')] })
    expect(JSON.parse(localStorage.getItem('pulse.unsent.U1')!)).toEqual({ C2: [p('a')] })
    saveUnsent('U1', { C1: [] })
    expect(localStorage.getItem('pulse.unsent.U1')).toBeNull()
  })
})
