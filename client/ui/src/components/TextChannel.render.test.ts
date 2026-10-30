// @vitest-environment jsdom
// The rendered-message cache as the channel uses it: mention pills follow who can be mentioned,
// and an event that touches no message (typing) renders none again. Also the typing line's timer.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { User } from '../lib/protocol/User'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
const h = vi.hoisted(() => ({ renders: 0 }))
vi.mock('../lib/markdown', async (importOriginal) => {
  const real = await importOriginal<typeof import('../lib/markdown')>()
  return {
    ...real,
    renderMessage: (...args: Parameters<typeof real.renderMessage>) => {
      h.renders++
      return real.renderMessage(...args)
    },
  }
})

import TextChannel from './TextChannel.svelte'
import { app, resetState } from '../lib/store.svelte'
import { addHistory, applyEvent, applyReady, emptyState, markHistory, TYPING_TTL_MS } from '../lib/state'

const user = (id: string, username: string): User => ({ id, username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const me = user('U1', 'alex')
const sam = user('U2', 'sam')
const msg = (id: string, content: string): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null,
})
const card = { banner_hash: null, banner_still: null, theme_top: '#000000', theme_bottom: '#000000', pronouns: null, bio: null }
const pills = () => [...document.querySelectorAll('.mention')].map((e) => e.textContent)

describe('TextChannel: rendered messages', () => {
  let c: ReturnType<typeof mount> | undefined
  beforeEach(() => {
    Element.prototype.scrollTo = () => {} // jsdom has none
    resetState()
    app.state = applyReady(emptyState(), {
      me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
      dm_members: [], voice: [], people: [{ user: sam, online: true, last_seen_at: null }], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [],
    })
    app.state = markHistory(addHistory(app.state, 'C1', [msg('M1', 'hi @sam'), msg('M2', 'and @kim'), msg('M3', 'and @jo')]), 'C1', true)
    c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    h.renders = 0
  })
  afterEach(() => {
    if (c) unmount(c)
    document.body.innerHTML = ''
  })

  it('someone typing renders no message again', () => {
    expect(pills()).toEqual(['@sam'])
    app.state = applyEvent(app.state, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, Date.now())
    flushSync()
    expect(document.querySelector('.typing')?.textContent).toContain('sam')
    expect(h.renders).toBe(0)
  })

  it('a rename and a new person together: the pills follow both', () => {
    expect(pills()).toEqual(['@sam'])
    app.state = applyEvent(app.state, { t: 'ProfileUpdated', d: { user: user('U2', 'kim'), card } }, 0)
    app.state = applyEvent(app.state, { t: 'UserCreated', d: { user: user('U3', 'jo') } }, 0)
    flushSync()
    expect(pills()).toEqual(['@kim', '@jo'])
  })

  it('the typing line fades each person on time, re-arming its one timer for the next', async () => {
    vi.useFakeTimers()
    try {
      const typing = () => document.querySelector('.typing')?.textContent?.trim()
      app.state = applyEvent(app.state, { t: 'MemberJoined', d: { server_id: 'S1', member: { user: user('U3', 'jo'), nickname: null } } }, 0)
      app.state = applyEvent(app.state, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, Date.now())
      flushSync()
      await vi.advanceTimersByTimeAsync(2000)
      app.state = applyEvent(app.state, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U3' } }, Date.now())
      flushSync()
      expect(typing()).toBe('sam, jo are typing…')
      await vi.advanceTimersByTimeAsync(TYPING_TTL_MS - 2000 + 100) // sam's ran out
      flushSync()
      expect(typing()).toBe('jo is typing…')
      await vi.advanceTimersByTimeAsync(2000) // jo's too: the timer was armed again for it
      flushSync()
      expect(typing()).toBe('')
      expect(vi.getTimerCount()).toBe(0) // nobody typing: no timer left
    } finally {
      vi.useRealTimers()
    }
  })
})
