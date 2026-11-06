// @vitest-environment jsdom
// The live card says where the stream's sound comes from, and offers a switch when This app stays silent (#215).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => null, pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import LiveCard from './LiveCard.svelte'
import { stream } from '../lib/stream/stream.svelte'
import type { SoundStatus } from '../lib/stream/sound'

const live = (audio = true) => ({ channelId: 'V1', res: 720 as const, fps: 60 as const, audio, audioMuted: false, label: 'Game', encoderWarning: false, allowGhosts: true, sound: 'app' as const })
const status = (s: Partial<SoundStatus>): SoundStatus => ({ tier: 'app', app: 'Firefox', heard: false, ms: 1000, ...s })

let c: ReturnType<typeof mount> | null = null
const open = () => {
  c = mount(LiveCard, { target: document.body, props: { channelId: 'V1', serverId: null } })
  flushSync()
}
const meta = () => document.querySelector('.meta')!.textContent!.replace(/\s+/g, ' ').trim()
const note = () => document.querySelector('[data-note="no-sound"]')
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve()
  flushSync()
}

describe('LiveCard sound', () => {
  beforeEach(() => {
    globalThis.MediaStream ??= class {} as never
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
    document.body.innerHTML = ''
  })
  afterEach(() => {
    if (c) unmount(c)
    c = null
    stream.live = null
    stream.sound = null
  })

  it('shows_the_source_in_use', () => {
    stream.live = live()
    stream.sound = status({})
    open()
    expect(meta()).toBe('720p60 · Sound: Firefox')
    stream.sound = status({ tier: 'everything', app: null })
    flushSync()
    expect(meta()).toBe('720p60 · Sound: everything except Pulse')
  })

  it('says_when_the_last_resort_is_in_use', () => {
    stream.live = live()
    stream.sound = status({ tier: 'lastResort', app: null })
    open()
    expect(meta()).toBe('720p60 · Sound: everything except Pulse and music apps')
    expect(document.querySelector('.meta [data-tip]')!.getAttribute('data-tip')).toContain("Couldn't tell which app owns this window")
  })

  it('says_no_sound_when_shared_without_it', () => {
    stream.live = live(false)
    open()
    expect(meta()).toBe('720p60 · No sound')
  })

  it('a_silent_app_gets_a_gentle_note_with_a_one_click_switch', async () => {
    stream.live = live()
    stream.sound = status({ ms: 19_999 })
    open()
    expect(note()).toBeNull()
    stream.sound = status({ ms: 20_000 })
    flushSync()
    expect(note()!.textContent).toContain('No sound from this app yet')
    const btn = note()!.querySelector('button')!
    expect(btn.textContent!.trim()).toBe('Use everything except Pulse')
    btn.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('stream_audio_sound', { sound: 'everything' })
    expect(stream.live!.sound).toBe('everything')
  })

  it('no_note_once_heard_or_for_everything', () => {
    stream.live = live()
    stream.sound = status({ ms: 30_000, heard: true })
    open()
    expect(note()).toBeNull()
    stream.sound = status({ ms: 30_000, tier: 'everything', app: null })
    flushSync()
    expect(note()).toBeNull()
  })
})
