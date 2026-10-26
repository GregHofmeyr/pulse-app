// @vitest-environment jsdom
// Message sounds, read marking and animated emoji all follow the one window-focus source.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Event } from '../lib/protocol/Event'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'list_messages' ? [] : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
const listeners = new Map<string, (e: { payload: unknown }) => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, fn: (e: { payload: unknown }) => void) => {
    listeners.set(name, fn)
    return () => {}
  },
}))
let tauriFocus: ((e: { payload: boolean }) => void) | undefined
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: async (h: (e: { payload: boolean }) => void) => ((tauriFocus = h), () => {}),
    isFocused: async () => true,
  }),
}))

import TextChannel from './TextChannel.svelte'
import { app, startListening, ui } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { watchWindowFocus, windowFocus } from '../lib/windowFocus.svelte'

const me = { id: 'U1', username: 'alex', avatar_hash: null }
const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const msg = (id: string, channel_id: string): Message => ({
  id, channel_id, author_id: 'U2', kind: 'normal', content: 'hi', reply_to_id: null,
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [],
})
function ready() {
  app.state = applyReady(emptyState(), {
    me, servers: [], channels: [{ id: 'D1', server_id: null, kind: 'dm', name: '', position: 0 }],
    members: [], dm_members: [{ channel_id: 'D1', members: [me, sam] }], voice: [], people: [], read_states: [], mutes: [],
    hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [],
  } as never)
}
const sounds = () => invoke.mock.calls.filter((c) => c[0] === 'play_sound').length

// In Tauri, with the subscription established: the OS window decides.
;(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {}
Element.prototype.scrollTo = () => {}
watchWindowFocus()

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  invoke.mockClear()
})

describe('one window-focus source', () => {
  it('the message sound follows it', async () => {
    await new Promise((r) => setTimeout(r, 0))
    ready()
    await startListening()
    ui.openChannelId = 'D1'
    const event = (id: string): Event => ({ t: 'MessageCreated', d: { message: msg(id, 'D1'), nonce: null } }) as Event
    tauriFocus!({ payload: true })
    listeners.get('pulse://event')!({ payload: event('M1') })
    expect(sounds()).toBe(0) // open and focused: no sound
    window.dispatchEvent(new window.Event('blur')) // DOM blur alone doesn't count in Tauri
    listeners.get('pulse://event')!({ payload: event('M2') })
    expect(sounds()).toBe(0)
    tauriFocus!({ payload: false })
    expect(windowFocus.focused).toBe(false)
    listeners.get('pulse://event')!({ payload: event('M3') })
    expect(sounds()).toBe(1)
    tauriFocus!({ payload: true })
  })

  it('read marking follows it', async () => {
    vi.useFakeTimers()
    try {
      ready()
      app.state = { ...app.state, messages: { D1: [msg('M5', 'D1')] } }
      tauriFocus!({ payload: false })
      cmp = mount(TextChannel, { target: document.body, props: { channelId: 'D1', serverId: null } })
      flushSync()
      vi.advanceTimersByTime(1000)
      expect(invoke.mock.calls.filter((c) => c[0] === 'mark_read')).toHaveLength(0)
      tauriFocus!({ payload: true })
      flushSync()
      vi.advanceTimersByTime(1000)
      expect(invoke).toHaveBeenCalledWith('mark_read', { channelId: 'D1', messageId: 'M5' })
    } finally {
      vi.useRealTimers()
    }
  })
})
