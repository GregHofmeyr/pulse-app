// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void> }))
const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.handlers[name] = cb
    return () => {}
  },
}))

import BadgeToast from './BadgeToast.svelte'
import { badgeStore, startBadgeListening } from '../lib/badges.svelte'
import { app, startListening } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { profileUi } from '../lib/profileCard.svelte'
import type { Badge } from '../lib/protocol/Badge'

const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const yapper: Badge = { key: 'the_yapper', earned_at: '2026-11-03T00:00:00Z', first_earner: false, tier: 2, rarity: 'uncommon', first_tiers: [], count: 0, hidden: false, seen: false }
const earned = (user_id = 'U1') => h.handlers['pulse://event']({ payload: { t: 'BadgeEarned', d: { user_id, badge: yapper, name: 'The Yapper', icon: '💬' } } })
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)

let cmp: ReturnType<typeof mount> | undefined
let off: (() => void) | undefined
beforeEach(async () => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  app.state = { ...emptyState(), me: alex } as never
  badgeStore.reset()
  await startListening()
  off?.()
  off = startBadgeListening()
  cmp = mount(BadgeToast, { target: document.body })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  profileUi.full = null
  document.body.innerHTML = ''
})

describe('BadgeToast', () => {
  it('a BadgeEarned shows the toast, plays the badge sound and counts it unseen', () => {
    earned()
    flushSync()
    const t = document.querySelector('.toast')!
    expect(t.textContent).toContain('Badge unlocked: The Yapper')
    expect(t.textContent).toContain('💬')
    expect(t.textContent).toContain('Uncommon')
    expect(calls('play_sound').map((c) => (c[1] as { name: string }).name)).toEqual(['badge'])
    expect(badgeStore.unseen).toBe(1)
    // your profile is refetched (the catalogue too, once it has been loaded)
    expect(calls('get_profile')).toEqual([['get_profile', { userId: 'U1' }]])
  })

  it("someone else's event is not yours", () => {
    earned('U2')
    flushSync()
    expect(document.querySelector('.toast')).toBeNull()
    expect(calls('play_sound')).toHaveLength(0)
  })

  it('clicking the toast opens your trophy case; the X dismisses', () => {
    earned()
    flushSync()
    document.querySelector<HTMLButtonElement>('.toast .open')!.click()
    flushSync()
    expect(profileUi.full).toEqual({ userId: 'U1', serverId: null })
    earned()
    flushSync()
    document.querySelector<HTMLButtonElement>('.toast button[aria-label="Dismiss"]')!.click()
    flushSync()
    expect(document.querySelector('.toast')).toBeNull()
  })

  it('fades after a while', () => {
    vi.useFakeTimers()
    try {
      earned()
      flushSync()
      expect(document.querySelector('.toast')).not.toBeNull()
      vi.advanceTimersByTime(8_000)
      flushSync()
      expect(document.querySelector('.toast')).toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })
})
