// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import LooksTiles from './LooksTiles.svelte'
import ProfileEditor from './ProfileEditor.svelte'
import { closeProfileEditor, openProfileEditor, ProfileEditorState } from '../../lib/profileEditor.svelte'
import { ringPrefs } from '../../lib/speakingRing.svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const HASH = 'a'.repeat(64)
const profile = (avatar: string | null = null): Profile => ({
  user: { ...alex, avatar_hash: avatar, avatar_still: avatar },
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
})

let cmp: ReturnType<typeof mount> | undefined
let st: ProfileEditorState
beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async () => undefined) as never)
  app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  closeProfileEditor()
  document.body.innerHTML = ''
})

const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!
const click = (el: Element) => { (el as HTMLElement).click(); flushSync() }
const tile = (label: string) => q(`button[aria-label="${label}"]`)
const items = () => [...document.querySelectorAll('[role=menuitem]')].map((b) => b.textContent?.trim())
function show(p = profile()) {
  st = new ProfileEditorState(p)
  cmp = mount(LooksTiles, { target: document.body, props: { state: st } })
  flushSync()
}

describe('the Looks tiles', () => {
  it('tiles_grouped_under_headings', () => {
    show()
    expect([...document.querySelectorAll('h3')].map((h) => h.textContent?.trim())).toEqual(['Avatar & banner', 'Name', 'Card', 'Speaking ring'])
    const groups = [...document.querySelectorAll('section')].map((s) => [...s.querySelectorAll('button[aria-label]')].map((b) => b.getAttribute('aria-label')))
    expect(groups).toEqual([['Avatar', 'Banner'], ['Name style'], ['Card theme', 'Glow'], ['Ring style', 'Ring colour']])
  })

  it('avatar_banner_tiles_open_change_remove_menu', () => {
    show(profile(HASH))
    click(tile('Avatar'))
    expect(items()).toEqual(['Change', 'Remove'])
    click(q('[role=menuitem]:last-child')) // Remove
    expect(st.draft.avatarHash).toBeNull()
    click(tile('Banner'))
    expect(items()).toEqual(['Change']) // no banner to remove
  })

  it('glow_tile_toggles_draft_glow', () => {
    show()
    const was = st.draft.glow
    expect(tile('Glow').getAttribute('aria-checked')).toBe(String(was))
    click(tile('Glow'))
    expect(st.draft.glow).toBe(!was)
    expect(tile('Glow').getAttribute('aria-checked')).toBe(String(!was))
    expect(st.dirty).toBe(true)
  })

  it('theme_tile_opens_card_theme_popover', () => {
    show()
    expect(document.querySelector("[role=dialog][aria-label=\"Card theme\"]")).toBeNull()
    expect(document.querySelector('[data-editor-layer]')).toBeNull()
    click(tile('Card theme'))
    expect(q('[role=dialog][aria-label="Card theme"]')).not.toBeNull()
    expect(q('[data-editor-layer]')).not.toBeNull()
  })
  it('name_tile_opens_name_style_popover', () => {
    show()
    click(tile('Name style'))
    expect(q('[role=dialog][aria-label="Name style"]')).not.toBeNull()
  })
})

describe('tile faces', () => {
  it('ring_tile_shows_the_draft_style_even_with_classic_for_all', () => {
    ringPrefs.classic = true
    show()
    st.draft.ringStyle = 'orbit'
    flushSync()
    const box = tile('Ring style').querySelector<HTMLElement>('.ring-box')!
    expect(box.dataset.style).toBe('orbit')
    expect(box.dataset.ring).toBe('still')
    ringPrefs.classic = false
  })
  it('glow_fits_inside_the_tile_padding', () => {
    const src = readFileSync(resolve(__dirname, 'LooksTiles.svelte'), 'utf8')
    const pad = Number(src.match(/\.tile\.mini \{ padding: (\d+)px/)![1])
    const shadow = src.match(/\.glow-demo\.on \{ box-shadow: ([^;]*);/)![1]
    const reach = shadow.split(/,\s*(?=\d)/).map((l) => { const [x, y, blur, spread] = l.trim().split(/\s+/).slice(0, 4).map((v) => parseInt(v)); return Math.max(Math.abs(x), Math.abs(y)) + blur + (spread || 0) })
    expect(Math.max(...reach)).toBeLessThanOrEqual(pad)
  })
})

describe('inside the editor', () => {
  async function open() {
    openProfileEditor('looks')
    st = new ProfileEditorState(profile())
    cmp = mount(ProfileEditor, { target: document.body, props: { state: st } })
    flushSync()
  }
  const send = (key: string) => {
    const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })
    window.dispatchEvent(e)
    flushSync()
    return e
  }

  it('escape_in_popover_cancels_only_the_popover', async () => {
    await open()
    click(tile('Card theme'))
    const before = st.draft.themeTop
    click(q('button[aria-label="Berry"]'))
    expect(st.draft.themeTop).not.toBe(before)
    send('Escape')
    expect(document.querySelector('[data-editor-layer]')).toBeNull()
    expect(document.querySelector('[role=dialog][aria-label="Edit profile"]')).not.toBeNull()
    expect(document.querySelector('[role=alert]')).toBeNull() // no close prompt
    expect(st.draft.themeTop).toBe(before) // cancelled
  })

  it('editor_is_inert_while_popover_is_open', async () => {
    await open()
    click(tile('Card theme'))
    expect(q('header').hasAttribute('inert')).toBe(true)
    expect(q('.body').hasAttribute('inert')).toBe(true)
    click(q('button[aria-label="Cancel"]'))
    expect(q('header').hasAttribute('inert')).toBe(false)
  })

  it('cancel_restores_draft_exactly', async () => {
    await open()
    const before = { top: st.draft.themeTop, bottom: st.draft.themeBottom }
    click(tile('Card theme'))
    click(q('button[aria-label="Custom"]'))
    const top = q<HTMLInputElement>('input[aria-label="Top colour"]')
    top.value = '#123456'
    top.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    click(q('button[aria-label="Cancel"]'))
    await new Promise((r) => requestAnimationFrame(() => r(null))) // the perFrame pickers would have flushed by now
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    flushSync()
    expect({ top: st.draft.themeTop, bottom: st.draft.themeBottom }).toEqual(before)
    expect(st.dirty).toBe(false)
  })

  it('apply_keeps_changes_in_draft_unsaved', async () => {
    await open()
    click(tile('Card theme'))
    click(q('button[aria-label="Berry"]'))
    click(q('button[aria-label="Apply"]'))
    expect(document.querySelector('[data-editor-layer]')).toBeNull()
    expect(st.draft.themeTop).toBe('#c2457a')
    expect(st.dirty).toBe(true)
    const save = [...document.querySelectorAll<HTMLButtonElement>('footer button')].find((b) => b.textContent?.trim() === 'Save')!
    expect(save.disabled).toBe(false)
    expect(vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'update_profile')).toEqual([])
  })
})
