// @vitest-environment jsdom
// Categories (#54): a reorder shows at once as a draft over the server's layout; the saved layout replaces it, a
// refused one (someone else moved first) snaps back to the server's order with a notice.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushSync } from 'svelte'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import { drafts, REFUSED_HOLD_MS, resetSaves, saveOrder } from './layoutDraft.svelte'
import { app, ui } from './store.svelte'
import { emptyState } from './state'
import { toRequest, type Order } from './layout'
import type { Channel } from './protocol/Channel'
import type { ServerLayout } from './protocol/ServerLayout'

const ch = (id: string, position: number, category_id?: string): Channel => ({ id, server_id: 'S1', kind: 'text', name: id, position, ...(category_id ? { category_id } : {}) })
const cat = { id: 'K1', server_id: 'S1', name: 'Gaming', position: 0 }
const order: Order = { top: ['B'], cats: [{ id: 'K1', channels: ['A'] }] }
const saved: ServerLayout = {
  server_id: 'S1', revision: 8, categories: [cat],
  channels: [{ channel_id: 'B', category_id: null, position: 0 }, { channel_id: 'A', category_id: 'K1', position: 0 }],
}
function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((a, b) => ((resolve = a), (reject = b)))
  return { promise, resolve, reject }
}

beforeEach(() => {
  invoke.mockReset()
  resetSaves()
  ui.notice = null
  app.state = { ...emptyState(), channels: { A: ch('A', 0), B: ch('B', 0, 'K1') }, layouts: { S1: { revision: 7, categories: [cat] } } }
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
})
afterEach(() => {
  vi.useRealTimers()
})
/** A refused draft is held for the change that beat it (#198); nothing comes, so it goes after the hold. */
const holdPasses = () => {
  vi.advanceTimersByTime(REFUSED_HOLD_MS)
  flushSync()
}
/** Lets a refusal be handled (its promise has been answered). */
const answered = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}
/** Someone else's layout lands (a LayoutChanged): the revision moves on. */
const theirsLands = async (revision = 8) => {
  app.state = { ...app.state, layouts: { S1: { revision, categories: [cat] } } }
  flushSync()
  await Promise.resolve()
}

describe('saveOrder', () => {
  it('the draft shows at once and the saved layout replaces it', async () => {
    const d = deferred<ServerLayout>()
    invoke.mockReturnValueOnce(d.promise)
    const done = saveOrder('S1', order)
    expect(drafts.byServer['S1']).toEqual(order)
    expect(invoke).toHaveBeenCalledWith('save_layout', { serverId: 'S1', layout: toRequest(order, 7) })
    d.resolve(saved)
    await done
    expect(drafts.byServer['S1']).toBeUndefined()
    expect(app.state.layouts['S1'].revision).toBe(8)
    expect(app.state.channels['A'].category_id).toBe('K1')
    expect('category_id' in app.state.channels['B']).toBe(false)
    expect(ui.notice).toBeNull()
  })

  it('a refused save snaps back once the hold passes', async () => {
    invoke.mockRejectedValueOnce('Someone else changed the list first')
    const before = app.state
    const done = saveOrder('S1', order)
    await answered()
    expect(drafts.byServer['S1']).toEqual(order)
    holdPasses()
    await done
    expect(drafts.byServer['S1']).toBeUndefined()
    expect(app.state).toBe(before)
    expect(ui.notice).toBe('Someone else changed the list first')
  })

  it('a second save waits for the first, shows at once, and goes on the revision the first returned', async () => {
    const first = deferred<ServerLayout>()
    const second = deferred<ServerLayout>()
    invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
    const next: Order = { top: [], cats: [{ id: 'K1', channels: ['A', 'B'] }] }
    const a = saveOrder('S1', order)
    const b = saveOrder('S1', next)
    expect(drafts.byServer['S1']).toEqual(next)
    expect(invoke).toHaveBeenCalledTimes(1)
    first.resolve(saved)
    await a
    expect(drafts.byServer['S1']).toEqual(next)
    expect(invoke).toHaveBeenCalledTimes(2)
    expect(invoke).toHaveBeenLastCalledWith('save_layout', { serverId: 'S1', layout: toRequest(next, 8) })
    second.resolve({ ...saved, revision: 9, channels: [{ channel_id: 'A', category_id: 'K1', position: 0 }, { channel_id: 'B', category_id: 'K1', position: 1 }] })
    await b
    expect(drafts.byServer['S1']).toBeUndefined()
    expect(app.state.layouts['S1'].revision).toBe(9)
    expect(ui.notice).toBeNull()
  })

  it('while one is in flight only the latest order waits', async () => {
    const first = deferred<ServerLayout>()
    invoke.mockReturnValueOnce(first.promise).mockResolvedValueOnce({ ...saved, revision: 9 })
    const middle: Order = { top: ['A'], cats: [{ id: 'K1', channels: ['B'] }] }
    const last: Order = { top: [], cats: [{ id: 'K1', channels: ['A', 'B'] }] }
    const done = Promise.all([saveOrder('S1', order), saveOrder('S1', middle), saveOrder('S1', last)])
    expect(drafts.byServer['S1']).toEqual(last)
    first.resolve(saved)
    await done
    expect(invoke.mock.calls.map(([, a]) => (a as { layout: unknown }).layout)).toEqual([toRequest(order, 7), toRequest(last, 8)])
    expect(drafts.byServer['S1']).toBeUndefined()
  })

  it("a refusal drops the waiting order too: back to the server's order, with the notice", async () => {
    const first = deferred<ServerLayout>()
    invoke.mockReturnValueOnce(first.promise)
    const before = app.state
    const a = saveOrder('S1', order)
    const b = saveOrder('S1', { top: [], cats: [{ id: 'K1', channels: ['A', 'B'] }] })
    first.reject('Someone else changed the list first')
    await answered()
    holdPasses()
    await Promise.all([a, b])
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(drafts.byServer['S1']).toBeUndefined()
    expect(app.state).toBe(before)
    expect(ui.notice).toBe('Someone else changed the list first')
    // the next save starts afresh
    invoke.mockResolvedValueOnce(saved)
    await saveOrder('S1', order)
    expect(invoke).toHaveBeenLastCalledWith('save_layout', { serverId: 'S1', layout: toRequest(order, 7) })
  })

  it('an order made from an older revision goes with that revision (and is refused)', async () => {
    invoke.mockRejectedValueOnce('stale')
    await saveOrder('S1', order, 6)
    expect(invoke).toHaveBeenCalledWith('save_layout', { serverId: 'S1', layout: toRequest(order, 6) })
    // the change it missed is already here: nothing to wait for
    expect(drafts.byServer['S1']).toBeUndefined()
  })

  it('one made from the revision the save in flight was based on follows it, on its new revision', async () => {
    const first = deferred<ServerLayout>()
    invoke.mockReturnValueOnce(first.promise).mockResolvedValueOnce({ ...saved, revision: 9 })
    const next: Order = { top: [], cats: [{ id: 'K1', channels: ['A', 'B'] }] }
    const a = saveOrder('S1', order)
    const b = saveOrder('S1', next, 7)
    first.resolve(saved)
    await Promise.all([a, b])
    expect(invoke).toHaveBeenLastCalledWith('save_layout', { serverId: 'S1', layout: toRequest(next, 8) })
  })

  // #198: refused before the other person's change arrives, the list would snap back and then move again to theirs.
  // The draft stays until their layout lands (or a short hold passes), so it's one move.
  describe('a refusal that beats the change it lost to (#198)', () => {
    it('keeps the draft until their layout lands, then shows theirs', async () => {
      invoke.mockRejectedValueOnce('Someone else changed the list first')
      let settled = false
      const done = saveOrder('S1', order).then(() => (settled = true))
      await answered()
      expect(drafts.byServer['S1']).toEqual(order)
      expect(ui.notice).toBe('Someone else changed the list first')
      expect(settled).toBe(false)
      await theirsLands()
      await done
      expect(drafts.byServer['S1']).toBeUndefined()
    })

    it('their layout already here when the refusal comes: the draft goes at once', async () => {
      const d = deferred<ServerLayout>()
      invoke.mockReturnValueOnce(d.promise)
      const done = saveOrder('S1', order)
      await theirsLands()
      expect(drafts.byServer['S1']).toEqual(order)
      d.reject('Someone else changed the list first')
      await done
      expect(drafts.byServer['S1']).toBeUndefined()
    })

    it('a new move during the hold is not dropped by it', async () => {
      invoke.mockRejectedValueOnce('stale').mockReturnValueOnce(new Promise(() => {}))
      const first = saveOrder('S1', order)
      await answered()
      const next: Order = { top: [], cats: [{ id: 'K1', channels: ['A', 'B'] }] }
      void saveOrder('S1', next)
      // the new move ends the hold: the refused save has settled
      await first
      holdPasses()
      await theirsLands()
      expect(drafts.byServer['S1']).toEqual(next)
    })
  })
})
