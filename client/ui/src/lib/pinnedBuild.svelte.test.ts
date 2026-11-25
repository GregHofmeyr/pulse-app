// @vitest-environment jsdom
// Pin to side's store (#619): saved per account, fetched when no copy is given, and an older fetch never wins.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from './protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { forgetPinnedBuild, isPinned, pinBuild, pinnedBuild, unpinBuild } from './pinnedBuild.svelte'
import { handleReady, resetState } from './store.svelte'
import { testReady } from './testReady'

const B1 = { id: 'B1', server_id: 'S1', kind: 'builds' as const, name: 'raven-builds', position: 0 }
const build = (id: string, title = `Build ${id}`): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, embeds: [], clip: null,
  build: { title, title_auto: false, game_id: null, game_guessed: false, link: null, notes: null, quick_drop: false, updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null },
})
const settle = async () => { for (let i = 0; i < 10; i++) await Promise.resolve() }

beforeEach(() => {
  localStorage.clear()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  forgetPinnedBuild()
  resetState()
  handleReady(testReady([B1]))
})

describe('pinnedBuild', () => {
  it('saves the pin with its account, and unpinning clears it', () => {
    pinBuild('B1', 'M001', build('M001'))
    expect(JSON.parse(localStorage.getItem('pulse.pinnedBuild')!)).toEqual({ userId: 'U1', channelId: 'B1', messageId: 'M001' })
    expect(isPinned('M001')).toBe(true)
    expect(invoke).not.toHaveBeenCalled()
    unpinBuild()
    expect(localStorage.getItem('pulse.pinnedBuild')).toBeNull()
    expect(isPinned('M001')).toBe(false)
  })

  it('pinned without a copy, it is fetched; an older fetch answering late is dropped', async () => {
    let first: (m: Message) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise((r) => (first = r as (m: Message) => void)))
    pinBuild('B1', 'M001')
    expect(invoke).toHaveBeenCalledWith('get_build', { messageId: 'M001' })
    pinBuild('B1', 'M002', build('M002'))
    first(build('M001'))
    await settle()
    expect(pinnedBuild.message?.id).toBe('M002')
  })

  it('a damaged saved pin is ignored', () => {
    localStorage.setItem('pulse.pinnedBuild', '{not json')
    forgetPinnedBuild()
    handleReady(testReady([B1]))
    expect(pinnedBuild.key).toBeNull()
  })

  it('a saved pin whose channel is no longer there is gone at once, without a fetch', () => {
    localStorage.setItem('pulse.pinnedBuild', JSON.stringify({ userId: 'U1', channelId: 'B9', messageId: 'M001' }))
    forgetPinnedBuild()
    handleReady(testReady([B1]))
    expect(pinnedBuild.gone).toBe(true)
    expect(invoke).not.toHaveBeenCalledWith('get_build', expect.anything())
    expect(localStorage.getItem('pulse.pinnedBuild')).toBeNull()
  })
})
