// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import StorageSettings from './StorageSettings.svelte'
import { LOADING_DELAY_MS } from '../lib/loading'

const server = { id: 'S1', name: 'Home', icon_hash: null }
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
function mountIt() {
  cmp = mount(StorageSettings, { target: document.body, props: { server } })
  flushSync()
}
async function show() {
  mountIt()
  await vi.waitFor(() => expect(document.querySelector('p:not(.loading)')).not.toBeNull())
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

  it('says loading once the answer is slow, until it arrives', async () => {
    vi.useFakeTimers()
    try {
      invoke.mockReset()
      let done!: (v: unknown) => void
      invoke.mockReturnValue(new Promise((r) => { done = r }))
      mountIt()
      // not at once: a quick answer would only flash it
      expect(document.querySelector('.loading')).toBeNull()
      vi.advanceTimersByTime(LOADING_DELAY_MS)
      flushSync()
      expect(document.querySelector('.loading')!.textContent).toBe('Loading…')
      done({ bytes: 0, count: 0, est_monthly_usd: 0 })
      await vi.waitFor(() => expect(document.querySelector('.loading')).toBeNull())
    } finally {
      vi.useRealTimers()
    }
  })

  it('a quick answer never shows loading', async () => {
    vi.useFakeTimers()
    try {
      invoke.mockReset()
      invoke.mockResolvedValue({ bytes: 0, count: 0, est_monthly_usd: 0 })
      mountIt()
      await vi.waitFor(() => expect(document.body.textContent).toContain('0 clips'))
      vi.advanceTimersByTime(LOADING_DELAY_MS * 2)
      flushSync()
      expect(document.querySelector('.loading')).toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })

  it('a cent exactly shows the amount, just under says under a cent', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue({ bytes: 700_000_000, count: 2, est_monthly_usd: 0.01 })
    await show()
    expect(document.body.textContent).toContain("about $0.01 a month")
    unmount(cmp!)
    document.body.innerHTML = ''
    invoke.mockResolvedValue({ bytes: 600_000_000, count: 2, est_monthly_usd: 0.0099 })
    await show()
    expect(document.body.textContent).toContain("under $0.01 a month")
  })

  it('an empty server reads 0 clips', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue({ bytes: 0, count: 0, est_monthly_usd: 0 })
    await show()
    expect(document.body.textContent).toContain("0 clips · 0 B · under $0.01 a month at R2's list price")
  })
})
