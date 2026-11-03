// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Sound } from '../lib/protocol/Sound'

const invoke = vi.hoisted(() => vi.fn(async (_c: string, _a?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import SoundboardSettings from './SoundboardSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { reloadMixer } from '../lib/mixer.svelte'

const server = { id: 'S1', name: 'Home', icon_hash: null }
const snd = (id: string, name: string, over: Partial<Sound> = {}): Sound => ({
  id, server_id: 'S1', name, emoji: '🎺', emoji_id: null, media_hash: 'h' + id, volume: 120, duration_ms: 2400,
  added_by: 'U2', created_at: '2026-10-01T00:00:00.000Z', plays: 3, ...over,
})

let cmp: ReturnType<typeof mount> | undefined
const settle = async () => {
  for (let i = 0; i < 4; i++) await tick()
  flushSync()
}
const calls = (c: string) => invoke.mock.calls.filter((x) => x[0] === c)
const rows = () => [...document.querySelectorAll<HTMLElement>('li')]
const row = (name: string) => rows().find((r) => r.textContent?.includes(name))!
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const text = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === t)!

beforeEach(() => {
  app.state = {
    ...emptyState(),
    sounds: { a: snd('a', 'Airhorn'), b: snd('b', 'Bruh', { plays: 1, duration_ms: 600, added_by: null }), x: snd('x', 'Elsewhere', { server_id: 'S2' }) },
    members: { S1: [{ user: { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, nickname: null }] },
  } as never
  localStorage.clear()
  reloadMixer()
  invoke.mockReset()
  invoke.mockImplementation((async (c: string) => (c === 'soundboard_support' ? { can_send: true, reason: null } : undefined)) as never)
  cmp = mount(SoundboardSettings, { target: document.body, props: { server } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('SoundboardSettings', () => {
  it("lists this server's sounds with emoji, who added them, plays and length", () => {
    expect(rows()).toHaveLength(2)
    expect(rows()[0].textContent).toContain('Airhorn')
    expect(row('Airhorn').textContent).toContain('🎺')
    expect(row('Airhorn').textContent).toContain('sam')
    expect(row('Airhorn').textContent).toContain('3 plays')
    expect(row('Airhorn').textContent).toContain('2.4 s')
    expect(row('Bruh').textContent).toContain('1 play')
    expect(row('Bruh').textContent).toContain('0.6 s')
    expect(row('Bruh').textContent).toContain('Unknown')
    expect(document.body.textContent).not.toContain('Elsewhere')
  })

  it('previews a sound only here', async () => {
    btn('Preview Airhorn').click()
    await settle()
    expect(calls('soundboard_play')[0][1]).toMatchObject({ hash: 'ha', soundPercent: 120 })
    expect(calls('play_sound_to_call')).toHaveLength(0)
  })

  it('removes a sound after asking', async () => {
    btn('Remove Airhorn').click()
    flushSync()
    expect(calls('delete_sound')).toHaveLength(0)
    expect(row('Airhorn').textContent).toContain('Remove Airhorn?')
    text('Cancel').click()
    flushSync()
    btn('Remove Airhorn').click()
    flushSync()
    text('Remove').click()
    await settle()
    expect(calls('delete_sound')[0][1]).toEqual({ id: 'a' })
  })

  it('opens the add and edit dialog', async () => {
    text('Add sound').click()
    await settle()
    expect(document.querySelector('[role=dialog][aria-label="Add a sound"]')).not.toBeNull()
    text('Cancel').click()
    await settle()
    btn('Edit Airhorn').click()
    await settle()
    expect(document.querySelector('[role=dialog][aria-label="Edit sound"]')).not.toBeNull()
    expect(document.querySelector<HTMLInputElement>('[aria-label="Name"]')!.value).toBe('Airhorn')
  })

  it('says when there are none', () => {
    app.state = { ...app.state, sounds: {} }
    flushSync()
    expect(document.body.textContent).toContain('No sounds yet.')
  })
})
