// @vitest-environment jsdom
// Moving a server in your rail (#185): shown at once, saved, put back on a refusal.
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { app, resetState } from './store.svelte'
import { moveServerTo, railIds, stepServer } from './serverOrder.svelte'

const srv = (id: string) => ({ id, name: id, icon_hash: null })

beforeEach(() => {
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  app.state = { ...app.state, servers: [srv('A'), srv('B'), srv('C')], serverOrder: [] }
})

describe('moving a server', () => {
  it('shows the new order at once and saves every server in it', async () => {
    const done = moveServerTo('C', 0)
    expect(railIds()).toEqual(['C', 'A', 'B'])
    expect(await done).toBeNull()
    expect(invoke).toHaveBeenCalledWith('set_server_order', { serverIds: ['C', 'A', 'B'] })
  })

  it('a refusal puts the old order back and says why', async () => {
    invoke.mockRejectedValueOnce({ kind: 'rejected', message: "You're moving things too fast." })
    expect(await stepServer('A', 1)).toContain('too fast')
    expect(railIds()).toEqual(['A', 'B', 'C'])
  })

  it('a step past either end, or onto itself, saves nothing', async () => {
    await stepServer('A', -1)
    await stepServer('C', 1)
    await moveServerTo('B', 1)
    expect(invoke).not.toHaveBeenCalled()
  })
})
