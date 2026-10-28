// @vitest-environment jsdom
import { flushSync } from 'svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { app, handleReady, resetState } from './store.svelte'
import { addPending, applyEvent, emptyState, removePending, type Pending } from './state'
import type { Ready } from './protocol/Ready'
import type { Message } from './protocol/Message'

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

describe('unsent messages in the store', () => {
  const user = (id: string, username: string) => ({ id, username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
  const ready = (me = user('U1', 'alex')): Ready => ({
    me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
    dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [],
  })
  const unsent = (nonce: string, status: Pending['status'] = 'pending'): Pending => ({ nonce, content: `text ${nonce}`, reply_to_id: null, thread_root_id: null, status, created_at: '2026-10-01T00:00:00.000Z' })
  const stored = (id: string) => JSON.parse(localStorage.getItem(`pulse.unsent.${id}`) ?? 'null')

  beforeEach(() => {
    localStorage.clear()
    resetState()
    flushSync()
  })

  it('saves a new pending message', () => {
    handleReady(ready())
    app.state = addPending(app.state, 'C1', unsent('a'))
    flushSync()
    expect(stored('U1').C1.map((p: Pending) => p.nonce)).toEqual(['a'])
  })

  it('clears storage on discard and on the server echo', () => {
    handleReady(ready())
    app.state = addPending(addPending(app.state, 'C1', unsent('a')), 'C1', unsent('b'))
    flushSync()
    app.state = removePending(app.state, 'C1', 'a')
    flushSync()
    expect(stored('U1').C1.map((p: Pending) => p.nonce)).toEqual(['b'])
    const m: Message = { id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: 'x', reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null }
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: m, nonce: 'b' } }, 0)
    flushSync()
    expect(localStorage.getItem('pulse.unsent.U1')).toBeNull()
  })

  it('does not save before it has restored (an empty map must not wipe storage)', () => {
    localStorage.setItem('pulse.unsent.U1', JSON.stringify({ C1: [unsent('a')] }))
    app.state = { ...emptyState(), me: user('U1', 'alex') }
    flushSync()
    expect(stored('U1').C1).toHaveLength(1)
  })

  it('restores once: a reconnect does not bring back a discarded message', () => {
    localStorage.setItem('pulse.unsent.U1', JSON.stringify({ C1: [unsent('a')] }))
    handleReady(ready())
    expect(app.state.pending['C1'].map((p) => p.status)).toEqual(['failed'])
    app.state = removePending(app.state, 'C1', 'a')
    flushSync()
    localStorage.setItem('pulse.unsent.U1', JSON.stringify({ C1: [unsent('a')] })) // stale copy
    handleReady(ready()) // reconnect
    expect(app.state.pending['C1']).toBeUndefined()
  })

  it('restores again after sign-out and back in', () => {
    localStorage.setItem('pulse.unsent.U1', JSON.stringify({ C1: [unsent('a')] }))
    handleReady(ready())
    flushSync()
    resetState()
    flushSync()
    handleReady(ready())
    flushSync()
    expect(app.state.pending['C1'].map((p) => p.nonce)).toEqual(['a'])
    expect(stored('U1').C1).toHaveLength(1)
  })

  it('a user switch never saves one user\'s unsent under another', () => {
    handleReady(ready())
    app.state = addPending(app.state, 'C1', unsent('a'))
    flushSync()
    handleReady(ready(user('U2', 'sam'))) // no sign-out in between
    flushSync()
    expect(app.state.pending['C1']).toBeUndefined()
    expect(localStorage.getItem('pulse.unsent.U2')).toBeNull()
    expect(stored('U1').C1).toHaveLength(1)
  })
})
