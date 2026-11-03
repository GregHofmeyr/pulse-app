// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Sound } from '../lib/protocol/Sound'

const invoke = vi.hoisted(() => vi.fn(async (_c: string, _a?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import SoundDialog from './SoundDialog.svelte'
import SoundTidy from './SoundTidy.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { soundboard } from '../lib/soundboard.svelte'
import { setNotch } from '../lib/mixer.svelte'

const peaks = Array.from({ length: 200 }, (_, i) => (i % 10) / 10)
const snd = (id: string, name: string, plays: number): Sound => ({
  id, server_id: 'S1', name, emoji: null, media_hash: 'h' + id, volume: 100, duration_ms: 2000, added_by: null,
  created_at: `2026-10-0${plays + 1}T00:00:00.000Z`, plays,
})
let answers: Record<string, unknown> = {}
let cmp: ReturnType<typeof mount> | undefined
const onClose = vi.fn()
const calls = (c: string) => invoke.mock.calls.filter((x) => x[0] === c)
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  flushSync()
}
const q = <T extends Element>(s: string) => document.querySelector<T>(s)!
const button = (t: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === t)!
const fill = (label: string, v: string) => {
  const el = q<HTMLInputElement>(`[aria-label="${label}"]`)
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const key = (el: Element, k: string) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  flushSync()
}
async function openAdd(duration = 20000) {
  answers['soundboard_open'] = { duration_ms: duration, peaks }
  cmp = mount(SoundDialog, { target: document.body, props: { mode: { kind: 'add', serverId: 'S1' }, onClose } })
  await settle()
  button('Choose a file').click()
  await settle()
}

beforeEach(() => {
  onClose.mockReset()
  answers = {
    soundboard_support: { can_send: true, reason: null },
    soundboard_pick_file: '/tmp/a.mp3',
    soundboard_upload: { hash: 'HASH' },
    create_sound: snd('n', 'Boom', 0),
    update_sound: snd('a', 'Boom', 0),
  }
  invoke.mockReset()
  invoke.mockImplementation((async (c: string) => {
    const a = answers[c]
    if (a instanceof Error) throw a
    return a
  }) as never)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('SoundDialog add', () => {
  it('shows the reason when this install cannot send', async () => {
    answers['soundboard_support'] = { can_send: false, reason: 'ffmpeg is missing' }
    cmp = mount(SoundDialog, { target: document.body, props: { mode: { kind: 'add', serverId: 'S1' }, onClose } })
    await settle()
    expect(document.body.textContent).toContain('ffmpeg is missing')
    expect(document.body.textContent).not.toContain('Choose a file')
  })

  it('Save is disabled without a file, then without a name', async () => {
    cmp = mount(SoundDialog, { target: document.body, props: { mode: { kind: 'add', serverId: 'S1' }, onClose } })
    await settle()
    expect(button('Save').disabled).toBe(true)
    unmount(cmp)
    cmp = undefined
    document.body.innerHTML = ''
    await openAdd()
    expect(button('Save').disabled).toBe(true)
    fill('Name', 'Boom')
    expect(button('Save').disabled).toBe(false)
  })

  it('starts on the first 6 s and the end handle cannot stretch it past 6 s', async () => {
    await openAdd()
    const start = q('[role=slider][aria-label="Start"]')
    const end = q('[role=slider][aria-label="End"]')
    expect(start.getAttribute('aria-valuenow')).toBe('0')
    expect(end.getAttribute('aria-valuenow')).toBe('6000')
    key(end, 'ArrowRight')
    expect(end.getAttribute('aria-valuenow')).toBe('6100')
    expect(q('[role=slider][aria-label="Start"]').getAttribute('aria-valuenow')).toBe('100')
    expect(document.body.textContent).toContain('6.0 s')
  })

  it('a handle dragged past 6 s drags the other along', async () => {
    await openAdd()
    const svg = q<SVGSVGElement>('svg.wave')
    svg.getBoundingClientRect = () => ({ left: 0, width: 200, top: 0, height: 60, right: 200, bottom: 60, x: 0, y: 0, toJSON() {} }) as DOMRect
    const end = q('[role=slider][aria-label="End"]')
    end.setPointerCapture = () => {}
    end.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, clientX: 60 }))
    end.dispatchEvent(new MouseEvent('pointermove', { bubbles: true, clientX: 120 })) // 60% of 20 s = 12 s
    end.dispatchEvent(new MouseEvent('pointerup', { bubbles: true, clientX: 120 }))
    flushSync()
    expect(end.getAttribute('aria-valuenow')).toBe('12000')
    expect(q('[role=slider][aria-label="Start"]').getAttribute('aria-valuenow')).toBe('6000')
  })

  it('uploads the span, then creates the sound with name, emoji and boost', async () => {
    await openAdd()
    fill('Name', '  Boom ')
    fill('Emoji', '💥')
    fill('Boost', '150')
    button('Save').click()
    await settle()
    expect(calls('soundboard_upload')[0][1]).toEqual({ path: '/tmp/a.mp3', startMs: 0, endMs: 6000 })
    expect(calls('create_sound')[0][1]).toEqual({ serverId: 'S1', name: 'Boom', emoji: '💥', mediaHash: 'HASH', volume: 150 })
    expect(onClose).toHaveBeenCalled()
  })

  it('previews the span locally', async () => {
    await openAdd()
    button('Preview').click()
    await settle()
    expect(calls('soundboard_preview')[0][1]).toMatchObject({ path: '/tmp/a.mp3', startMs: 0, endMs: 6000, soundPercent: 100 })
  })

  it('previews at the Calls and Soundboard levels', async () => {
    setNotch('calls', 'master', 70)
    setNotch('calls', 'soundboard', 130)
    try {
      await openAdd()
      button('Preview').click()
      await settle()
      expect(calls('soundboard_preview')[0][1]).toMatchObject({ callsPercent: 70, soundboardPercent: 130 })
    } finally {
      setNotch('calls', 'master', 100)
      setNotch('calls', 'soundboard', 100)
    }
  })

  it('previews at 100 when the soundboard volume is 0', async () => {
    soundboard.volume_pct = 0
    try {
      await openAdd()
      button('Preview').click()
      await settle()
      expect(calls('soundboard_preview')[0][1]).toMatchObject({ soundPercent: 100, callsPercent: 100, soundboardPercent: 100 })
    } finally {
      soundboard.volume_pct = 100
    }
  })

  it('shows a refusal from the server inline and stays open', async () => {
    await openAdd()
    fill('Name', 'Boom')
    answers['create_sound'] = 'that emoji is not allowed'
    invoke.mockImplementation((async (c: string) => {
      if (c === 'create_sound') throw 'that emoji is not allowed'
      return answers[c]
    }) as never)
    button('Save').click()
    await settle()
    expect(q('[role=alert]').textContent).toContain('that emoji is not allowed')
    expect(onClose).not.toHaveBeenCalled()
  })
})

describe('SoundDialog edit', () => {
  it('updates name, emoji and boost and never uploads', async () => {
    cmp = mount(SoundDialog, { target: document.body, props: { mode: { kind: 'edit', sound: snd('a', 'Airhorn', 1) }, onClose } })
    await settle()
    expect(document.body.textContent).not.toContain('Choose a file')
    expect(document.querySelector('svg.wave')).toBeNull()
    fill('Name', 'Boom')
    fill('Emoji', '')
    button('Save').click()
    await settle()
    expect(calls('update_sound')[0][1]).toEqual({ id: 'a', request: { name: 'Boom', emoji: null, volume: 100 } })
    expect(calls('soundboard_upload')).toHaveLength(0)
    expect(onClose).toHaveBeenCalled()
  })
})

describe('SoundTidy', () => {
  const seven = () => [0, 1, 2, 3, 4, 5, 6].map((n) => snd('s' + n, 'Sound' + n, n))
  const rows = () => [...document.querySelectorAll('li')]
  beforeEach(() => {
    app.state = emptyState()
    app.state.sounds = Object.fromEntries(seven().map((x) => [x.id, x]))
  })

  it('a row deleted by someone else meanwhile goes quietly', async () => {
    answers['list_sounds'] = seven()
    cmp = mount(SoundTidy, { target: document.body, props: { serverId: 'S1', onClose } })
    await settle()
    invoke.mockImplementation((async (c: string) => {
      if (c === 'delete_sound') throw 'not found'
      return answers[c]
    }) as never)
    rows()[0].querySelector<HTMLButtonElement>('button')!.click()
    await settle()
    expect(document.querySelector('[role=alert]')).toBeNull()
    expect(rows().some((r) => r.textContent?.includes('Sound0'))).toBe(false)
  })

  it('drops a row when its sound is deleted elsewhere', async () => {
    answers['list_sounds'] = seven()
    cmp = mount(SoundTidy, { target: document.body, props: { serverId: 'S1', onClose } })
    await settle()
    expect(rows()[1].textContent).toContain('Sound1')
    // a SoundDeleted arrived (the store is replaced, never mutated)
    const { s1: _gone, ...sounds } = app.state.sounds
    app.state = { ...app.state, sounds }
    await settle()
    expect(rows().some((r) => r.textContent?.includes('Sound1'))).toBe(false)
    expect(rows()).toHaveLength(4)
  })

  it('lists the 5 least played, and Delete removes the row', async () => {
    answers['list_sounds'] = [0, 1, 2, 3, 4, 5, 6].map((n) => snd('s' + n, 'Sound' + n, n))
    cmp = mount(SoundTidy, { target: document.body, props: { serverId: 'S1', onClose } })
    await settle()
    const rows = () => [...document.querySelectorAll('li')]
    expect(rows()).toHaveLength(5)
    expect(rows()[0].textContent).toContain('Sound0')
    expect(rows()[1].textContent).toContain('1 play')
    expect(rows()[2].textContent).toContain('2 plays')
    rows()[0].querySelector<HTMLButtonElement>('button')!.click()
    await settle()
    expect(calls('delete_sound')[0][1]).toEqual({ id: 's0' })
    expect(rows().some((r) => r.textContent?.includes('Sound0'))).toBe(false)
  })
})
