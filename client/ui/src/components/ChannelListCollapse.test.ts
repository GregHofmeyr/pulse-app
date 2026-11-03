// @vitest-environment jsdom
// Collapsible sidebar sections (#99): a section's heading hides its channels, remembered on this
// device per server; a collapsed section still shows the channel you're on, unread ones, and the
// voice channel you're in with who's there.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { resetCollapsed } from '../lib/sidebarSections.svelte'
import type { Ready } from '../lib/protocol/Ready'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [
    { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    { id: 'T2', server_id: 'S1', kind: 'text', name: 'memes', position: 1 },
    { id: 'T3', server_id: 'S1', kind: 'text', name: 'news', position: 2 },
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 3 },
    { id: 'V2', server_id: 'S1', kind: 'voice', name: 'Games', position: 4 },
  ],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [],
}

let cmp: ReturnType<typeof mount> | undefined
function show(props: { activeChannelId?: string | null; voiceChannelId?: string | null } = {}) {
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {}, ...props },
  })
  flushSync()
}
const names = () => [...document.querySelectorAll('.row .grow')].map((e) => e.textContent?.trim())
const heading = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button.section')].find((b) => b.textContent?.includes(text))!

beforeEach(() => {
  localStorage.clear()
  resetCollapsed()
  app.state = applyReady(emptyState(), ready)
  app.state = { ...app.state, reads: { ...app.state.reads, T3: { channel_id: 'T3', last_read_message_id: null, unread: 2, mentions: 0 } } }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('collapsible sidebar sections', () => {
  it('a heading collapses its section, keeping the open channel and unread ones', () => {
    show({ activeChannelId: 'T1' })
    expect(heading('TEXT CHANNELS').getAttribute('aria-expanded')).toBe('true')
    heading('TEXT CHANNELS').click()
    flushSync()
    expect(heading('TEXT CHANNELS').getAttribute('aria-expanded')).toBe('false')
    expect(names()).toEqual(['general', 'news', 'Lounge', 'Games'])
    heading('TEXT CHANNELS').click()
    flushSync()
    expect(names()).toEqual(['general', 'memes', 'news', 'Lounge', 'Games'])
  })

  it('collapsed voice keeps the channel you are in; the choice is remembered on this device', () => {
    show({ voiceChannelId: 'V2' })
    heading('VOICE CHANNELS').click()
    flushSync()
    expect(names()).toEqual(['general', 'memes', 'news', 'Games'])
    unmount(cmp!)
    cmp = undefined
    resetCollapsed() // as after a restart: read back from storage
    show({ voiceChannelId: null })
    expect(heading('VOICE CHANNELS').getAttribute('aria-expanded')).toBe('false')
    expect(names()).toEqual(['general', 'memes', 'news'])
  })
})
