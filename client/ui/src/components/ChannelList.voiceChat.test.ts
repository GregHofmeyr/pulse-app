// @vitest-environment jsdom
// A voice channel's chat icon on its row (#226): it opens the chat, and carries its unreads (a dot) and mentions (a
// count), not the channel's name. Only for people in the server, like the rest of a voice row's controls.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testReady } from '../lib/testReady'
import type { Channel } from '../lib/protocol/Channel'

const CHANNELS: Channel[] = [
  { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
  { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 1 },
]
let cmp: ReturnType<typeof mount> | undefined
const onOpenChat = vi.fn()
const onJoinVoice = vi.fn()
const show = (reads: { channel_id: string; unread: number; mentions: number }[] = [], opts: { member?: boolean; muted?: boolean } = {}) => {
  const r = testReady(CHANNELS)
  r.read_states = reads.map((x) => ({ ...x, last_read_message_id: null }))
  if (opts.member === false) r.members = r.members.map((m) => ({ ...m, members: m.members.filter((x) => x.user.id !== r.me.id) }))
  if (opts.muted) r.mutes = [{ target_kind: 'channel', target_id: 'V1', until: null }]
  app.state = applyReady(emptyState(), r)
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice, onOpenChat, onMessage: () => {}, onVoiceSettings: () => {} },
  })
  flushSync()
}
const voiceRow = () => document.querySelector<HTMLElement>('.row[data-channel="V1"]')!
const chat = () => document.querySelector<HTMLButtonElement>('.chat-btn')

beforeEach(() => {
  onOpenChat.mockClear()
  onJoinVoice.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('the voice row chat icon (#226)', () => {
  it('sits right after its row (not inside it: no button in a button) and opens the chat, not the call', () => {
    show()
    expect(voiceRow().nextElementSibling).toBe(chat())
    expect(voiceRow().querySelector('.chat-btn')).toBeNull()
    expect(chat()!.getAttribute('aria-label')).toBe('Open chat for Lounge')
    chat()!.click()
    expect(onOpenChat).toHaveBeenCalledWith('V1')
    expect(onJoinVoice).not.toHaveBeenCalled()
    // text channels have none
    expect(document.querySelectorAll('.chat-btn').length).toBe(1)
  })

  it('unreads show as a dot on the icon, mentions as a count; the name stays as it is', () => {
    show([{ channel_id: 'V1', unread: 3, mentions: 0 }])
    expect(chat()!.querySelector('.chat-dot')).not.toBeNull()
    expect(chat()!.classList.contains('lit')).toBe(true)
    expect(chat()!.getAttribute('aria-label')).toBe('Open chat for Lounge, unread messages')
    expect(voiceRow().classList.contains('unread')).toBe(false)
    unmount(cmp!)
    show([{ channel_id: 'V1', unread: 3, mentions: 2 }])
    expect(chat()!.querySelector('.chat-pill')?.textContent).toBe('2')
    expect(chat()!.querySelector('.chat-dot')).toBeNull()
    expect(chat()!.getAttribute('aria-label')).toBe('Open chat for Lounge, 2 mentions')
    expect(voiceRow().querySelector('.pill')).toBeNull()
  })

  it('a muted voice channel shows no dot, but still counts mentions', () => {
    show([{ channel_id: 'V1', unread: 3, mentions: 0 }], { muted: true })
    expect(chat()!.querySelector('.chat-dot')).toBeNull()
    expect(chat()!.classList.contains('lit')).toBe(false)
    unmount(cmp!)
    show([{ channel_id: 'V1', unread: 3, mentions: 1 }], { muted: true })
    expect(chat()!.querySelector('.chat-pill')?.textContent).toBe('1')
  })

  it("hovering the icon keeps its row's hover highlight (#230)", () => {
    show()
    expect(voiceRow().classList.contains('hover')).toBe(false)
    chat()!.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(voiceRow().classList.contains('hover')).toBe(true)
    chat()!.dispatchEvent(new Event('pointerleave'))
    flushSync()
    expect(voiceRow().classList.contains('hover')).toBe(false)
  })

  it('people not in the server have none', () => {
    show([], { member: false })
    expect(chat()).toBeNull()
  })
})

// #277: while something's shared to a voice channel's stage (#234), its row says so, for people outside the call.
// A text mark, not Spotify's icon: their icon has to be at least 21px, too big for a row of 14-17px icons.
describe('the voice row Spotify mark (#277)', () => {
  const share = (title: string) => ({
    id: 'SH1', channel_id: 'V1', shared_by: 'U1', shared_at: new Date().toISOString(), expires_at: new Date(Date.now() + 3 * 3600_000).toISOString(),
    embed: { url: 'https://open.spotify.com/track/abc', provider: 'spotify', kind: 'player', provider_id: 'track:abc', title, description: null, site_name: 'Spotify', author: 'M83', thumb: null, start: null, vertical: false },
  })
  const mark = () => voiceRow().querySelector<HTMLElement>('.on-stage')

  it('none while nothing is shared', () => {
    show()
    expect(mark()).toBeNull()
  })

  it('shows while a share is on the stage, says what it is, and goes when it is cleared', () => {
    show()
    app.state = { ...app.state, stageShares: { V1: [share('Midnight City') as never] } }
    flushSync()
    expect(mark()!.textContent).toBe('Spotify')
    expect(mark()!.dataset.tip).toBe('On the stage: Midnight City')
    expect(mark()!.querySelector('svg')).toBeNull()
    app.state = { ...app.state, stageShares: {} }
    flushSync()
    expect(mark()).toBeNull()
  })
})
