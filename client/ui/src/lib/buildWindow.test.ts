// @vitest-environment jsdom
// The build pop-out (#620), from the main window: Pop out hands the core one build's snapshot, its edits follow, and
// it closes when the build is deleted or out of reach.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Build } from './protocol/Build'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))

import { forgetBuildWindow, noteBuildWindowEvent, poppedOut, popOutBuild } from './buildWindow.svelte'
import { app, handleReady, resetState } from './store.svelte'
import { applyEvent } from './state'
import { testMe, testReady } from './testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = () =>
  testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  })
const build = (id: string, b: Partial<Build> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: `Build ${id}`, title_auto: false, game_id: 'G1', game_guessed: false, link: null, notes: 'Max crit', quick_drop: false,
    updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
})
const event = (e: Event) => {
  app.state = applyEvent(app.state, e, Date.now())
  noteBuildWindowEvent(e)
}
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>)
const settle = () => new Promise((r) => setTimeout(r, 0))

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async () => undefined)
  forgetBuildWindow()
  resetState()
  handleReady(ready())
})

describe('the build pop-out from the main window', () => {
  it('Pop out sends the snapshot, on top by default', async () => {
    await popOutBuild(build('M001'))
    const [args] = calls('open_build_window')
    expect(args.onTop).toBe(true)
    expect(args.snapshot).toMatchObject({ message_id: 'M001', title: 'Build M001', game_name: 'The First Descendant', author_name: 'sam' })
    expect(poppedOut.messageId).toBe('M001')
  })

  it('of two Pop outs at once, the later is the one noted, whichever answer comes first (#639)', async () => {
    const answers: Array<() => void> = []
    invoke.mockImplementation((cmd: string) => (cmd === 'open_build_window' ? new Promise<undefined>((r) => answers.push(() => r(undefined))) : Promise.resolve(undefined)))
    const first = popOutBuild(build('M001'))
    const second = popOutBuild(build('M002'))
    answers[1]()
    await second
    answers[0]()
    await first
    expect(calls('open_build_window').map((a) => (a.snapshot as { message_id: string }).message_id)).toEqual(['M001', 'M002'])
    expect(poppedOut.messageId).toBe('M002')
    // its edits are the ones sent on
    event({ t: 'MessageUpdated', d: { message: build('M002', { title: 'Raven endgame' }) } })
    event({ t: 'MessageUpdated', d: { message: build('M001', { title: 'Other' }) } })
    await settle()
    expect(calls('update_build_window').map((a) => (a.snapshot as { message_id: string }).message_id)).toEqual(['M002'])
  })

  it("A closing (its build deleted) while B's Pop out is in flight still notes B", async () => {
    await popOutBuild(build('M001'))
    let answer = () => {}
    invoke.mockImplementation((cmd: string) => (cmd === 'open_build_window' ? new Promise<undefined>((r) => (answer = () => r(undefined))) : Promise.resolve(undefined)))
    const second = popOutBuild(build('M002'))
    event({ t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M001', author_id: 'U2', mentions: [], thread_root_id: null } } as Event)
    answer()
    await second
    expect(poppedOut.messageId).toBe('M002')
    event({ t: 'MessageUpdated', d: { message: build('M002', { title: 'Raven endgame' }) } })
    await settle()
    expect(calls('update_build_window').map((a) => (a.snapshot as { message_id: string }).message_id)).toEqual(['M002'])
  })

  it('a sign-out while a Pop out is in flight notes nothing of it', async () => {
    let answer = () => {}
    invoke.mockImplementation((cmd: string) => (cmd === 'open_build_window' ? new Promise<undefined>((r) => (answer = () => r(undefined))) : Promise.resolve(undefined)))
    const open = popOutBuild(build('M001'))
    resetState()
    answer()
    await open
    expect(poppedOut.messageId).toBeNull()
  })

  it("its build's edit and archive are sent on; another build's are not", async () => {
    await popOutBuild(build('M001'))
    event({ t: 'MessageUpdated', d: { message: build('M001', { title: 'Raven endgame' }) } })
    event({ t: 'MessageUpdated', d: { message: build('M002', { title: 'Other' }) } })
    event({ t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M001'], archived_at: '2026-10-05T00:00:00Z', archived_by: 'U2' } })
    await settle()
    const sent = calls('update_build_window').map((a) => a.snapshot as { message_id: string; title: string; archived: boolean })
    expect(sent.map((s) => [s.message_id, s.title, s.archived])).toEqual([['M001', 'Raven endgame', false], ['M001', 'Raven endgame', true]])
  })

  // Review Focus 5
  it('deleting the build closes its pop-out', async () => {
    await popOutBuild(build('M001'))
    event({ t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M001', author_id: 'U2', mentions: [], thread_root_id: null } } as Event)
    await settle()
    expect(calls('close_build_window')).toEqual([{ messageId: 'M001' }])
    expect(poppedOut.messageId).toBeNull()
  })

  it('a comment deleted under it does not close it', async () => {
    await popOutBuild(build('M001'))
    event({ t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M001', author_id: 'U2', mentions: [], thread_root_id: 'M000' } } as Event)
    await settle()
    expect(calls('close_build_window')).toEqual([])
  })

  it('losing access (its channel leaves the data) closes it', async () => {
    await popOutBuild(build('M001'))
    event({ t: 'ChannelDeleted', d: { channel_id: 'B1', server_id: 'S1' } })
    await settle()
    expect(calls('close_build_window')).toEqual([{ messageId: 'M001' }])
  })

  it('signing out forgets the popped-out build', async () => {
    await popOutBuild(build('M001'))
    resetState()
    expect(poppedOut.messageId).toBeNull()
    expect(poppedOut.channelId).toBeNull()
    handleReady(ready())
    event({ t: 'MessageUpdated', d: { message: build('M001', { title: 'Raven endgame' }) } })
    await settle()
    expect(calls('update_build_window'), "nothing of the last account's pop-out is sent on").toEqual([])
  })
})
