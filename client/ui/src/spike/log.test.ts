import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown) => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import { logResult } from './log'

const lines = () => invoke.mock.calls.map((c) => JSON.parse((c[1] as { line: string }).line))

describe('logResult', () => {
  beforeEach(() => invoke.mockClear())
  it('logs ok:true with the result', async () => {
    await logResult('S1', async () => ({ fps: 60 }))
    expect(lines()).toMatchObject([{ test: 'S1', ok: true, fps: 60 }])
  })
  it('a failed test still logs a line, with the error', async () => {
    await logResult('S8', async () => { throw new Error('picker cancelled') })
    expect(lines()).toMatchObject([{ test: 'S8', ok: false, error: 'Error: picker cancelled' }])
  })
})
