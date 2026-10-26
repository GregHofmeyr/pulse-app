// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { voice, voiceApi } from '../lib/voice.svelte'
import type { Ready } from '../lib/protocol/Ready'

const me = { id: 'U1', username: 'alex', avatar_hash: null }
const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
    { id: 'V2', server_id: 'S1', kind: 'voice', name: 'Games', position: 1 },
  ],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [],
}

let cmp: ReturnType<typeof mount> | undefined
const rows = (ch: string) => {
  const all = [...document.querySelectorAll('.row')]
  const start = all.findIndex((r) => r.textContent?.includes(ch))
  const out: Element[] = []
  for (let el = all[start]?.nextElementSibling; el && el.classList.contains('occupant'); el = el.nextElementSibling) out.push(el)
  return out
}

beforeEach(() => {
  invoke.mockReset()
  app.state = applyReady(emptyState(), ready)
  voice.channelId = null
  voice.joining = null
  voice.leaving = null
  voice.joinFailed = null
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {} },
  })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ChannelList joining', () => {
  it('joining_shows_a_greyed_entry_with_spinner_at_once', async () => {
    invoke.mockReturnValue(new Promise(() => {}))
    void voiceApi.join('V1')
    flushSync()
    const [row] = rows('Lounge')
    expect(row.classList.contains('pending')).toBe(true)
    expect(row.textContent).toContain('Joining…')
    expect(row.querySelector('[role=progressbar], .spinner')).not.toBeNull()
  })

  it('failed_join_removes_the_entry', async () => {
    invoke.mockRejectedValue("couldn't connect to voice: x")
    await voiceApi.join('V1')
    await tick()
    flushSync()
    expect(rows('Lounge')).toHaveLength(0)
    expect(voice.joinFailed?.channelId).toBe('V1')
    expect(voice.joining).toBeNull()
  })

  it('a_superseded_join_is_not_a_failure', async () => {
    let rejectFirst!: (e: unknown) => void
    invoke.mockReturnValueOnce(new Promise((_, rej) => (rejectFirst = rej)))
    invoke.mockReturnValue(new Promise(() => {}))
    const first = voiceApi.join('V1')
    void voiceApi.join('V2')
    rejectFirst('cancelled')
    await first
    await tick()
    flushSync()
    expect(voice.joinFailed).toBeNull()
    expect(rows('Lounge')).toHaveLength(0)
    expect(rows('Games')).toHaveLength(1)
    expect(document.querySelectorAll('.occupant.pending')).toHaveLength(1)
  })
})
