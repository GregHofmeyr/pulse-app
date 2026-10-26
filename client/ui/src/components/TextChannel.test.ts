// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

// The core: list_messages for the scroll-up page waits until the test releases it.
let releaseOlder: (page: Message[]) => void = () => {}
const invoke = vi.fn(async (cmd: string, args?: { before?: string | null }): Promise<unknown> => {
  if (cmd === 'list_messages' && args?.before) return new Promise((done) => (releaseOlder = done))
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import TextChannel from './TextChannel.svelte'
import { app } from '../lib/store.svelte'
import { addHistory, applyReady, emptyState, markHistory, trimHistory } from '../lib/state'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
})
const page = (from: number, n: number) => Array.from({ length: n }, (_, i) => msg(from + i))

describe('TextChannel: leaving while an older page is loading', () => {
  it('drops the late page (no hole) and does not throw', async () => {
    const me = { id: 'U1', username: 'alex', avatar_hash: null }
    const s = applyReady(emptyState(), {
      me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [],
    })
    app.state = markHistory(addHistory(s, 'C1', page(100, 200)), 'C1', false) // M0100..M0299
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    document.querySelector('.list')!.dispatchEvent(new Event('scroll')) // scrollTop 0: load older
    expect(invoke).toHaveBeenCalledWith('list_messages', { channelId: 'C1', before: 'M0100' })

    unmount(c) // switched channel: Shell trims on leave
    app.state = trimHistory(app.state, 'C1')
    const trimmed = app.state.messages['C1']
    releaseOlder(page(90, 10)) // a SHORT late page (M0090..M0099) lands after the trim
    await tick()
    await new Promise((r) => setTimeout(r, 0))

    expect(app.state.messages['C1']).toBe(trimmed)
    expect(app.state.messages['C1'][0].id).toBe('M0200')
    expect(app.state.history['C1'].start).toBe(false) // the trimmed list has not reached the channel start
  })
})
