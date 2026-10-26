// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { HOVER_DELAY_MS } from '../lib/hoverPrefetch'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  vi.useFakeTimers()
  invoke.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

function render(onJoinVoice = vi.fn()) {
  const me = { id: 'U1', username: 'alex', avatar_hash: null }
  app.state = {
    ...emptyState(),
    me,
    members: { S1: [{ user: me, nickname: null }] },
    channels: { V1: { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 } },
  }
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: vi.fn(), onJoinVoice },
  })
  flushSync()
  return [...document.querySelectorAll<HTMLButtonElement>('button.row')].find((b) => b.textContent?.includes('Lounge'))!
}

const prefetches = () => invoke.mock.calls.filter(([cmd]) => cmd === 'prefetch_voice').length

describe('ChannelList voice prefetch', () => {
  it('resting on a voice channel prefetches its token', async () => {
    const row = render()
    row.dispatchEvent(new MouseEvent('mouseenter'))
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
    expect(prefetches()).toBe(1)
  })

  it('clicking before the hover delay cancels the prefetch (the join fetches its own token)', async () => {
    const onJoinVoice = vi.fn()
    const row = render(onJoinVoice)
    row.dispatchEvent(new MouseEvent('mouseenter'))
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS / 2)
    row.click()
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS * 5)
    expect(onJoinVoice).toHaveBeenCalledWith('V1')
    expect(prefetches()).toBe(0)
  })
})
