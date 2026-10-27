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
  channels: [
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
    { id: 'V2', server_id: 'S1', kind: 'voice', name: 'Games', position: 1 },
  ],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
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
    document.querySelector('[role=dialog]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    await tick()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(watchCalls()).toEqual([{ on: false }])
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

const esc = (el: Element, init: KeyboardEventInit = {}) => el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, ...init }))

describe('VoiceStatus panel behaviour', () => {
  it('opening_moves_focus_into_the_panel_and_escape_returns_it_to_the_trigger', async () => {
    await open()
    const dlg = document.querySelector<HTMLElement>('[role=dialog]')!
    expect(dlg.contains(document.activeElement)).toBe(true)
    esc(document.activeElement!)
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(document.activeElement).toBe(trigger())
  })

  it('escape_meant_for_something_else_does_not_close_the_panel', async () => {
    await open()
    esc(document.body)
    flushSync()
    expect(document.querySelector('[role=dialog]')).not.toBeNull()
    const dlg = document.querySelector('[role=dialog]')!
    const ev = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    const input = document.createElement('input')
    dlg.appendChild(input)
    input.addEventListener('keydown', (e) => e.preventDefault())
    input.dispatchEvent(ev)
    flushSync()
    expect(document.querySelector('[role=dialog]')).not.toBeNull()
  })

  it('clicking_the_trigger_again_closes_the_panel', async () => {
    await open()
    invoke.mockClear()
    trigger()!.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    trigger()!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(watchCalls()).toEqual([{ on: false }])
  })

  it('closing_while_unfocused_emits_no_extra_false', async () => {
    await open()
    windowFocus.focused = false
    flushSync()
    invoke.mockClear()
    trigger()!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(watchCalls()).toEqual([])
  })

  it('reconnecting_while_open_closes_the_panel_and_stops_polling', async () => {
    await open()
    invoke.mockClear()
    emit({ kind: 'stage', stage: 'reconnecting', attempt: 2 })
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(trigger()).toBeNull()
    expect(document.body.textContent).toContain('Reconnecting… (attempt 2)')
    expect(watchCalls()).toEqual([{ on: false }])
  })

  it('reconnecting_with_focus_in_the_panel_keeps_focus_on_the_status', async () => {
    await open()
    expect(document.querySelector('[role=dialog]')!.contains(document.activeElement)).toBe(true)
    emit({ kind: 'stage', stage: 'reconnecting', attempt: 2 })
    flushSync()
    const status = document.querySelector<HTMLElement>('span.vc-main')!
    expect(status.tabIndex).toBe(-1)
    expect(document.activeElement).toBe(status)
  })

  it('focus_on_leave_survives_reconnecting', () => {
    connect()
    const leave = document.querySelector<HTMLButtonElement>('button.vc-leave')!
    leave.focus()
    emit({ kind: 'stage', stage: 'reconnecting', attempt: 2 })
    flushSync()
    expect(document.activeElement).toBe(leave)
  })

  it('reconnecting_does_not_take_focus_from_elsewhere', async () => {
    await open()
    const input = document.createElement('input')
    document.body.appendChild(input)
    input.focus()
    emit({ kind: 'stage', stage: 'reconnecting', attempt: 2 })
    flushSync()
    expect(document.activeElement).toBe(input)
  })

  it('leaving_clears_the_report_so_the_next_call_starts_empty', async () => {
    await open()
    emit(link)
    flushSync()
    expect(document.querySelector('.ping')?.textContent).toContain('150')
    emit({ kind: 'state', channel_id: null, connection: 'disconnected', controls: { muted: false, deafened: false } })
    flushSync()
    expect(voice.link).toBeNull()
    await open()
    expect(document.querySelector('.ping')?.textContent).toBe('—')
    expect(document.querySelector('.copy')?.hasAttribute('disabled')).toBe(true)
    expect(document.body.textContent).not.toContain('voice.example.test')
  })

  it('a_failed_watch_command_is_not_an_unhandled_rejection', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const unhandled = vi.fn()
    process.on('unhandledRejection', unhandled)
    invoke.mockImplementation((c: string) => (c === 'watch_voice_link' ? Promise.reject('boom') : Promise.resolve()))
    await open()
    await new Promise((r) => setTimeout(r, 10))
    process.off('unhandledRejection', unhandled)
    expect(unhandled).not.toHaveBeenCalled()
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it('switching_channel_names_the_channel_being_joined', () => {
    connect()
    voice.joining = 'V2'
    voice.stage = { stage: 'connecting', attempt: null }
    flushSync()
    expect(document.querySelector('.vc-text small')?.textContent).toBe('Games')
  })

  it('the_failed_state_can_be_dismissed', () => {
    voice.joinFailed = { channelId: 'V1', error: 'nope' }
    flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label=Dismiss]')!.click()
    flushSync()
    expect(voice.joinFailed).toBeNull()
    expect(document.querySelector('.vc')).toBeNull()
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
