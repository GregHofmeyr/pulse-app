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
import { applyEvent, applyLayout, applyReady, emptyState } from '../lib/state'
import { startVoiceListening, voice, voiceApi } from '../lib/voice.svelte'
import type { Ready } from '../lib/protocol/Ready'
import { HOVER_DELAY_MS } from '../lib/hoverPrefetch'
import { categoryUi, requestCreateCategory } from '../lib/categoryUi.svelte'
import { drafts, resetSaves } from '../lib/layoutDraft.svelte'
import { isCollapsed, resetCollapsed, setCollapsed } from '../lib/sidebarSections.svelte'
import { testReady } from '../lib/testReady'
import type { Category } from '../lib/protocol/Category'
import type { Channel } from '../lib/protocol/Channel'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
    { id: 'V2', server_id: 'S1', kind: 'voice', name: 'Games', position: 1 },
  ],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
}

let cmp: ReturnType<typeof mount> | undefined
const rows = (ch: string) => {
  const all = [...document.querySelectorAll('.row')]
  const start = all.findIndex((r) => r.textContent?.includes(ch))
  const out: Element[] = []
  // the row's chat icon (#226) sits between it and its occupants
  for (let el = all[start]?.nextElementSibling; el && el.matches('.occupant, .row-side'); el = el.nextElementSibling) if (el.matches('.occupant')) out.push(el)
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

describe('muted for you', () => {
  const lounge = () => Object.values(app.state.channels).find((c) => c.kind === 'voice')!
  const join = (u: string) => {
    app.state = applyEvent(app.state, { t: 'VoiceJoined', d: { channel_id: lounge().id, user_id: u, flags: { muted: false, deafened: false } } } as never, Date.now())
    flushSync()
  }
  const row = (u: string) => document.querySelector<HTMLElement>(`.occupant[data-user="${u}"]`)!

  it('a_person_you_muted_for_yourself_is_marked_in_the_voice_channel_list_and_says_so_to_screen_readers', () => {
    join('U2')
    expect(row('U2').querySelector('.for-me')).toBeNull()
    expect(row('U2').getAttribute('aria-label')).toBe('sam')
    voice.localMutes = { U2: true }
    flushSync()
    expect(row('U2').querySelector('.for-me svg')).not.toBeNull()
    // their own mute still shows beside it, and the row's name carries both (its content isn't read)
    app.state = applyEvent(app.state, { t: 'VoiceStateChanged', d: { channel_id: lounge().id, user_id: 'U2', flags: { muted: true, deafened: false } } } as never, Date.now())
    flushSync()
    expect(row('U2').querySelector('.flag:not(.for-me) svg')).not.toBeNull()
    expect(row('U2').getAttribute('aria-label')).toBe('sam, muted for you, muted')
  })

  it('you_are_never_marked_muted_for_yourself', () => {
    join('U1')
    voice.localMutes = { U1: true }
    flushSync()
    expect(row('U1').querySelector('.for-me')).toBeNull()
  })
})

// #219: a joiner's row arrives with a brief highlight, a leaver's fades out
describe('arrivals', () => {
  const lounge = () => Object.values(app.state.channels).find((c) => c.kind === 'voice')!
  const ev = (t: string, u: string) => {
    app.state = applyEvent(app.state, { t, d: { channel_id: lounge().id, user_id: u, flags: { muted: false, deafened: false } } } as never, Date.now())
    flushSync()
  }
  const row = (u: string) => document.querySelector<HTMLElement>(`.occupant[data-user="${u}"]`)

  it('someone_joining_a_channel_already_on_screen_arrives_with_the_highlight', () => {
    ev('VoiceJoined', 'U2')
    expect(row('U2')!.classList.contains('arrived')).toBe(true)
  })

  it('people_already_there_when_the_list_opens_do_not_flash', () => {
    if (cmp) unmount(cmp)
    document.body.innerHTML = ''
    app.state = applyEvent(app.state, { t: 'VoiceJoined', d: { channel_id: lounge().id, user_id: 'U2', flags: { muted: false, deafened: false } } } as never, Date.now())
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
    })
    flushSync()
    expect(row('U2')!.classList.contains('arrived')).toBe(false)
  })

  it('the_highlight_goes_once_it_has_played', () => {
    vi.useFakeTimers()
    try {
      ev('VoiceJoined', 'U2')
      vi.advanceTimersByTime(5000)
      flushSync()
      expect(row('U2')!.classList.contains('arrived')).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })

  it('the_entrance_slides_and_fades_on_the_motion_tokens_and_reduced_motion_or_a_paused_window_skips_it', () => {
    const css = readFileSync(resolve(__dirname, 'ChannelList.svelte'), 'utf8').match(/<style>([\s\S]*)<\/style>/)![1]
    expect(css).toMatch(/\.occupant\.arrived \{[^}]*animation: occupant-in var\(--dur-slow\) var\(--ease\)/)
    expect(css).toMatch(/@keyframes occupant-in \{ from \{ opacity: 0; transform: translateX\(/)
    // the highlight is a layer whose opacity fades, not a repainted background
    expect(css).toMatch(/\.occupant\.arrived::before \{[^}]*background: var\(--accent-soft\)[^}]*animation: occupant-glow/)
    expect(css).toMatch(/:global\(\.streaming\) \.occupant\.arrived[^{]*:global\(\.unfocused\) \.occupant\.arrived[^{]*\{[^}]*animation: none/)
    expect(css).toMatch(/prefers-reduced-motion: reduce\) \{[^}]*\.occupant\.arrived[^}]*animation: none/)
  })

  it('a_leavers_row_fades_out', () => {
    const src = readFileSync(resolve(__dirname, 'ChannelList.svelte'), 'utf8')
    expect(src).toMatch(/<div class="occupant"[^>]*out:exitFade/)
  })
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
    let joins = 0
    // the first join_voice is the one that gets cancelled; everything else (the second join, set_call_mix) hangs
    invoke.mockImplementation((c: string) => (c === 'join_voice' && joins++ === 0 ? new Promise((_, rej) => (rejectFirst = rej)) : new Promise(() => {})))
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

  it('a_join_the_core_cancels_on_its_own_frees_the_channel_for_another_click', async () => {
    let cancel!: (e: unknown) => void
    invoke.mockImplementation((c: string) => (c === 'join_voice' ? new Promise((_, rej) => (cancel = rej)) : Promise.resolve()))
    const joining = voiceApi.join('V1')
    await settle()
    cancel('cancelled') // e.g. signed out: no newer join replaced it
    await joining
    await settle()
    expect(voice.joining).toBeNull()
    expect(voice.joinFailed).toBeNull()
    expect(voice.stage).toEqual({ stage: 'disconnected', attempt: null })
    expect(document.querySelectorAll('.occupant.pending')).toHaveLength(0)
    void voiceApi.join('V1')
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'join_voice')).toHaveLength(2)
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

describe('ChannelList voice prefetch', () => {
  // the hover delay runs on fake timers; the row is re-mounted with a join spy
  function render(onJoinVoice = vi.fn()) {
    if (cmp) unmount(cmp)
    document.body.innerHTML = ''
    vi.useFakeTimers()
    invoke.mockResolvedValue(undefined)
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice, onMessage: () => {}, onVoiceSettings: () => {} },
    })
    flushSync()
    return [...document.querySelectorAll<HTMLButtonElement>('button.row')].find((b) => b.textContent?.includes('Lounge'))!
  }
  const prefetches = () => invoke.mock.calls.filter(([cmd]) => cmd === 'prefetch_voice').length
  afterEach(() => {
    vi.useRealTimers()
  })

  it('resting on a voice channel prefetches its token', async () => {
    const row = render()
    row.dispatchEvent(new MouseEvent('mouseenter'))
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
    expect(prefetches()).toBe(1)
  })

  it('clicking before the hover delay cancels the prefetch (the join fetches its own token)', async () => {
    const onJoinVoice = vi.fn()
    const row = render(onJoinVoice)
    row.dispatchEvent(new MouseEvent('mouseenter'))
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS / 2)
    row.click()
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS * 5)
    expect(onJoinVoice).toHaveBeenCalledWith('V1')
    expect(prefetches()).toBe(0)
  })
})

describe('ChannelList quotes group', () => {
  it('creates a quotes channel from the QUOTES group', async () => {
    const made = { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 2 }
    invoke.mockResolvedValue(made)
    const onSelect = vi.fn()
    unmount(cmp!)
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
    })
    flushSync()
    expect([...document.querySelectorAll('.label')].map((l) => l.textContent?.trim())).toContain('Quotes')
    document.querySelector<HTMLButtonElement>('button[aria-label="Create quotes channel"]')!.click()
    await tick()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="New quotes channel name"]')!
    input.value = 'Hall of Fame'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await tick()
    await tick()
    expect(invoke).toHaveBeenCalledWith('create_channel', { serverId: 'S1', kind: 'quotes', name: 'hall-of-fame' })
    expect(onSelect).toHaveBeenCalledWith('Q1')
  })

  it('lists quotes channels under the group and selects one', () => {
    app.state = applyReady(emptyState(), { ...ready, channels: [...ready.channels, { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }] })
    flushSync()
    const row = [...document.querySelectorAll<HTMLButtonElement>('.row')].find((r) => r.textContent?.includes('quotes'))
    expect(row).toBeTruthy()
  })
})

describe('ChannelList clips group', () => {
  it('creates a clips channel from the CLIPS group', async () => {
    const made = { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 3 }
    invoke.mockResolvedValue(made)
    const onSelect = vi.fn()
    unmount(cmp!)
    app.state = applyReady(emptyState(), { ...ready, channels: [...ready.channels, { id: 'K0', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }] })
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
    })
    flushSync()
    const labels = [...document.querySelectorAll('.label')].map((l) => l.textContent?.trim())
    expect(labels).toContain('Clips')
    expect(labels.indexOf('Clips')).toBe(labels.indexOf('Quotes') + 1)
    expect([...document.querySelectorAll('.row')].some((r) => r.textContent?.includes('highlights'))).toBe(true)
    const add = document.querySelector<HTMLButtonElement>('button[aria-label="Create clips channel"]')!
    expect(add.querySelector('path')?.getAttribute('d')).toContain('M4 4h16v16H4z') // the film icon
    add.click()
    await tick()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="New clips channel name"]')!
    input.value = 'Best Plays'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await tick()
    await tick()
    expect(invoke).toHaveBeenCalledWith('create_channel', { serverId: 'S1', kind: 'clips', name: 'best-plays' })
    expect(onSelect).toHaveBeenCalledWith('K1')
  })
})

describe('voice occupant names', () => {
  it('voice occupants show styled names and still avatars', () => {
    const A = 'a'.repeat(64)
    const S = 'b'.repeat(64)
    const styled = { ...sam, avatar_hash: A, avatar_still: S, style: { font: 'fredoka', colors: ['#ffc4a3'] } }
    window.dispatchEvent(new Event('focus'))
    app.state = applyReady(emptyState(), { ...ready, members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: styled, nickname: null }] }], voice: [{ channel_id: 'V1', members: [{ user_id: 'U2', flags: { muted: false, deafened: false } }] }] })
    flushSync()
    const occ = document.querySelector<HTMLElement>('.occupant[data-user="U2"]')!
    const name = occ.querySelector<HTMLElement>('.name')!
    expect(name.tagName).toBe('SPAN') // the row opens the participant menu; "View profile" is in it
    expect(name.textContent).toBe('sam')
    expect(name.getAttribute('style')).toContain('Fredoka')
    const img = occ.querySelector('img')!
    expect(img.getAttribute('src')).toBe(`pulse-media://localhost/${S}`)
    occ.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(img.getAttribute('src')).toBe(`pulse-media://localhost/${A}`)
    expect(occ.getAttribute('aria-label')).toBe('sam')
  })
})

// ---- By categories (#54) ----

const cat = (id: string, name: string, position: number, extra: Partial<Category> = {}): Category => ({ id, server_id: 'S1', name, position, ...extra })
const cats: Category[] = [cat('C1', 'Text Channels', 0, { default_kind: 'text' }), cat('C2', 'Gaming', 1), cat('C3', 'Voice Channels', 2, { default_kind: 'voice' })]
const live: Channel[] = [
  { id: 'T0', server_id: 'S1', kind: 'text', name: 'welcome', position: 0 },
  { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0, category_id: 'C1' },
  { id: 'T2', server_id: 'S1', kind: 'text', name: 'memes', position: 1, category_id: 'C1' },
  { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0, category_id: 'C2' },
  { id: 'V2', server_id: 'S1', kind: 'voice', name: 'Games', position: 1, category_id: 'C2' },
  { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0, category_id: 'C3' },
]
const layouts = [{ server_id: 'S1', revision: 3, categories: cats, channels: [] }]

function showLive(over: { member?: boolean; onSelect?: (id: string) => void; active?: string } = {}) {
  if (cmp) unmount(cmp)
  document.body.innerHTML = ''
  resetSaves()
  app.state = applyReady(emptyState(), testReady(live, { layouts, ...(over.member === false ? { members: [] } : {}) }))
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: over.active ?? null, voiceChannelId: null, speaking: new Set<string>(), onSelect: over.onSelect ?? (() => {}), onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
  })
  flushSync()
}
const headings = () => [...document.querySelectorAll<HTMLButtonElement>('button.section')]
const headingOf = (id: string) => document.querySelector<HTMLButtonElement>(`.cat[data-category="${id}"] > .label button.section`)!
const rowNames = (root: ParentNode = document) => [...root.querySelectorAll('.row .grow')].map((e) => e.textContent?.trim())
const rowOf = (id: string) => document.querySelector<HTMLElement>(`.row[data-channel="${id}"]`)!
const press = (el: Element, k: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init })
  el.dispatchEvent(e)
  flushSync()
  return e
}
const menuItems = () => [...document.querySelectorAll<HTMLButtonElement>('[role=menu] > [role=menuitem]')]
const menuItem = (label: string) => menuItems().find((b) => b.textContent?.trim() === label)!
const rightClick = (el: Element) => {
  el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }))
  flushSync()
}
const type = (input: HTMLInputElement, value: string) => {
  input.value = value
  input.dispatchEvent(new Event('input', { bubbles: true }))
}
const savedLayout = () => invoke.mock.calls.filter(([c]) => c === 'save_layout').map(([, a]) => (a as { layout: unknown }).layout)
const flush = async () => {
  for (let i = 0; i < 4; i++) await tick()
  flushSync()
}

describe('ChannelList by categories', () => {
  beforeEach(() => {
    // a save stays in flight: the draft keeps showing
    invoke.mockImplementation((c: string) => (c === 'save_layout' ? new Promise(() => {}) : Promise.resolve(undefined)))
  })
  afterEach(() => {
    resetSaves()
    categoryUi.create = null
  })

  it('shows the uncategorised channels on top, then each category in order with its channels', () => {
    showLive()
    expect(headings().map((h) => h.textContent?.trim())).toEqual(['Text Channels', 'Gaming', 'Voice Channels'])
    expect(rowNames(document.querySelector('.cat.top')!)).toEqual(['welcome'])
    expect(rowNames(document.querySelector('.cat[data-category="C1"]')!)).toEqual(['general', 'memes'])
    expect(rowNames(document.querySelector('.cat[data-category="C2"]')!)).toEqual(['quotes', 'Games'])
    expect(rowNames(document.querySelector('.cat[data-category="C3"]')!)).toEqual(['Lounge'])
    // each section is its heading then its rows
    expect(document.querySelector('.cat[data-category="C2"] > .label')).toBe(document.querySelector('.cat[data-category="C2"]')!.firstElementChild)
  })

  it('a heading shows the name as stored; CSS upper-cases it', () => {
    showLive()
    expect(headingOf('C2').textContent?.trim()).toBe('Gaming')
    const css = readFileSync(resolve(__dirname, 'ChannelList.svelte'), 'utf8').match(/<style>([\s\S]*)<\/style>/)![1]
    expect(css).toMatch(/\.label \{[^}]*text-transform: uppercase/)
  })

  it.each([
    ['Text', 'text', 'Hall Of Fame', 'hall-of-fame'],
    ['Quotes', 'quotes', 'Best Lines', 'best-lines'],
    ['Clips', 'clips', 'Best Plays', 'best-plays'],
    ['Voice', 'voice', 'Raid Room', 'Raid Room'],
  ])('+ creates a %s channel inside its category', async (label, kind, typed, name) => {
    const onSelect = vi.fn()
    showLive({ onSelect })
    invoke.mockImplementation(async (c: string) => (c === 'create_channel' ? { id: 'N1', server_id: 'S1', kind, name, position: 2, category_id: 'C2' } : undefined))
    document.querySelector<HTMLButtonElement>('button.add[aria-label="Create channel in Gaming"]')!.click()
    flushSync()
    const picker = document.querySelector<HTMLElement>('[role=radiogroup][aria-label="Channel kind"]')!
    const radios = [...picker.querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(radios.map((r) => r.getAttribute('aria-label'))).toEqual(['Text', 'Quotes', 'Clips', 'Voice'])
    expect(radios.map((r) => r.dataset.tip)).toEqual(['Text', 'Quotes', 'Clips', 'Voice'])
    expect(radios.find((r) => r.getAttribute('aria-checked') === 'true')?.getAttribute('aria-label')).toBe('Text')
    radios.find((r) => r.getAttribute('aria-label') === label)!.click()
    flushSync()
    expect(radios.find((r) => r.getAttribute('aria-checked') === 'true')?.getAttribute('aria-label')).toBe(label)
    // the field sits in the category, under its heading
    const input = document.querySelector<HTMLInputElement>('.cat[data-category="C2"] input[aria-label="New channel name"]')!
    expect(input.classList.contains('input-sm')).toBe(true)
    type(input, typed)
    press(input, 'Enter')
    await flush()
    expect(invoke).toHaveBeenCalledWith('create_channel', { serverId: 'S1', kind, name, categoryId: 'C2' })
    expect(document.querySelector('input[aria-label="New channel name"]')).toBeNull()
    if (kind === 'voice') expect(onSelect).not.toHaveBeenCalled()
    else expect(onSelect).toHaveBeenCalledWith('N1')
  })

  it('the arrow keys move between kinds; Escape or focus leaving the field cancels', () => {
    showLive()
    document.querySelector<HTMLButtonElement>('button.add[aria-label="Create channel in Gaming"]')!.click()
    flushSync()
    const input = () => document.querySelector<HTMLInputElement>('input[aria-label="New channel name"]')
    const radio = (l: string) => document.querySelector<HTMLButtonElement>(`[role=radio][aria-label="${l}"]`)!
    press(radio('Text'), 'ArrowRight')
    expect(radio('Quotes').getAttribute('aria-checked')).toBe('true')
    expect(document.activeElement).toBe(radio('Quotes'))
    // focus moving within the field and its picker keeps it
    input()!.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: radio('Clips') }))
    flushSync()
    expect(input()).not.toBeNull()
    press(input()!, 'Escape')
    expect(input()).toBeNull()
    document.querySelector<HTMLButtonElement>('button.add[aria-label="Create channel in Gaming"]')!.click()
    flushSync()
    input()!.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: document.body }))
    flushSync()
    expect(input()).toBeNull()
    expect(invoke.mock.calls.some(([c]) => c === 'create_channel')).toBe(false)
  })

  it("a category's menu creates a channel in it, renames it in its heading and deletes it", async () => {
    showLive()
    rightClick(headingOf('C2'))
    expect(menuItems().map((b) => b.textContent?.trim())).toEqual(['Create channel', 'Rename category…', 'Delete category', 'Move up', 'Move down'])
    menuItem('Create channel').click()
    flushSync()
    expect(document.querySelector('.cat[data-category="C2"] input[aria-label="New channel name"]')).not.toBeNull()
    press(document.querySelector('input[aria-label="New channel name"]')!, 'Escape')

    rightClick(headingOf('C2'))
    menuItem('Rename category…').click()
    flushSync()
    const field = document.querySelector<HTMLInputElement>('.cat[data-category="C2"] > .label input[aria-label="Rename category Gaming"]')!
    expect(field.value).toBe('Gaming')
    invoke.mockRejectedValueOnce('That name is too long')
    type(field, 'Games')
    press(field, 'Enter')
    await flush()
    expect(invoke).toHaveBeenCalledWith('rename_category', { categoryId: 'C2', name: 'Games' })
    expect(document.querySelector('.cat[data-category="C2"] .err')?.textContent).toBe('That name is too long')
    invoke.mockResolvedValueOnce(cat('C2', 'Games', 1))
    press(field, 'Enter')
    await flush()
    expect(document.querySelector('input[aria-label="Rename category Gaming"]')).toBeNull()
    expect(document.activeElement).toBe(headingOf('C2'))

    rightClick(headingOf('C2'))
    menuItem('Delete category').click()
    await flush()
    expect(invoke).toHaveBeenCalledWith('delete_category', { categoryId: 'C2' })
  })

  it('Escape leaves a rename without saving', () => {
    showLive()
    rightClick(headingOf('C1'))
    menuItem('Rename category…').click()
    flushSync()
    const field = document.querySelector<HTMLInputElement>('input[aria-label="Rename category Text Channels"]')!
    type(field, 'Chat')
    press(field, 'Escape')
    expect(document.querySelector('input[aria-label="Rename category Text Channels"]')).toBeNull()
    expect(invoke.mock.calls.some(([c]) => c === 'rename_category')).toBe(false)
  })

  it('a category deleted meanwhile closes its menu and its rename', () => {
    showLive()
    rightClick(headingOf('C2'))
    expect(menuItems().length).toBeGreaterThan(0)
    app.state = applyLayout(app.state, { server_id: 'S1', revision: 4, categories: [cats[0], cats[2]], channels: [] })
    flushSync()
    expect(document.querySelector('[role=menu]')).toBeNull()
    rightClick(headingOf('C1'))
    menuItem('Rename category…').click()
    flushSync()
    app.state = applyLayout(app.state, { server_id: 'S1', revision: 5, categories: [cats[2]], channels: [] })
    flushSync()
    expect(document.querySelector('input[aria-label^="Rename category"]')).toBeNull()
  })

  it("Move up / Move down in a category's menu reorder the categories (disabled at the ends)", async () => {
    showLive()
    rightClick(headingOf('C1'))
    expect(menuItem('Move up').disabled).toBe(true)
    menuItem('Move down').click()
    await flush()
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C2', channels: ['Q1', 'V2'] }, { id: 'C1', channels: ['T1', 'T2'] }, { id: 'C3', channels: ['V1'] }] }])
    // the draft shows at once
    expect(headings().map((h) => h.textContent?.trim())).toEqual(['Gaming', 'Text Channels', 'Voice Channels'])
    rightClick(headingOf('C3'))
    expect(menuItem('Move down').disabled).toBe(true)
  })

  it('Alt+Down on a channel moves it, into the next category at the edge; focus stays on it', async () => {
    showLive()
    rowOf('T2').focus()
    press(rowOf('T2'), 'ArrowDown', { altKey: true })
    await flush()
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T1'] }, { id: 'C2', channels: ['T2', 'Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }] }])
    expect(rowNames(document.querySelector('.cat[data-category="C2"]')!)).toEqual(['memes', 'quotes', 'Games'])
    expect(document.activeElement).toBe(rowOf('T2'))
    // up out of the first category into the top area
    rowOf('T1').focus()
    press(rowOf('T1'), 'ArrowUp', { altKey: true })
    await flush()
    expect(rowNames(document.querySelector('.cat.top')!)).toEqual(['welcome', 'general'])
    expect(document.activeElement).toBe(rowOf('T1'))
    // a voice row moves too
    press(rowOf('V1'), 'ArrowUp', { altKey: true })
    await flush()
    expect(rowNames(document.querySelector('.cat[data-category="C2"]')!)).toEqual(['memes', 'quotes', 'Games', 'Lounge'])
    // at the very top nothing moves
    const before = savedLayout().length
    press(rowOf('T0'), 'ArrowUp', { altKey: true })
    expect(savedLayout().length).toBe(before)
  })

  it('Alt+Up on a heading moves the category; focus stays on its heading', async () => {
    showLive()
    headingOf('C3').focus()
    press(headingOf('C3'), 'ArrowUp', { altKey: true })
    await flush()
    expect(headings().map((h) => h.textContent?.trim())).toEqual(['Text Channels', 'Voice Channels', 'Gaming'])
    expect(document.activeElement).toBe(headingOf('C3'))
  })

  it('Alt+Down on a row in a collapsed category opens it, so the move shows', async () => {
    localStorage.clear()
    resetCollapsed()
    setCollapsed('cat:C1', true)
    // the channel you're on stays shown in a collapsed category
    showLive({ active: 'T1' })
    expect(rowNames(document.querySelector('.cat[data-category="C1"]')!)).toEqual(['general'])
    rowOf('T1').focus()
    press(rowOf('T1'), 'ArrowDown', { altKey: true })
    await flush()
    expect(isCollapsed('cat:C1')).toBe(false)
    expect(rowNames(document.querySelector('.cat[data-category="C1"]')!)).toEqual(['memes', 'general'])
    expect(document.activeElement).toBe(rowOf('T1'))
    localStorage.clear()
    resetCollapsed()
  })

  it('a refused move snaps back with focus still on what moved', async () => {
    showLive()
    invoke.mockImplementation((c: string) => (c === 'save_layout' ? Promise.reject('Someone else changed the list first') : Promise.resolve(undefined)))
    rowOf('T2').focus()
    press(rowOf('T2'), 'ArrowDown', { altKey: true })
    await flush()
    await flush()
    // back in its own category (a new element again), and still focused
    expect(rowNames(document.querySelector('.cat[data-category="C1"]')!)).toEqual(['general', 'memes'])
    expect(document.activeElement).toBe(rowOf('T2'))
    headingOf('C3').focus()
    press(headingOf('C3'), 'ArrowUp', { altKey: true })
    await flush()
    await flush()
    expect(headings().map((h) => h.textContent?.trim())).toEqual(['Text Channels', 'Gaming', 'Voice Channels'])
    expect(document.activeElement).toBe(headingOf('C3'))
  })

  it("Move up / Move down in a channel's menu", async () => {
    showLive()
    rightClick(rowOf('Q1'))
    expect(menuItems().map((b) => b.textContent?.trim()).slice(-2)).toEqual(['Move up', 'Move down'])
    menuItem('Move up').click()
    await flush()
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T1', 'T2', 'Q1'] }, { id: 'C2', channels: ['V2'] }, { id: 'C3', channels: ['V1'] }] }])
    rightClick(rowOf('T0'))
    expect(menuItem('Move up').disabled).toBe(true)
    expect(menuItem('Move down').disabled).toBe(false)
  })

  it('outside the server: no +, no category menu, no moves', () => {
    showLive({ member: false })
    expect(document.querySelector('button.add')).toBeNull()
    rightClick(headingOf('C2'))
    expect(document.querySelector('[role=menu]')).toBeNull()
    press(headingOf('C2'), 'F10', { shiftKey: true })
    expect(document.querySelector('[role=menu]')).toBeNull()
    press(rowOf('T1'), 'ArrowDown', { altKey: true })
    press(headingOf('C1'), 'ArrowDown', { altKey: true })
    expect(savedLayout()).toEqual([])
    rightClick(rowOf('T1'))
    expect(menuItems().map((b) => b.textContent?.trim())).not.toContain('Move up')
  })

  it('Create category opens a heading-styled field at the end of the list', async () => {
    showLive()
    requestCreateCategory('S2') // another server's: not this list's
    flushSync()
    expect(document.querySelector('input[aria-label="New category name"]')).toBeNull()
    // and dropped, so it can't pop open later when S2's list shows
    expect(categoryUi.create).toBeNull()
    requestCreateCategory('S1')
    flushSync()
    expect(categoryUi.create).toBeNull()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="New category name"]')!
    expect(input.closest('.label')).not.toBeNull()
    expect(input.closest('.list')!.lastElementChild!.contains(input)).toBe(true)
    expect(document.activeElement).toBe(input)
    invoke.mockResolvedValueOnce(cat('C9', 'Raid Night', 3))
    type(input, '  Raid Night ')
    press(input, 'Enter')
    await flush()
    expect(invoke).toHaveBeenCalledWith('create_category', { serverId: 'S1', name: 'Raid Night' })
    expect(document.querySelector('input[aria-label="New category name"]')).toBeNull()
    requestCreateCategory('S1')
    flushSync()
    press(document.querySelector('input[aria-label="New category name"]')!, 'Escape')
    expect(document.querySelector('input[aria-label="New category name"]')).toBeNull()
  })

  it('without layouts (an older server) the four kind sections stay, each with its own +', () => {
    showLive()
    app.state = applyReady(emptyState(), testReady(live.map(({ category_id: _, ...c }) => c)))
    flushSync()
    expect(headings().map((h) => h.textContent?.trim())).toEqual(['Text Channels', 'Quotes', 'Clips', 'Voice Channels'])
    expect([...document.querySelectorAll('button.add')].map((b) => b.getAttribute('aria-label')))
      .toEqual(['Create text channel', 'Create quotes channel', 'Create clips channel', 'Create voice channel'])
    expect(document.querySelector('[data-category]')).toBeNull()
    rightClick(headings()[0])
    expect(document.querySelector('[role=menu]')).toBeNull()
  })
})
