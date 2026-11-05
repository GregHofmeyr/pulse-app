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
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
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
    expect(heading('Text Channels').getAttribute('aria-expanded')).toBe('true')
    heading('Text Channels').click()
    flushSync()
    expect(heading('Text Channels').getAttribute('aria-expanded')).toBe('false')
    expect(names()).toEqual(['general', 'news', 'Lounge', 'Games'])
    heading('Text Channels').click()
    flushSync()
    expect(names()).toEqual(['general', 'memes', 'news', 'Lounge', 'Games'])
  })

  it('collapsed voice keeps the channel you are in; the choice is remembered on this device', () => {
    show({ voiceChannelId: 'V2' })
    heading('Voice Channels').click()
    flushSync()
    expect(names()).toEqual(['general', 'memes', 'news', 'Games'])
    unmount(cmp!)
    cmp = undefined
    resetCollapsed() // as after a restart: read back from storage
    show({ voiceChannelId: null })
    expect(heading('Voice Channels').getAttribute('aria-expanded')).toBe('false')
    expect(names()).toEqual(['general', 'memes', 'news'])
  })
})

describe('collapsible categories (#54)', () => {
  // Gaming holds every kind: what a collapsed section keeps applies to each row, whatever its kind.
  const cat = (id: string, name: string, position: number) => ({ id, server_id: 'S1', name, position })
  const live: Ready = {
    ...ready,
    channels: [
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0, category_id: 'C1' },
      { id: 'G1', server_id: 'S1', kind: 'text', name: 'lfg', position: 0, category_id: 'C2' },
      { id: 'G2', server_id: 'S1', kind: 'quotes', name: 'lines', position: 1, category_id: 'C2' },
      { id: 'G3', server_id: 'S1', kind: 'clips', name: 'plays', position: 2, category_id: 'C2' },
      { id: 'G4', server_id: 'S1', kind: 'voice', name: 'Squad', position: 3, category_id: 'C2' },
      { id: 'G5', server_id: 'S1', kind: 'voice', name: 'Raid', position: 4, category_id: 'C2' },
      { id: 'G6', server_id: 'S1', kind: 'voice', name: 'Duo', position: 5, category_id: 'C2' },
    ],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: { ...me, id: 'U2', username: 'sam', display_name: 'sam' }, nickname: null }] }],
    // you're in Squad; sam is in Raid
    voice: [{ channel_id: 'G4', members: [{ user_id: 'U1', flags: { muted: false, deafened: false } }] }, { channel_id: 'G5', members: [{ user_id: 'U2', flags: { muted: false, deafened: false } }] }],
    read_states: [{ channel_id: 'G2', last_read_message_id: null, unread: 4, mentions: 0 }],
    layouts: [{ server_id: 'S1', revision: 1, categories: [cat('C1', 'Text Channels', 0), cat('C2', 'Gaming', 1)], channels: [] }],
  }
  const gaming = () => document.querySelector<HTMLButtonElement>('.cat[data-category="C2"] > .label button.section')!
  const inGaming = () => [...document.querySelectorAll('.cat[data-category="C2"] .row .grow')].map((e) => e.textContent?.trim())
  const occupants = () => [...document.querySelectorAll<HTMLElement>('.cat[data-category="C2"] .occupant')].map((o) => o.dataset.user)

  beforeEach(() => {
    app.state = applyReady(emptyState(), live)
  })

  it('a collapsed category keeps the open channel, unread ones and the voice channel you are in with who is there', () => {
    show({ activeChannelId: 'G3', voiceChannelId: 'G4' })
    expect(inGaming()).toEqual(['lfg', 'lines', 'plays', 'Squad', 'Raid', 'Duo'])
    expect(occupants()).toEqual(['U1', 'U2'])
    gaming().click()
    flushSync()
    expect(gaming().getAttribute('aria-expanded')).toBe('false')
    expect(inGaming()).toEqual(['lines', 'plays', 'Squad'])
    expect(occupants()).toEqual(['U1'])
    // the other category is untouched
    expect(document.querySelector('.cat[data-category="C1"] .row')?.textContent).toContain('general')
  })

  it('collapsed is remembered per category on this device', () => {
    show()
    gaming().click()
    flushSync()
    unmount(cmp!)
    cmp = undefined
    resetCollapsed()
    show()
    expect(gaming().getAttribute('aria-expanded')).toBe('false')
    expect(inGaming()).toEqual(['lines'])
    expect(JSON.parse(localStorage.getItem('pulse.collapsedSections')!)).toEqual(['cat:C2'])
  })
})
