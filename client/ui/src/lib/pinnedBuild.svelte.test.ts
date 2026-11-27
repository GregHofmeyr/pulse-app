// @vitest-environment jsdom
// Pin to side's store (#619): saved per account, fetched when no copy is given, and an older fetch never wins.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from './protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { forgetPinnedBuild, isPinned, notePinnedBuildEvent, pinBuild, pinnedBuild, refreshPinnedBuild, unpinBuild } from './pinnedBuild.svelte'
import { handleReady, resetState } from './store.svelte'
import { testReady } from './testReady'
import { microtasks } from './testSettle'

const B1 = { id: 'B1', server_id: 'S1', kind: 'builds' as const, name: 'raven-builds', position: 0 }
const build = (id: string, title = `Build ${id}`): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, embeds: [], clip: null,
  build: { title, title_auto: false, game_id: null, game_guessed: false, link: null, notes: null, quick_drop: false, updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null },
})

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
    expect(JSON.parse(localStorage.getItem('pulse.pinnedBuild.U1')!)).toEqual({ channelId: 'B1', messageId: 'M001' })
    expect(isPinned('M001')).toBe(true)
    expect(invoke).not.toHaveBeenCalled()
    unpinBuild()
    expect(localStorage.getItem('pulse.pinnedBuild.U1')).toBeNull()
    expect(isPinned('M001')).toBe(false)
  })

  it('pinned without a copy, it is fetched; an older fetch answering late is dropped', async () => {
    let first: (m: Message) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise((r) => (first = r as (m: Message) => void)))
    pinBuild('B1', 'M001')
    expect(invoke).toHaveBeenCalledWith('get_build', { messageId: 'M001' })
    pinBuild('B1', 'M002', build('M002'))
    first(build('M001'))
    await microtasks(10)
    expect(pinnedBuild.message?.id).toBe('M002')
  })

  it('a damaged saved pin is ignored', () => {
    localStorage.setItem('pulse.pinnedBuild.U1', '{not json')
    forgetPinnedBuild()
    handleReady(testReady([B1]))
    expect(pinnedBuild.key).toBeNull()
  })

  it('a saved pin whose channel is no longer there is gone at once, without a fetch', () => {
    localStorage.setItem('pulse.pinnedBuild.U1', JSON.stringify({ channelId: 'B9', messageId: 'M001' }))
    forgetPinnedBuild()
    handleReady(testReady([B1]))
    expect(pinnedBuild.gone).toBe(true)
    expect(invoke).not.toHaveBeenCalledWith('get_build', expect.anything())
    expect(localStorage.getItem('pulse.pinnedBuild.U1')).toBeNull()
  })

  // #636 review
  it('each account keeps its own pin', () => {
    pinBuild('B1', 'M001', build('M001'))
    resetState()
    handleReady(testReady([B1], { me: { ...testReady([]).me, id: 'U2', username: 'sam' } }))
    expect(pinnedBuild.key).toBeNull()
    pinBuild('B1', 'M002', build('M002'))
    expect(JSON.parse(localStorage.getItem('pulse.pinnedBuild.U1')!)).toEqual({ channelId: 'B1', messageId: 'M001' })
    expect(JSON.parse(localStorage.getItem('pulse.pinnedBuild.U2')!)).toEqual({ channelId: 'B1', messageId: 'M002' })
  })

  it("the old single saved pin moves to its account's key", () => {
    localStorage.setItem('pulse.pinnedBuild', JSON.stringify({ userId: 'U1', channelId: 'B1', messageId: 'M005' }))
    forgetPinnedBuild()
    handleReady(testReady([B1]))
    expect(pinnedBuild.key).toEqual({ channelId: 'B1', messageId: 'M005' })
    expect(localStorage.getItem('pulse.pinnedBuild')).toBeNull()
    expect(JSON.parse(localStorage.getItem('pulse.pinnedBuild.U1')!)).toEqual({ channelId: 'B1', messageId: 'M005' })
  })

  it('a refetch answering after a live edit never puts the older copy back', async () => {
    let answer: (m: Message) => void = () => {}
    invoke.mockImplementation((cmd: string) => (cmd === 'get_build' ? new Promise((r) => (answer = r as (m: Message) => void)) : Promise.resolve(undefined)))
    pinBuild('B1', 'M001', build('M001'))
    void refreshPinnedBuild()
    notePinnedBuildEvent({ t: 'MessageUpdated', d: { message: build('M001', 'Edited') } })
    answer(build('M001', 'Stale'))
    await microtasks(10)
    expect(pinnedBuild.message?.build?.title).toBe('Edited')
  })

  it('a refetch answering after an archive never puts the older copy back', async () => {
    let answer: (m: Message) => void = () => {}
    invoke.mockImplementation((cmd: string) => (cmd === 'get_build' ? new Promise((r) => (answer = r as (m: Message) => void)) : Promise.resolve(undefined)))
    pinBuild('B1', 'M001', build('M001'))
    void refreshPinnedBuild()
    notePinnedBuildEvent({ t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M001'], archived_at: '2026-10-05T00:00:00Z', archived_by: 'U2' } })
    answer(build('M001'))
    await microtasks(10)
    expect(pinnedBuild.message?.build?.archived_at).toBe('2026-10-05T00:00:00Z')
  })
})
