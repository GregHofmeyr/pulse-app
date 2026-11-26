// @vitest-environment jsdom
// Speaking rings (#651 §8.4): the dev-only stress harness fakes n speakers on the current stage with a synthetic level
// feed, and cleans up after itself. Release builds never load it.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
const applied = vi.hoisted(() => [] as Record<string, number>[])
vi.mock('./ringLevels.svelte', async (orig) => {
  const real = await orig<typeof import('./ringLevels.svelte')>()
  return { ...real, apply: (l: Record<string, number>) => (applied.push(l), real.apply(l)) }
})

import { install } from './ringStress'
import { voice } from './voice.svelte'
import { app } from './store.svelte'
import { emptyState } from './state'
import { userOf } from './selectors'

declare global {
  interface Window {
    __ringStress?: (n: number, style: string) => () => void
  }
}

afterEach(() => {
  vi.useRealTimers()
  voice.speaking = new Set()
  voice.channelId = null
})

describe('the ring stress harness (#651)', () => {
  it('the_harness_fakes_speakers_and_cleans_up', () => {
    vi.useFakeTimers()
    app.state = { ...emptyState(), voice: { V1: [{ user_id: 'U1', flags: { muted: false, deafened: false } }] } }
    voice.channelId = 'V1'
    install()
    const stop = window.__ringStress!(8, 'bars')
    expect(voice.speaking.size).toBe(8)
    expect(app.state.voice.V1).toHaveLength(9)
    const fake = [...voice.speaking][0]
    expect(userOf(app.state, fake)?.ring_style).toBe('bars')
    applied.length = 0
    vi.advanceTimersByTime(100)
    expect(applied.length).toBeGreaterThan(0)
    expect(Object.keys(applied[0])).toHaveLength(8)
    for (const v of Object.values(applied[0])) expect(v >= 0 && v <= 1).toBe(true)
    stop()
    expect(voice.speaking.size).toBe(0)
    expect(app.state.voice.V1).toHaveLength(1)
    expect(userOf(app.state, fake)).toBeNull()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('the_harness_is_dev_only', () => {
    const main = readFileSync(resolve(__dirname, '../main.ts'), 'utf8')
    expect(main).toMatch(/if \(import\.meta\.env\.DEV\) void import\('\.\/lib\/ringStress'\)\.then\(\(m\) => m\.install\(\)\)/)
  })
})
