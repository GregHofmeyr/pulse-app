// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import ProfileCard from './ProfileCard.svelte'
import ProfileCardPopover from './ProfileCardPopover.svelte'
import { app, noteProfileUpdate, PROFILE_STALE_MS, resetState, ui } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { closeCard, closeFull, openCard, profileUi } from '../lib/profileCard.svelte'
import { mixHex, NEAR_BLACK, THEME_PRESETS } from '../lib/cardTheme'
import { appearance } from '../lib/appearance.svelte'
import type { User } from '../lib/protocol/User'
import type { CardProfile } from '../lib/protocol/CardProfile'
import type { ProfileStats } from '../lib/protocol/ProfileStats'

const BANNER = 'c'.repeat(64)
const sam: User = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const alex: User = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const card = (over: Partial<CardProfile> = {}): CardProfile => ({ banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null, ...over })
const stats: ProfileStats = { voice_seconds: 9000, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 }

let cmp: ReturnType<typeof mount> | undefined
const calls = (cmd: string) => vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd)

beforeEach(() => {
  resetState()
  app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null }, U2: { user: sam, online: true, last_seen_at: null } } } as never
  appearance.showNameStyles = true
  closeCard()
  closeFull()
  ui.goTo = null
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async () => undefined) as never)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function show(props: Record<string, unknown> = {}) {
  cmp = mount(ProfileCard, { target: document.body, props: { user: sam, card: card(), badges: [], stats, serverId: null, ...props } })
  flushSync()
  return document.querySelector('.card') as HTMLElement
}

describe('ProfileCard', () => {
  it('renders each preset theme', () => {
    for (const p of THEME_PRESETS) {
      const el = show({ card: card({ theme_top: p.top, theme_bottom: p.bottom }) })
      expect(el.style.getPropertyValue('--card-top')).toBe(mixHex(p.top, NEAR_BLACK, 0.3))
      expect(el.style.getPropertyValue('--glow-bottom')).toBe(p.bottom)
      unmount(cmp!)
      cmp = undefined
      document.body.innerHTML = ''
    }
  })

  it('banner, avatar, name, username · pronouns, bio', () => {
    const el = show({ card: card({ banner_hash: BANNER, pronouns: 'they/them', bio: 'hi **there**' }) })
    expect(el.querySelector('.banner img')!.getAttribute('src')).toContain(BANNER)
    expect(el.querySelector('.av')).not.toBeNull()
    expect(el.querySelector('.av')!.getAttribute('style')).toContain('80px')
    expect(el.querySelector('.name')!.textContent).toBe('sam')
    expect(el.querySelector('.handle')!.textContent).toBe('sam · they/them')
    expect(el.querySelector('.bio')!.innerHTML).toContain('<strong>there</strong>')
  })

  it('no banner still renders', () => {
    const el = show()
    expect(el.querySelector('.banner img')).toBeNull()
    expect(el.querySelector('.name')!.textContent).toBe('sam')
    expect(el.querySelector('.handle')!.textContent).toBe('sam')
    expect(el.querySelector('.bio')).toBeNull()
  })

  it('badges row hidden when there are none', () => {
    expect(show().querySelector('.badges')).toBeNull()
    unmount(cmp!)
    cmp = undefined
    const el = show({ badges: [{ key: 'night_owl', earned_at: '2026-10-03T00:00:00Z', first_earner: false }] })
    expect(el.querySelector('.badges [aria-label="Night Owl"]')).not.toBeNull()
  })

  it('shows Member since and Voice from the stats', () => {
    const el = show()
    expect(el.querySelector('.info')!.textContent).toContain('Member since')
    expect(el.querySelector('.info')!.textContent).toContain('Oct 3, 2026')
    expect(el.querySelector('.info')!.textContent).toContain('2.5 h')
  })

  it("shows In <channel> when they're in voice", () => {
    app.state = {
      ...app.state,
      channels: { V1: { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 } },
      voice: { V1: [{ user_id: 'U2', flags: { muted: false, deafened: false } }], V9: [{ user_id: 'U1', flags: { muted: false, deafened: false } }] },
    } as never
    expect(show().querySelector('.info')!.textContent).toContain('In Lounge')
    unmount(cmp!)
    cmp = undefined
    // a room you can't see (not in your channels) isn't named
    app.state = { ...app.state, channels: {}, } as never
    expect(show().querySelector('.room')).toBeNull()
    unmount(cmp!)
    cmp = undefined
    // nor is a group call (no server), even one you're in
    app.state = {
      ...app.state,
      channels: { G1: { id: 'G1', server_id: null, kind: 'group', name: 'Pals', position: 0 } },
      voice: { G1: [{ user_id: 'U2', flags: { muted: false, deafened: false } }] },
    } as never
    expect(show().querySelector('.room')).toBeNull()
  })

  it('the name keeps their own colour (never a role colour) and their nickname', () => {
    const styled = { ...sam, style: { font: null, colors: ['#ffc4a3'] } }
    app.state = {
      ...app.state,
      people: { ...app.state.people, U2: { user: styled, online: true, last_seen_at: null } },
      members: { S1: [{ user: styled, nickname: 'Sammy' }] },
      roleColors: { S1: { U2: '#a3d8ff' } },
    } as never
    const name = show({ user: styled, serverId: 'S1' }).querySelector('.name')!
    expect(name.textContent).toBe('Sammy')
    expect(name.getAttribute('style')).toContain('rgb(255, 196, 163)')
    expect(name.getAttribute('style')).not.toContain('rgb(163, 216, 255)')
  })

  it('the message box sends to the DM and switches to it', async () => {
    const onClose = vi.fn()
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'create_dm' ? { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 } : undefined)) as never)
    const el = show({ onClose })
    const box = el.querySelector('input.msg') as HTMLInputElement
    expect(box.placeholder).toBe('Message @sam')
    box.value = 'hey'
    box.dispatchEvent(new Event('input'))
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await vi.waitFor(() => expect(calls('send_message')).toHaveLength(1))
    expect(calls('create_dm')[0][1]).toEqual({ userIds: ['U2'] })
    expect(calls('send_message')[0][1]).toMatchObject({ channelId: 'D1', content: 'hey', replyToId: null })
    expect(app.state.pending.D1).toHaveLength(1)
    expect(ui.goTo).toBe('D1')
    expect(onClose).toHaveBeenCalled()
  })

  it('a failed send shows the error and keeps the draft', async () => {
    const onClose = vi.fn()
    vi.mocked(invoke).mockImplementation((async (cmd: string) => {
      if (cmd === 'create_dm') throw "You can't message sam."
    }) as never)
    const el = show({ onClose })
    const box = el.querySelector('input.msg') as HTMLInputElement
    box.value = 'hey'
    box.dispatchEvent(new Event('input'))
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await vi.waitFor(() => expect(el.querySelector('[role="alert"]')).not.toBeNull())
    expect(el.querySelector('[role="alert"]')!.textContent).toContain("You can't message sam.")
    expect(box.value).toBe('hey')
    expect(calls('send_message')).toHaveLength(0)
    expect(ui.goTo).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
  })

  it('a malicious bio renders inert', () => {
    const el = show({ card: card({ bio: '<img src=x onerror=alert(1)> [click](javascript:alert(1))' }) })
    const bio = el.querySelector('.bio')!
    expect(bio.textContent).toContain('click')
    expect(bio.querySelector('[onerror]')).toBeNull()
    expect([...bio.querySelectorAll('a')].some((a) => /^\s*javascript:/i.test(a.getAttribute('href') ?? ''))).toBe(false)
  })

  it('no message box on your own card or in preview', () => {
    expect(show({ user: alex }).querySelector('input.msg')).toBeNull()
    unmount(cmp!)
    cmp = undefined
    const el = show({ preview: true })
    expect(el.querySelector('input.msg')).toBeNull()
    expect([...el.querySelectorAll('button')].some((b) => b.textContent === 'View full profile')).toBe(false)
  })

  it('View full profile calls openFull', () => {
    const el = show({ serverId: 'S1' })
    const b = [...el.querySelectorAll('button')].find((x) => x.textContent === 'View full profile')!
    b.click()
    expect(profileUi.full).toEqual({ userId: 'U2', serverId: 'S1' })
  })
})

describe('ProfileCardPopover', () => {
  const profile = (u: User, c: CardProfile) => ({ user: u, card: c, stats, badges: [], name_archive: [] })

  it('an open card follows a ProfileUpdated', async () => {
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'get_profile' ? profile(sam, card({ bio: 'old' })) : undefined)) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    const anchor = document.createElement('button')
    document.body.append(anchor)
    openCard('U2', null, anchor)
    flushSync()
    await vi.waitFor(() => expect(document.querySelector('.bio')?.textContent).toContain('old'))
    const nameBefore = document.querySelector('.card .name')!.getAttribute('style') ?? ''
    const styled = { ...sam, style: { font: 'fredoka', colors: ['#a3d8ff'] } }
    const e = { t: 'ProfileUpdated', d: { user: styled, card: card({ bio: 'new', theme_top: '#8a3cff', theme_bottom: '#ff8a2a' }) } } as const
    noteProfileUpdate(e)
    app.state = applyEvent(app.state, e, 0)
    flushSync()
    await tick()
    expect(document.querySelector('.card .name')!.getAttribute('style')).not.toBe(nameBefore)
    expect(document.querySelector('.card .name')!.getAttribute('style')).toContain('Fredoka')
    expect(document.querySelector('.bio')!.textContent).toContain('new')
    expect((document.querySelector('.card') as HTMLElement).style.getPropertyValue('--glow-top')).toBe('#8a3cff')
    expect(calls('get_profile')).toHaveLength(1)
  })

  it('shows the user with a skeleton bio while loading', () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    openCard('U2', null, document.body)
    flushSync()
    expect(document.querySelector('.card .name')!.textContent).toBe('sam')
    expect(document.querySelector('.skeleton')).not.toBeNull()
  })

  it('Escape closes and returns focus', async () => {
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'get_profile' ? profile(sam, card()) : undefined)) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    const anchor = document.createElement('button')
    document.body.append(anchor)
    anchor.focus()
    openCard('U2', null, anchor)
    flushSync()
    expect(document.querySelector('[role="dialog"]')).not.toBeNull()
    expect(document.activeElement).not.toBe(anchor)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(profileUi.card).toBeNull()
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    expect(document.activeElement).toBe(anchor)
  })

  it('stays inside the window, measured, even when opened low down', () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never)
    const h = vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(507)
    try {
      cmp = mount(ProfileCardPopover, { target: document.body })
      const anchor = document.createElement('button')
      document.body.append(anchor)
      anchor.getBoundingClientRect = () => ({ left: 20, right: 60, top: window.innerHeight - 30, bottom: window.innerHeight - 10, width: 40, height: 20, x: 20, y: window.innerHeight - 30, toJSON() {} })
      openCard('U2', null, anchor)
      flushSync()
      const top = parseInt((document.querySelector('.pop') as HTMLElement).style.top)
      expect(top + 507).toBeLessThanOrEqual(window.innerHeight - 8)
      expect(top).toBeGreaterThanOrEqual(8)
    } finally {
      h.mockRestore()
    }
  })

  it('clicking the same name again closes it', () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    const anchor = document.createElement('button')
    document.body.append(anchor)
    anchor.addEventListener('click', () => openCard('U2', null, anchor))
    anchor.focus()
    anchor.click()
    flushSync()
    expect(profileUi.card).not.toBeNull()
    // a real click: pointerdown, then click
    anchor.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    flushSync()
    anchor.click()
    flushSync()
    expect(profileUi.card).toBeNull()
    expect(document.querySelector('[role="dialog"]')).toBeNull()
  })

  it('a press on the anchor that never took focus is left to its click too', () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    const anchor = document.createElement('span')
    document.body.append(anchor)
    anchor.getBoundingClientRect = () => ({ left: 10, right: 50, top: 10, bottom: 30, width: 40, height: 20, x: 10, y: 10, toJSON() {} })
    openCard('U2', null, anchor)
    flushSync()
    anchor.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, clientX: 20, clientY: 20 }))
    flushSync()
    expect(profileUi.card).not.toBeNull()
    openCard('U2', null, anchor)
    expect(profileUi.card).toBeNull()
  })

  it('a press inside what had focus (not the opener) closes it', () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    const box = document.createElement('textarea')
    const anchor = document.createElement('button')
    document.body.append(box, anchor)
    box.focus()
    openCard('U2', null, anchor)
    flushSync()
    box.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    flushSync()
    expect(profileUi.card).toBeNull()
  })

  it('toggling compares the opener, not its rectangle', () => {
    const rect = () => ({ left: 10, right: 50, top: 10, bottom: 30, width: 40, height: 20, x: 10, y: 10, toJSON() {} })
    const a = document.createElement('button')
    const b = document.createElement('button')
    a.getBoundingClientRect = rect
    b.getBoundingClientRect = rect
    openCard('U2', null, a)
    openCard('U2', null, b) // another element at the same spot: moves the card, doesn't close it
    expect(profileUi.card?.opener).toBe(b)
    let top = 10
    b.getBoundingClientRect = () => ({ ...rect(), top: (top += 100), y: top })
    openCard('U2', null, b) // the same element, scrolled since: closes
    expect(profileUi.card).toBeNull()
  })

  it('a cached card shows at once and refreshes in the background once over 5 minutes old', async () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(5_000_000)
    try {
      vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'get_profile' ? profile(sam, card({ bio: 'fresh' })) : undefined)) as never)
      cmp = mount(ProfileCardPopover, { target: document.body })
      const anchor = document.createElement('button')
      document.body.append(anchor)
      openCard('U2', null, anchor)
      flushSync()
      await vi.waitFor(() => expect(document.querySelector('.bio')?.textContent).toContain('fresh'))
      expect(calls('get_profile')).toHaveLength(1)
      closeCard()
      flushSync()
      // reopened within 5 minutes: no request
      now.mockReturnValue(5_000_000 + PROFILE_STALE_MS - 1)
      openCard('U2', null, anchor)
      flushSync()
      await tick()
      expect(calls('get_profile')).toHaveLength(1)
      closeCard()
      flushSync()
      // reopened later: the cached card at once, then one background request
      vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'get_profile' ? profile(sam, card({ bio: 'newer' })) : undefined)) as never)
      now.mockReturnValue(5_000_000 + 2 * PROFILE_STALE_MS)
      openCard('U2', null, anchor)
      flushSync()
      expect(document.querySelector('.bio')!.textContent).toContain('fresh')
      expect(document.querySelector('.skeleton')).toBeNull()
      await vi.waitFor(() => expect(document.querySelector('.bio')?.textContent).toContain('newer'))
      await tick()
      expect(calls('get_profile')).toHaveLength(2)
    } finally {
      now.mockRestore()
    }
  })

  it('a window resize closes it', () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    openCard('U2', null, document.body)
    flushSync()
    window.dispatchEvent(new Event('resize'))
    flushSync()
    expect(profileUi.card).toBeNull()
  })

  it('an outside click closes it', () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never)
    cmp = mount(ProfileCardPopover, { target: document.body })
    const outside = document.createElement('div')
    const anchor = document.createElement('button')
    document.body.append(outside, anchor)
    openCard('U2', null, anchor)
    flushSync()
    outside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    flushSync()
    expect(profileUi.card).toBeNull()
  })
})
