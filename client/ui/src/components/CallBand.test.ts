// @vitest-environment jsdom
// The call band at the top of a DM or group (#50), and the ringing popover elsewhere.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import { invoke } from '@tauri-apps/api/core'
import CallBand from './CallBand.svelte'
import CallRingPopover from './CallRingPopover.svelte'
import ConversationList from './ConversationList.svelte'
import { app, handleReady, resetState, ui } from '../lib/store.svelte'
import { handleCallEvent, resetCalls } from '../lib/calls.svelte'
import { voice } from '../lib/voice.svelte'
import { testReady } from '../lib/testReady'
import type { CallRing } from '../lib/protocol/CallRing'
import type { StreamInfo } from '../lib/protocol/StreamInfo'
import { stream, streamApi } from '../lib/stream/stream.svelte'
import { callView } from '../lib/stream/viewer.svelte'

const user = (id: string, username: string) => ({ id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const DM = { id: 'DM1', server_id: null, kind: 'dm' as const, name: null, position: 0 }
const GROUP = { id: 'G1', server_id: null, kind: 'group' as const, name: 'Friends', position: 0 }
const call = (channel: string) => ({ id: `C-${channel}`, channel_id: channel, started_by: 'U2', started_at: new Date().toISOString(), line_id: null })
const ring = (channel: string): CallRing => ({ call_id: `C-${channel}`, channel_id: channel, caller_id: 'U2', expires_at: new Date(Date.now() + 30_000).toISOString() })
const cmds = (name: string) => vi.mocked(invoke).mock.calls.filter(([c]) => c === name)
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === label || b.getAttribute('aria-label') === label)

let cmps: ReturnType<typeof mount>[] = []
const show = (c: typeof CallBand | typeof CallRingPopover, props: Record<string, unknown> = {}) => {
  cmps.push(mount(c as never, { target: document.body, props }))
  flushSync()
}

beforeEach(() => {
  resetState()
  resetCalls()
  handleReady(testReady([DM, GROUP], {
    me: user('U1', 'alex'),
    dm_members: [{ channel_id: 'DM1', user_ids: ['U1', 'U2'] }, { channel_id: 'G1', user_ids: ['U1', 'U2', 'U3'] }],
    people: ['alex', 'sam', 'jo'].map((n, i) => ({ user: user(`U${i + 1}`, n), online: true, last_seen_at: null, status: 'online' as const })),
  }))
  voice.channelId = null
  voice.joining = null
  voice.leaving = null
  ui.openChannelId = null
  flushSync()
  vi.mocked(invoke).mockClear()
})
afterEach(() => {
  cmps.forEach((c) => unmount(c))
  cmps = []
  resetCalls()
  document.body.innerHTML = ''
})

describe('CallBand', () => {
  it('shows nothing without a call', () => {
    show(CallBand, { channelId: 'DM1' })
    expect(document.querySelector('.band')).toBeNull()
  })
  it('a call going on: who is in it (muted flag), and Join', async () => {
    app.state = { ...app.state, calls: { DM1: call('DM1') }, voice: { DM1: [{ user_id: 'U2', flags: { muted: true, deafened: false } }] } }
    show(CallBand, { channelId: 'DM1' })
    expect(document.querySelector('.band')!.textContent).toContain('Call in progress')
    expect(document.querySelectorAll('.who')).toHaveLength(1)
    expect(document.querySelector('.who .flag')).not.toBeNull()
    expect(button('Decline')).toBeUndefined()
    button('Join')!.click()
    await vi.waitFor(() => expect(cmds('join_voice')).toHaveLength(1))
  })
  it('ringing you here: static, with Join and Decline, no popover', async () => {
    app.state = { ...app.state, calls: { DM1: call('DM1') } }
    ui.openChannelId = 'DM1'
    handleCallEvent({ t: 'CallRinging', d: { ring: ring('DM1') } })
    show(CallBand, { channelId: 'DM1' })
    show(CallRingPopover)
    expect(document.querySelector('.band.ringing')!.textContent).toContain('sam is calling…')
    expect(document.querySelector('.ring')).toBeNull()
    button('Decline')!.click()
    await vi.waitFor(() => expect(cmds('decline_call')).toEqual([['decline_call', { channelId: 'DM1' }]]))
    flushSync()
    expect(document.querySelector('.band.ringing')).toBeNull()
  })
  it('once you are in: mute, deafen and leave', () => {
    app.state = { ...app.state, calls: { DM1: call('DM1') }, voice: { DM1: [{ user_id: 'U1', flags: { muted: false, deafened: false } }] } }
    voice.channelId = 'DM1'
    show(CallBand, { channelId: 'DM1' })
    expect(button('Mute')).toBeDefined()
    expect(button('Deafen')).toBeDefined()
    button('Leave call')!.click()
    expect(cmds('leave_voice')).toHaveLength(1)
    expect(button('Join')).toBeUndefined()
  })

  describe('the soundboard (#175)', () => {
    const inCall = () => {
      app.state = {
        ...app.state,
        calls: { DM1: call('DM1') },
        voice: { DM1: [{ user_id: 'U1', flags: { muted: false, deafened: false } }, { user_id: 'U2', flags: { muted: false, deafened: false } }] },
      }
      voice.channelId = 'DM1'
    }
    const popover = () => document.querySelector('.soundboard-popover')

    it("once you are in, a Soundboard button opens the voice panel's soundboard, and toggles it shut", () => {
      inCall()
      show(CallBand, { channelId: 'DM1' })
      const sb = button('Soundboard')!
      // #221: in the call's action row, beside Go Live, as in the voice panel
      expect(sb.closest('.call-actions')).not.toBeNull()
      expect(button('Go Live')!.closest('.call-actions')).toBe(sb.closest('.call-actions'))
      expect(sb.getAttribute('aria-haspopup')).toBe('dialog')
      expect(sb.getAttribute('aria-expanded')).toBe('false')
      expect(popover()).toBeNull()
      sb.click()
      flushSync()
      expect(popover()).not.toBeNull()
      expect(popover()!.querySelector('input[placeholder="Find a sound"]')).not.toBeNull()
      expect(sb.getAttribute('aria-expanded')).toBe('true')
      sb.click()
      flushSync()
      expect(popover()).toBeNull()
    })

    it('no Soundboard button before you join, and leaving the call closes it', () => {
      app.state = { ...app.state, calls: { DM1: call('DM1') }, voice: { DM1: [{ user_id: 'U2', flags: { muted: false, deafened: false } }] } }
      show(CallBand, { channelId: 'DM1' })
      expect(button('Soundboard')).toBeUndefined()
      inCall()
      flushSync()
      button('Soundboard')!.click()
      flushSync()
      expect(popover()).not.toBeNull()
      voice.channelId = null
      flushSync()
      expect(popover()).toBeNull()
      expect(button('Soundboard')).toBeUndefined()
    })

    it("right-click someone in the band: Mute soundboard mutes that person's sounds for you", async () => {
      inCall()
      show(CallBand, { channelId: 'DM1' })
      const sam = document.querySelector<HTMLElement>('.who[aria-label="sam"]')!
      sam.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
      flushSync()
      const item = document.querySelector<HTMLButtonElement>('[data-item="sound-mute"]')!
      expect(item.textContent).toBe('Mute soundboard')
      item.click()
      flushSync()
      const { soundboard, setSoundMute } = await import('../lib/soundboard.svelte')
      expect(soundboard.mutes.U2).toBe(true)
      setSoundMute('U2', false)
    })
  })
})

describe('streams in the call (#51)', () => {
  const ua = navigator.userAgent
  const onWindows = (yes: boolean) => Object.defineProperty(navigator, 'userAgent', { value: yes ? 'Mozilla/5.0 (Windows NT 10.0)' : ua, configurable: true })
  const info = (channel: string, user: string, at = '2026-11-10T10:00:00Z'): StreamInfo => ({
    channel_id: channel, user_id: user, settings: { height: 1080, fps: 60, audio: true, allow_ghosts: false }, status: 'live', viewers: [], ghosts: 0, started_at: at,
  })
  const flags = { muted: false, deafened: false }
  /** sam (and maybe you) live in the group call; you're in it unless `out`. */
  const live = (streams: StreamInfo[], out = false) => {
    app.state = {
      ...app.state,
      calls: { G1: call('G1') },
      voice: { G1: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }, { user_id: 'U3', flags }].filter((m) => !out || m.user_id !== 'U1') },
      streams: { G1: Object.fromEntries(streams.map((s) => [s.user_id, s])) },
    }
    voice.channelId = out ? null : 'G1'
  }
  const watchBtn = (name: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="Watch ${name}'s stream"]`)
  let watchSpy: ReturnType<typeof vi.spyOn>
  beforeEach(() => {
    watchSpy = vi.spyOn(streamApi, 'watch').mockResolvedValue(undefined as never)
  })
  afterEach(() => {
    watchSpy.mockRestore()
    onWindows(false)
    stream.watching = []
    callView.full = null
    callView.goTo = null
    callView.joinThenWatch = null
  })

  it('shows who is live with a LIVE badge, and Watch opens their stream in full view', () => {
    onWindows(true)
    live([info('G1', 'U2')])
    show(CallBand, { channelId: 'G1' })
    const row = document.querySelector('.live-who')!
    expect(row.querySelector('.live')!.textContent).toBe('LIVE')
    expect(row.textContent).toContain('sam')
    const w = watchBtn('sam')!
    expect(w.textContent).toBe('Watch')
    w.click()
    expect(watchSpy).toHaveBeenCalledWith('G1', 'U2')
    expect(callView.full).toEqual({ channelId: 'G1', streamer: 'U2' })
  })

  it('your own stream has its badge but no Watch; each other streamer has theirs, oldest first', () => {
    onWindows(true)
    live([info('G1', 'U3', '2026-11-10T10:05:00Z'), info('G1', 'U1', '2026-11-10T10:01:00Z'), info('G1', 'U2', '2026-11-10T10:02:00Z')])
    show(CallBand, { channelId: 'G1' })
    expect([...document.querySelectorAll('.live-name')].map((n) => n.textContent)).toEqual(['alex', 'sam', 'jo'])
    expect(watchBtn('alex')).toBeNull()
    expect(watchBtn('sam')).not.toBeNull()
    expect(watchBtn('jo')).not.toBeNull()
  })

  it('a stream you already watch: Open takes you back to it in full view without watching again', () => {
    onWindows(true)
    live([info('G1', 'U2')])
    stream.watching = [{ channelId: 'G1', streamer: 'U2' } as never]
    show(CallBand, { channelId: 'G1' })
    const w = watchBtn('sam')!
    expect(w.textContent).toBe('Open')
    w.click()
    expect(watchSpy).not.toHaveBeenCalled()
    expect(callView.full).toEqual({ channelId: 'G1', streamer: 'U2' })
  })

  it('out of the call: Join & watch joins first (no ghosts in a DM or group)', async () => {
    onWindows(true)
    live([info('G1', 'U2')], true)
    show(CallBand, { channelId: 'G1' })
    const w = watchBtn('sam')!
    expect(w.textContent).toBe('Join & watch')
    w.click()
    expect(watchSpy).not.toHaveBeenCalled()
    expect(callView.joinThenWatch).toEqual({ channelId: 'G1', streamer: 'U2' })
    await vi.waitFor(() => expect(cmds('join_voice')).toHaveLength(1))
  })

  it('on Linux, Watch says why it is off and does nothing', () => {
    live([info('G1', 'U2')])
    show(CallBand, { channelId: 'G1' })
    const w = watchBtn('sam')!
    expect(w.getAttribute('aria-disabled')).toBe('true')
    expect(w.dataset.tip).toBe("Watching from Linux isn't supported yet")
    w.click()
    expect(watchSpy).not.toHaveBeenCalled()
    expect(callView.full).toBeNull()
  })

  // #285: as a server voice channel shows yours as a tile, the band shows your own stream (the live card, its preview
  // and Stop) while you're live here; no ghost switch (a DM or group has no ghosts)
  it('you live here: your live card shows under the band, without the ghost switch', () => {
    onWindows(true)
    globalThis.MediaStream ??= class {} as never
    live([info('G1', 'U1')])
    stream.live = { channelId: 'G1', res: 1080, fps: 60, audio: true, audioMuted: false, label: 'Game', encoderWarning: false, allowGhosts: false, sound: 'app' } as never
    show(CallBand, { channelId: 'G1' })
    const card = document.querySelector('section[aria-label="You\'re live"]')!
    expect(card).not.toBeNull()
    expect(card.querySelector('video')).not.toBeNull()
    expect(card.textContent).not.toContain('Allow ghost viewers')
    expect([...card.querySelectorAll('button')].some((b) => b.textContent?.trim() === 'Stop')).toBe(true)
    stream.live = null
    flushSync()
    expect(document.querySelector('section[aria-label="You\'re live"]')).toBeNull()
  })

  it('live in another call: no live card in this one', () => {
    onWindows(true)
    live([info('G1', 'U2')])
    stream.live = { channelId: 'DM1', res: 1080, fps: 60, audio: true, audioMuted: false, label: 'Game', encoderWarning: false, allowGhosts: false, sound: 'app' } as never
    show(CallBand, { channelId: 'G1' })
    expect(document.querySelector('section[aria-label="You\'re live"]')).toBeNull()
    stream.live = null
  })

  it('nobody live: no LIVE row', () => {
    live([])
    show(CallBand, { channelId: 'G1' })
    expect(document.querySelector('.band')).not.toBeNull()
    expect(document.querySelector('.live-who')).toBeNull()
  })
})

describe('CallRingPopover', () => {
  it('outside the conversation: the caller, Join and Decline', async () => {
    show(CallRingPopover)
    handleCallEvent({ t: 'CallRinging', d: { ring: ring('DM1') } })
    flushSync()
    const pop = document.querySelector('.ring')!
    expect(pop.textContent).toContain('sam')
    expect(pop.textContent).toContain('is calling you')
    expect(pop.querySelector('.av')).not.toBeNull()
    button('Join')!.click()
    await vi.waitFor(() => expect(cmds('join_voice')).toHaveLength(1))
    expect(ui.goTo).toBe('DM1')
    flushSync()
    expect(document.querySelector('.ring')).toBeNull()
  })
  it('a group: its name, with who is calling', () => {
    show(CallRingPopover)
    handleCallEvent({ t: 'CallRinging', d: { ring: ring('G1') } })
    flushSync()
    const pop = document.querySelector('.ring')!
    expect(pop.textContent).toContain('Friends')
    expect(pop.textContent).toContain('sam is calling')
  })
})

describe('the conversation list', () => {
  it('marks a conversation with a call going on', () => {
    app.state = { ...app.state, calls: { G1: call('G1') } }
    cmps.push(mount(ConversationList, { target: document.body, props: { activeId: null, onOpen: () => {}, onNew: () => {}, onLeft: () => {} } }))
    flushSync()
    expect(document.querySelectorAll('.in-call')).toHaveLength(1)
  })
})
