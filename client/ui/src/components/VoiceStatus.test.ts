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

import VoiceStatus from './VoiceStatus.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { startVoiceListening, voice } from '../lib/voice.svelte'
import { windowFocus } from '../lib/windowFocus.svelte'
import type { Ready } from '../lib/protocol/Ready'

const me = { id: 'U1', username: 'alex', avatar_hash: null }
const ready: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [],
}
const link = { kind: 'link', rtt_ms: 150, jitter_ms: 4, loss_pct: 0, transport: 'udp', server: 'voice.example.test', version: '1.0.0', history: [[1000, 150]] }

let cmp: ReturnType<typeof mount> | undefined
const watchCalls = () => invoke.mock.calls.filter((c) => c[0] === 'watch_voice_link').map((c) => c[1])

function connect() {
  emit({ kind: 'state', channel_id: 'V1', connection: 'connected', controls: { muted: false, deafened: false } })
  emit({ kind: 'stage', stage: 'connected', attempt: null })
  flushSync()
}

beforeEach(async () => {
  await startVoiceListening()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  app.state = applyReady(emptyState(), ready)
  voice.channelId = null
  voice.joining = null
  voice.leaving = null
  voice.joinFailed = null
  voice.stage = { stage: 'disconnected', attempt: null }
  voice.link = null
  voice.connection = 'disconnected'
  windowFocus.focused = true
  cmp = mount(VoiceStatus, { target: document.body })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const trigger = () => document.querySelector<HTMLButtonElement>('button[aria-haspopup=dialog]')

async function open() {
  connect()
  trigger()!.click()
  flushSync()
  await tick()
  await new Promise((r) => setTimeout(r)) // clickOutside arms its listeners a macrotask after mount
}

describe('VoiceStatus', () => {
  it('shows_nothing_when_not_in_voice', () => {
    expect(document.querySelector('.vc')).toBeNull()
  })

  it('shows_the_stage_while_joining', () => {
    voice.joining = 'V1'
    voice.stage = { stage: 'connecting', attempt: null }
    flushSync()
    expect(document.body.textContent).toContain('Connecting to voice')
    expect(document.body.textContent).toContain('Lounge')
    expect(trigger()).toBeNull()
    expect(document.querySelector('.vc-text strong.warn')).not.toBeNull()
  })

  it('connected_becomes_a_button_that_opens_the_panel', async () => {
    connect()
    expect(trigger()).not.toBeNull()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    trigger()!.click()
    flushSync()
    await tick()
    expect(document.querySelector('[role=dialog][aria-label="Connection details"]')).not.toBeNull()
    expect(invoke).toHaveBeenCalledWith('watch_voice_link', { on: true })
  })

  it('closing_the_panel_stops_link_polling', async () => {
    await open()
    invoke.mockClear()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    flushSync()
    await tick()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(invoke).toHaveBeenCalledWith('watch_voice_link', { on: false })
  })

  it('unfocused_window_stops_link_polling', async () => {
    await open()
    invoke.mockClear()
    windowFocus.focused = false
    flushSync()
    expect(watchCalls()).toEqual([{ on: false }])
    windowFocus.focused = true
    flushSync()
    expect(watchCalls()).toEqual([{ on: false }, { on: true }])
  })

  it('failed_join_shows_retry', async () => {
    voice.joinFailed = { channelId: 'V1', error: "couldn't connect to voice: x" }
    flushSync()
    expect(document.body.textContent).toContain("couldn't connect to voice: x")
    const retry = [...document.querySelectorAll('button')].find((b) => b.textContent?.includes('Retry'))!
    retry.click()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('join_voice', expect.objectContaining({ channelId: 'V1' }))
  })

  it('values_are_coloured_by_level', async () => {
    await open()
    emit(link)
    flushSync()
    expect(document.querySelector('.ping')?.classList.contains('warn')).toBe(true)
    expect(document.querySelector('.ping')?.textContent).toContain('150')
    expect(document.querySelector('.jitter')?.classList.contains('good')).toBe(true)
  })

  it('copy_details_writes_the_report_text', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    await open()
    emit(link)
    flushSync()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent?.includes('Copy details'))!.click()
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining('Ping: 150 ms'))
  })
})

describe('LinkStatsPanel dismissal', () => {
  it('a_click_outside_closes_the_panel', async () => {
    await open()
    document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
  })
})
