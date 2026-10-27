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
const os = vi.hoisted(() => ({ focused: true, throws: false }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: async (h: (e: { payload: boolean }) => void) => ((tauriFocus = h), () => {}),
    isFocused: () => {
      if (os.throws) throw new Error('no window')
      return Promise.resolve(os.focused)
    },
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
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
})
function ready() {
  app.state = applyReady(emptyState(), {
    me, servers: [], channels: [{ id: 'D1', server_id: null, kind: 'dm', name: '', position: 0 }],
    members: [], dm_members: [{ channel_id: 'D1', members: [me, sam] }], voice: [], people: [], read_states: [], mutes: [],
    hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
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

  it('a sound decision is logged with the sound settings; open and focused only when the OS disagrees', async () => {
    await new Promise((r) => setTimeout(r, 0))
    ready()
    await startListening()
    ui.openChannelId = 'D1'
    const event = (id: string): Event => ({ t: 'MessageCreated', d: { message: msg(id, 'D1'), nonce: null } }) as Event
    const logs = () => invoke.mock.calls.filter((c) => c[0] === 'ui_log').map((c) => (c[1] as { line: string; info: boolean }))
    const settle = () => new Promise((r) => setTimeout(r, 0))
    tauriFocus!({ payload: true })
    await new Promise((r) => setTimeout(r, 2100)) // past the gap from the previous test's sound
    // chatting in the open, focused DM: nothing logged (the usual case, every message)
    os.focused = true
    invoke.mockClear()
    listeners.get('pulse://event')!({ payload: event('M10') })
    await settle()
    expect(logs()).toEqual([])
    // the page thinks it's focused, the OS says not: that's worth a line
    os.focused = false
    listeners.get('pulse://event')!({ payload: event('M11') })
    await settle()
    expect(logs().map((l) => l.line)).toEqual([
      'sound: message private=true mentioned=false open=true focused=true muted=false gap_ok=true sound=on play=false; the OS says focused=false',
    ])
    expect(logs().every((l) => l.info)).toBe(true)
    // nothing about who or what
    expect(logs().every((l) => !l.line.includes('U2') && !l.line.includes('hi') && !l.line.includes('D1'))).toBe(true)
    // unfocused: it dings, and says the sound settings (switched off reads differently from lost)
    os.focused = true
    tauriFocus!({ payload: false })
    localStorage.setItem('pulse.sounds', JSON.stringify({ set: 'classic', volume_pct: 100, off: ['message'] }))
    invoke.mockClear()
    listeners.get('pulse://event')!({ payload: event('M12') })
    expect(logs().map((l) => l.line)).toEqual(['sound: message private=true mentioned=false open=true focused=false muted=false gap_ok=true sound=off play=true'])
    localStorage.removeItem('pulse.sounds')
    tauriFocus!({ payload: true })
  }, 10_000)

  it('a diagnostic that throws never costs the event', async () => {
    await new Promise((r) => setTimeout(r, 0))
    ready()
    await startListening()
    ui.openChannelId = 'D1'
    tauriFocus!({ payload: true })
    os.throws = true
    try {
      listeners.get('pulse://event')!({ payload: { t: 'MessageCreated', d: { message: msg('M20', 'D1'), nonce: null } } as Event })
      expect(app.state.messages.D1?.some((m) => m.id === 'M20')).toBe(true)
    } finally {
      os.throws = false
    }
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
