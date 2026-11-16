// @vitest-environment jsdom
// "Playing" on a profile card (#380): the wide art faded behind a strip with the logo (else the name) and the elapsed
// time; a game without art or play time is the name alone.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import PlayingPanel from './PlayingPanel.svelte'
import { app, clock, resetState, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { Playing } from '../lib/protocol/Playing'

const thumb = (c: string) => ({ hash: c.repeat(64), width: 480, height: 155 })
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const show = (playing: Playing, userId: string | null = null) => {
  cmp = mount(PlayingPanel, { target: document.body, props: { playing, userId } })
  flushSync()
}

describe('PlayingPanel', () => {
  it('a Steam game: hero behind, logo on the strip, and how long', () => {
    clock.nowIso = '2026-11-16T19:05:00.000Z'
    show({ name: 'Space Game', steam_app_id: 570, started_at: '2026-11-16T18:00:00.000Z', art: { hero: thumb('a'), header: thumb('b'), logo: thumb('c') } })
    const section = document.querySelector('section.playing')!
    expect(section.getAttribute('aria-label')).toBe('Playing Space Game')
    expect(section.classList.contains('has-art')).toBe(true)
    expect(document.querySelector('img.wide')!.getAttribute('src')).toContain('a'.repeat(64))
    const logo = document.querySelector('img.logo')!
    expect([logo.getAttribute('src')?.includes('c'.repeat(64)), logo.getAttribute('alt')]).toEqual([true, 'Space Game'])
    expect(document.querySelector('.name')).toBeNull()
    expect(document.querySelector('.time')!.textContent).toBe('for 1h 05m')
  })

  it('no logo: the name on the strip, over the header', () => {
    show({ name: 'Hat Game', steam_app_id: 440, started_at: null, art: { hero: null, header: thumb('b'), logo: null } })
    expect(document.querySelector('img.wide')!.getAttribute('src')).toContain('b'.repeat(64))
    expect(document.querySelector('.name')!.textContent).toBe('Hat Game')
    expect(document.querySelector('.time')).toBeNull()
  })

  describe('Join (#381)', () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const join = { kind: 'server' as const, app_id: 440, address: '203.0.113.7:27015' }
    const game = (j?: Playing['join']): Playing => ({ name: 'Hat Game', steam_app_id: 440, started_at: null, art: null, join: j })
    const joinButton = () => document.querySelector<HTMLButtonElement>('button.join')
    const start = () => {
      resetState()
      app.state = { ...emptyState(), me } as never
      invoke.mockReset()
      invoke.mockResolvedValue(undefined)
    }

    it("a friend's public server: Join opens it through the core", async () => {
      start()
      show(game(join), 'U2')
      expect(joinButton()!.textContent).toBe('Join')
      joinButton()!.click()
      await Promise.resolve()
      expect(invoke).toHaveBeenCalledWith('join_steam_game', { join })
    })

    it('none on your own card, without one, or for one that does not check out', () => {
      start()
      show(game(join), 'U1')
      expect(joinButton()).toBeNull()
      unmount(cmp!)
      show(game(), 'U2')
      expect(joinButton()).toBeNull()
      unmount(cmp!)
      show(game({ ...join, address: '0.0.0.0:27015' }), 'U2')
      expect(joinButton()).toBeNull()
      unmount(cmp!)
      cmp = undefined
      show(game(join))
      expect(joinButton()).toBeNull()
    })

    it("Steam not opening says so", async () => {
      start()
      invoke.mockRejectedValue('Steam didn\'t open. Is it installed?')
      show(game(join), 'U2')
      joinButton()!.click()
      for (let i = 0; i < 3; i++) await Promise.resolve()
      expect(ui.notice).toContain("Steam didn't open")
    })
  })

  it('a hand-added game (or art and play time off): the name alone', () => {
    show({ name: 'Old Game', steam_app_id: null, started_at: null, art: null })
    expect(document.querySelector('img')).toBeNull()
    expect(document.querySelector('section.playing')!.classList.contains('has-art')).toBe(false)
    expect(document.querySelector('.name')!.textContent).toBe('Old Game')
  })
})
