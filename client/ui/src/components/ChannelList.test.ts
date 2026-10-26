// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
let emit: (e: unknown) => void = () => {}
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (_: string, cb: (e: { payload: unknown }) => void) => {
    emit = (payload) => cb({ payload })
    return () => {}
  },
}))

import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { startVoiceListening, voice, voiceApi } from '../lib/voice.svelte'
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

beforeEach(async () => {
  await startVoiceListening()
  invoke.mockReset()
  app.state = applyReady(emptyState(), ready)
  voice.channelId = null
  voice.joining = null
  voice.leaving = null
  voice.joinFailed = null
  voice.stage = { stage: 'disconnected', attempt: null }
  voice.link = null
  voice.error = ''
  voice.connection = 'disconnected'
  voice.micLive = false
  voice.volumes = {}
  voice.localMutes = {}
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
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

const flags = { muted: false, deafened: false }
const st = (channel_id: string | null, connection: string) => ({ kind: 'state', channel_id, connection, controls: flags })
const stage = (s: string) => ({ kind: 'stage', stage: s, attempt: null })
const settle = async () => { await tick(); flushSync() }

describe('voice listener rules', () => {
  it('a_second_click_on_the_channel_being_joined_does_not_restart_it', async () => {
    invoke.mockReturnValue(new Promise(() => {}))
    void voiceApi.join('V1')
    await settle()
    emit(stage('connecting'))
    void voiceApi.join('V1')
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'join_voice')).toHaveLength(1)
    expect(voice.stage.stage).toBe('connecting')
    expect(voice.joining).toBe('V1')
    expect(rows('Lounge')).toHaveLength(1)
  })

  it('a_failed_join_is_reported_once_by_the_status_not_also_as_a_voice_error', async () => {
    invoke.mockImplementation((c: string) => (c === 'join_voice' ? Promise.reject("couldn't connect to voice: x") : Promise.resolve()))
    await voiceApi.join('V1')
    expect(voice.joinFailed).toEqual({ channelId: 'V1', error: "couldn't connect to voice: x" })
    expect(voice.error).toBe('')
  })

  it('a_join_rejecting_after_leave_is_not_a_failure', async () => {
    let rejectJoin!: (e: unknown) => void
    invoke.mockImplementation((c: string) => (c === 'join_voice' ? new Promise((_, rej) => (rejectJoin = rej)) : Promise.resolve()))
    const joining = voiceApi.join('V1')
    await settle()
    expect(rejectJoin).toBeDefined()
    void voiceApi.leave()
    rejectJoin("couldn't connect to voice: x")
    await joining
    await settle()
    expect(voice.joinFailed).toBeNull()
    expect(voice.error).toBe('')
  })

  it('a_link_sample_after_leaving_is_dropped', () => {
    const sample = { kind: 'link', rtt_ms: 40, jitter_ms: 2, loss_pct: 0, transport: 'udp', server: 'voice.example.test', version: '1.0.0', history: [] }
    emit(sample)
    expect(voice.link).toBeNull()
    voice.channelId = 'V1'
    emit(sample)
    expect(voice.link).not.toBeNull()
  })

  it('a_real_failure_resets_the_stage', async () => {
    invoke.mockRejectedValue("couldn't connect to voice: x")
    await voiceApi.join('V1')
    expect(voice.stage).toEqual({ stage: 'disconnected', attempt: null })
  })

  it('joining_survives_the_cancelled_joins_disconnect_and_ends_when_the_new_channel_is_connected', async () => {
    invoke.mockReturnValue(new Promise(() => {}))
    void voiceApi.join('V2')
    emit(st(null, 'disconnected'))
    emit(stage('requesting_access'))
    emit(stage('connecting'))
    expect(voice.joining).toBe('V2')
    emit(st('V2', 'connecting'))
    // the cancelled join's disconnect lands late, then audio starts: channelId is unset, the row must stay
    emit(st(null, 'disconnected'))
    emit(stage('starting_audio'))
    await settle()
    expect(voice.joining).toBe('V2')
    expect(rows('Games')).toHaveLength(1)
    emit(st('V2', 'connected'))
    emit(stage('connected'))
    await settle()
    expect(voice.joining).toBeNull()
  })

  it('joining_clears_when_audio_starts_in_the_channel_you_are_in', async () => {
    invoke.mockReturnValue(new Promise(() => {}))
    void voiceApi.join('V1')
    emit(st('V1', 'connecting'))
    emit(stage('starting_audio'))
    expect(voice.joining).toBeNull()
  })

  it('leaving_settles_when_the_server_list_drops_you', async () => {
    app.state = applyEvent(app.state, { t: 'VoiceJoined', d: { channel_id: 'V1', user_id: 'U1', flags } }, 0)
    voice.channelId = 'V1'
    await settle()
    invoke.mockResolvedValue(undefined)
    void voiceApi.leave()
    await settle()
    expect(voice.leaving).toBe('V1')
    expect(rows('Lounge')).toHaveLength(0)
    app.state = applyEvent(app.state, { t: 'VoiceLeft', d: { channel_id: 'V1', user_id: 'U1' } } as never, 0)
    await settle()
    expect(voice.leaving).toBeNull()
  })
})
