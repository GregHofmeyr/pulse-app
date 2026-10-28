// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import StorageSettings from './StorageSettings.svelte'

const server = { id: 'S1', name: 'Home', icon_hash: null }
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const settle = () => new Promise((r) => setTimeout(r, 0))
async function show() {
  cmp = mount(StorageSettings, { target: document.body, props: { server } })
  await settle()
  flushSync()
}

describe('StorageSettings', () => {
  it('shows count, size and cost', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue({ bytes: 3_000_000_000, count: 12, est_monthly_usd: 0.046 })
    await show()
    expect(invoke).toHaveBeenCalledWith('clip_storage', { serverId: 'S1' })
    expect(document.body.textContent).toContain('Clips')
    expect(document.body.textContent).toContain("12 clips · 3.0 GB · about $0.05 a month at R2's list price")
  })

  it('tiny totals say under a cent', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue({ bytes: 47_736_000, count: 1, est_monthly_usd: 0.0007 })
    await show()
    expect(document.body.textContent).toContain("1 clip · 47.7 MB · under $0.01 a month at R2's list price")
  })

  it('storage not set up says so', async () => {
    invoke.mockReset()
    invoke.mockRejectedValue("Clips storage isn't set up on this server.")
    await show()
    expect(document.querySelector('[role=alert]')!.textContent).toBe("Clips storage isn't set up on this server.")
  })
})
