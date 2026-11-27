// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import CardThemePopover from './CardThemePopover.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { THEME_PRESETS } from '../../lib/cardTheme'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile: Profile = {
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [], name_archive: [], showcase: [], showcase_chosen: false,
}
let cmp: ReturnType<typeof mount> | undefined
let st: ProfileEditorState
const onClose = vi.fn()
beforeEach(() => {
  onClose.mockReset()
  app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
  st = new ProfileEditorState(profile)
  cmp = mount(CardThemePopover, { target: document.body, props: { state: st, onClose } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const click = (el: Element) => { (el as HTMLElement).click(); flushSync() }
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!

describe('the Card theme popover', () => {
  it('shows_each_preset_as_a_mini_card_and_marks_the_current_one', () => {
    const tiles = THEME_PRESETS.map((p) => q(`button[aria-label="${p.label}"]`))
    for (const t of tiles) expect(t.querySelector('.mc-banner')).not.toBeNull()
    expect(q('button[aria-label="Plain"]').getAttribute('aria-checked')).toBe('true')
    expect(q('button[aria-label="Custom"]').querySelector('.mc-banner')).toBeNull() // a plus until a custom pair is held
  })
  it('picking_a_preset_writes_the_draft_and_cancel_restores_it', () => {
    click(q('button[aria-label="Sunset"]'))
    expect([st.draft.themeTop, st.draft.themeBottom]).toEqual(['#8a3cff', '#ff8a2a'])
    click(q('button[aria-label="Cancel"]'))
    expect([st.draft.themeTop, st.draft.themeBottom]).toEqual(['#3a3d46', '#16171b'])
    expect(onClose).toHaveBeenCalledTimes(1)
  })
  it('custom_pickers_flush_on_apply', () => {
    click(q('button[aria-label="Custom"]'))
    const bottom = q<HTMLInputElement>('input[aria-label="Bottom colour"]')
    bottom.value = '#445566'
    bottom.dispatchEvent(new Event('input', { bubbles: true }))
    click(q('button[aria-label="Apply"]'))
    expect(st.draft.themeBottom).toBe('#445566')
    expect(onClose).toHaveBeenCalledTimes(1)
  })
  it('previews_you_on_your_card_in_chat_and_in_the_channel_list', () => {
    expect(q('.card')).not.toBeNull()
    expect(q('[aria-label="In chat"]')).not.toBeNull()
    expect(q('[aria-label="In the channel list"]')).not.toBeNull()
  })
})

// Ported from ProfileSettings.test.ts (the old Card tab) when the old page went (#727, task 10).
describe('the Card theme popover, as the old tab behaved', () => {
  it('a_preset_pick_restyles_the_preview_card', () => {
    expect(q('.card').getAttribute('style')).toContain('--glow-top: #3a3d46')
    click(q('button[aria-label="Berry"]'))
    expect(q('.card').getAttribute('style')).toContain('--glow-top: #c2457a')
    expect(q('.card').getAttribute('style')).toContain('--glow-bottom: #3a1f5e')
    expect(q('button[aria-label="Berry"]').getAttribute('aria-checked')).toBe('true')
  })

  it('the_custom_pickers_write_the_draft_and_show_their_hex', () => {
    click(q('button[aria-label="Custom"]'))
    const [top, bottom] = [...document.querySelectorAll<HTMLInputElement>('.pickers input[type=color]')]
    for (const [el, v] of [[top, '#ffffff'], [bottom, '#ffff00']] as const) {
      el.value = v
      el.dispatchEvent(new Event('input', { bubbles: true }))
      el.dispatchEvent(new Event('change', { bubbles: true }))
    }
    flushSync()
    expect(q('.card').getAttribute('style')).toContain('--glow-top: #ffffff')
    expect(q('.card').getAttribute('style')).toContain('--glow-bottom: #ffff00')
    expect(q('button[aria-label="Custom"]').getAttribute('aria-checked')).toBe('true')
    expect([...document.querySelectorAll('.pick')].map((l) => l.textContent!.replace(/\s+/g, ' ').trim())).toEqual(['Top #ffffff', 'Bottom #ffff00'])
    // the Custom tile is a mini card in the custom colours; a preset picked puts the + back
    expect(q('button[aria-label="Custom"]').querySelector('.mc-banner')).not.toBeNull()
    click(q('button[aria-label="Night sky"]'))
    expect(q('button[aria-label="Custom"]').querySelector('.mc-banner')).toBeNull()
  })

  it('arrows_move_along_the_tiles_with_one_tab_stop_and_wrap', () => {
    const radios = [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Card theme"] [role=radio]')]
    expect(radios.filter((r) => r.tabIndex === 0)).toEqual([q('button[aria-label="Plain"]')])
    const plain = radios.indexOf(q<HTMLButtonElement>('button[aria-label="Plain"]'))
    radios[plain].focus()
    radios[plain].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    const next = radios[(plain + 1) % radios.length]
    expect(document.activeElement).toBe(next)
    expect(next.getAttribute('aria-checked')).toBe('true')
    expect(next.tabIndex).toBe(0)
    expect(radios[plain].tabIndex).toBe(-1)
  })

  it('a_saved_custom_theme_is_what_the_popover_shows', () => {
    unmount(cmp!)
    document.body.innerHTML = ''
    st = new ProfileEditorState({ ...profile, card: { ...profile.card, theme_top: '#ffb6d9', theme_bottom: '#ffffff' } })
    cmp = mount(CardThemePopover, { target: document.body, props: { state: st, onClose } })
    flushSync()
    expect(q('button[aria-label="Custom"]').getAttribute('aria-checked')).toBe('true')
    expect(q('button[aria-label="Custom"]').querySelector('.mc-banner')).not.toBeNull()
    click(q('button[aria-label="Custom"]'))
    expect([...document.querySelectorAll<HTMLInputElement>('.pickers input[type=color]')].map((i) => i.value)).toEqual(['#ffb6d9', '#ffffff'])
    expect(q('.card').getAttribute('style')).toContain('--glow-top: #ffb6d9')
  })

  it('apply_takes_a_colour_still_waiting_for_its_frame', () => {
    click(q('button[aria-label="Custom"]'))
    const top = q<HTMLInputElement>('input[aria-label="Top colour"]')
    top.value = '#ffb6d9'
    top.dispatchEvent(new Event('input', { bubbles: true }))
    click(q('button[aria-label="Apply"]'))
    expect(st.draft.themeTop).toBe('#ffb6d9')
  })
})
