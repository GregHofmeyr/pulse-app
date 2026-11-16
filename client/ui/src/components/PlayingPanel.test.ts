// @vitest-environment jsdom
// "Playing" on a profile card (#380): the wide art faded behind a strip with the logo (else the name) and the elapsed
// time; a game without art or play time is the name alone.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import PlayingPanel from './PlayingPanel.svelte'
import { clock } from '../lib/store.svelte'
import type { Playing } from '../lib/protocol/Playing'

const thumb = (c: string) => ({ hash: c.repeat(64), width: 480, height: 155 })
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const show = (playing: Playing) => {
  cmp = mount(PlayingPanel, { target: document.body, props: { playing } })
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

  it('a hand-added game (or art and play time off): the name alone', () => {
    show({ name: 'Old Game', steam_app_id: null, started_at: null, art: null })
    expect(document.querySelector('img')).toBeNull()
    expect(document.querySelector('section.playing')!.classList.contains('has-art')).toBe(false)
    expect(document.querySelector('.name')!.textContent).toBe('Old Game')
  })
})
