// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import FullProfile from './FullProfile.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { closeCard, closeFull } from '../lib/profileCard.svelte'
import type { User } from '../lib/protocol/User'
import type { Profile } from '../lib/protocol/Profile'

const sam: User = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const alex: User = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile = (over: Partial<Profile> = {}): Profile => ({
  user: sam,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
  stats: { voice_seconds: 9000, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  ...over,
})

let cmp: ReturnType<typeof mount> | undefined
const calls = (cmd: string) => vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd)

function show(p: Profile | null = profile(), onClose = vi.fn()) {
  app.state = {
    ...emptyState(),
    me: alex,
    people: { U1: { user: alex, online: true, last_seen_at: null }, U2: { user: sam, online: true, last_seen_at: null } },
    servers: [{ id: 'S1', name: 'Den' }],
    profiles: p ? { U2: p } : {},
  } as never
  cmp = mount(FullProfile, { target: document.body, props: { userId: 'U2', serverId: null, onClose } })
  flushSync()
  return onClose
}
const panel = (title: string) => [...document.querySelectorAll('section.board')].find((s) => s.querySelector('h2')?.textContent === title) as HTMLElement | undefined

beforeEach(() => {
  resetState()
  closeCard()
  closeFull()
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async () => undefined) as never)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('FullProfile', () => {
  it('shows stats totals', () => {
    show()
    const s = panel('Stats')!.textContent!
    expect(s).toContain('Voice')
    expect(s).toContain('2.5 h')
    expect(s).toContain('Member since')
    expect(s).toContain('Oct 3, 2026')
    expect(document.querySelector('.card.full')).not.toBeNull()
    expect(calls('get_profile')).toHaveLength(0) // cached: no refetch
  })

  it('shows the most-used reaction and times renamed when set', () => {
    show(profile({ stats: { ...profile().stats, top_reaction: '🔥', times_renamed: 3 } }))
    const s = panel('Stats')!
    expect(s.textContent).toContain('Most-used reaction')
    expect(s.querySelector('img')!.getAttribute('alt')).toBe('🔥')
    expect(s.textContent).toContain('Times renamed')
    expect(s.textContent).toContain('3')
  })

  it('hides most-used reaction and times renamed when empty', () => {
    show()
    const s = panel('Stats')!.textContent!
    expect(s).not.toContain('Most-used reaction')
    expect(s).not.toContain('Times renamed')
  })

  it('hides the Name Archive with no rows and lists rows newest first', () => {
    show()
    expect(panel('Name Archive')).toBeUndefined()
    unmount(cmp!)
    cmp = undefined
    show(profile({ name_archive: [
      { server_id: 'S1', name: 'Newer', changed_by: 'U1', changed_at: '2026-10-20T00:00:00Z' },
      { server_id: 'S1', name: 'Older', changed_by: 'U2', changed_at: '2026-10-10T00:00:00Z' },
    ] }))
    const rows = [...panel('Name Archive')!.querySelectorAll('li')]
    expect(rows).toHaveLength(2)
    expect(rows[0].textContent).toContain('Newer')
    expect(rows[0].textContent).toContain('Den')
    expect(rows[0].textContent).toContain('by alex')
    expect(rows[0].textContent).toContain('Oct 20, 2026')
    expect(rows[1].textContent).toContain('Older')
  })

  it('trophy case shows No trophies yet, then badges', () => {
    show()
    expect(panel('Trophy case')!.textContent).toContain('No trophies yet')
    unmount(cmp!)
    cmp = undefined
    show(profile({ badges: [{ key: 'night_owl', earned_at: '2026-10-03T00:00:00Z', first_earner: false }] }))
    const t = panel('Trophy case')!
    expect(t.textContent).not.toContain('No trophies yet')
    expect(t.textContent).toContain('Night Owl')
    expect(t.textContent).toContain('Oct 3, 2026')
    expect(t.querySelector('svg')).not.toBeNull()
  })

  it('an unknown badge key gets a readable name', () => {
    show(profile({ badges: [{ key: 'early-bird_2026', earned_at: '2026-10-03T00:00:00Z', first_earner: true }] }))
    expect(panel('Trophy case')!.textContent).toContain('Early Bird 2026')
  })

  it('fetches the profile when it is not cached', async () => {
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'get_profile' ? profile() : undefined)) as never)
    show(null)
    await vi.waitFor(() => expect(calls('get_profile')).toHaveLength(1))
  })

  it('rows identical but for who renamed them are both kept', () => {
    show(profile({ name_archive: [
      { server_id: 'S1', name: 'Same', changed_by: 'U1', changed_at: '2026-10-20T00:00:00Z' },
      { server_id: 'S1', name: 'Same', changed_by: 'U2', changed_at: '2026-10-20T00:00:00Z' },
    ] }))
    expect(panel('Name Archive')!.querySelectorAll('li')).toHaveLength(2)
  })

  it('a server you are not in reads "a server"', () => {
    show(profile({ name_archive: [{ server_id: 'S9', name: 'Elsewhere', changed_by: 'U2', changed_at: '2026-10-20T00:00:00Z' }] }))
    expect(panel('Name Archive')!.querySelector('li')!.textContent).toContain('a server')
  })

  it('restyles live on a ProfileUpdated', () => {
    show()
    const styled = { ...sam, style: { font: 'fredoka', colors: ['#a3d8ff'] } }
    const card = { ...profile().card, bio: 'new bio', theme_top: '#8a3cff', theme_bottom: '#ff8a2a' }
    app.state = applyEvent(app.state, { t: 'ProfileUpdated', d: { user: styled, card } }, 0)
    flushSync()
    expect(document.querySelector('.card .name')!.getAttribute('style')).toContain('Fredoka')
    expect(document.querySelector('.bio')!.textContent).toContain('new bio')
    expect((document.querySelector('.modal') as HTMLElement).style.getPropertyValue('--glow-top')).toBe('#8a3cff')
    expect(panel('Stats')!.textContent).toContain('2.5 h') // stats kept
  })

  it('a failed load with nothing cached offers Retry', async () => {
    vi.mocked(invoke).mockImplementation((async (cmd: string) => { if (cmd === 'get_profile') throw 'offline' }) as never)
    show(null)
    await vi.waitFor(() => expect(document.querySelector('[role="alert"]')?.textContent).toContain('offline'))
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'get_profile' ? profile() : undefined)) as never)
    const retry = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Retry')!
    retry.click()
    await vi.waitFor(() => expect(panel('Stats')).toBeDefined())
    expect(calls('get_profile')).toHaveLength(2)
    expect(document.querySelector('[role="alert"]')).toBeNull()
  })

  it('the board background is the theme\'s tinted surface', async () => {
    const { readFileSync } = await import('node:fs')
    const { resolve } = await import('node:path')
    const src = readFileSync(resolve(__dirname, 'FullProfile.svelte'), 'utf8')
    const modal = /\.modal \{([^}]*)\}/.exec(src)![1]
    expect(modal).toContain('background: linear-gradient(var(--card-top), var(--card-bottom))')
    expect(modal).not.toContain('var(--bg-1)')
  })

  it('Escape closes', () => {
    const onClose = show()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(onClose).toHaveBeenCalled()
  })
})
