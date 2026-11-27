// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import NameStylePopover from './NameStylePopover.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { NAME_FONTS } from '../../lib/nameFonts'
import { NAME_EFFECTS } from '../../lib/nameEffects'
import { NAME_GRADIENTS, NAME_SOLIDS } from '../../lib/nameStyle'
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
  cmp = mount(NameStylePopover, { target: document.body, props: { state: st, onClose } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const click = (el: Element) => { (el as HTMLElement).click(); flushSync() }
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!

describe('the Name style popover', () => {
  it('font_tiles_show_Gg_in_each_font', () => {
    const group = q('[role=radiogroup][aria-label="Name font"]')
    expect(group.querySelectorAll('[role=radio]').length).toBe(NAME_FONTS.length + 1)
    for (const f of NAME_FONTS) {
      const tile = q(`[aria-label="Name font"] [role=radio][aria-label="${f.label}"]`)
      expect(tile.textContent?.trim()).toBe('Gg')
      expect(tile.querySelector<HTMLElement>('.name')!.style.fontFamily).toContain(f.family.split(',')[0].replace(/'/g, ''))
    }
    click(q('[aria-label="Name font"] [aria-label="Script"]'))
    expect(st.draft.font).toBe('pacifico')
    expect(q('[aria-label="Name font"] [aria-label="Script"]').getAttribute('aria-checked')).toBe('true')
  })
  it('effect_tiles_cover_none_and_every_effect', () => {
    const group = q('[role=radiogroup][aria-label="Name effect"]')
    expect(group.querySelector('[aria-label="None"]')).not.toBeNull()
    for (const e of NAME_EFFECTS) expect(group.querySelector(`[aria-label="${e.label}"] [data-fx="${e.key}"]`)).not.toBeNull()
    expect(group.querySelectorAll('[role=radio]').length).toBe(NAME_EFFECTS.length + 1)
    expect(group.querySelector('.fx-play')).toBeNull() // drawn still
    click(group.querySelector('[aria-label="Glitch"]')!)
    expect(st.draft.effect).toBe('glitch')
    click(group.querySelector('[aria-label="None"]')!)
    expect(st.draft.effect).toBeNull()
  })
  it('colour_has_solids_gradients_and_custom', () => {
    const group = q('[role=radiogroup][aria-label="Name colour"]')
    expect(group.querySelector('[aria-label="Default"]')).not.toBeNull()
    for (const p of NAME_SOLIDS) expect(group.querySelector(`[aria-label="${p.label}"]`)).not.toBeNull()
    for (const p of NAME_GRADIENTS) expect(group.querySelector(`[aria-label="${p.label} gradient"]`)).not.toBeNull()
    click(group.querySelector(`[aria-label="${NAME_SOLIDS[0].label}"]`)!)
    expect(st.draft.colors).toEqual(NAME_SOLIDS[0].colors)
    click(group.querySelector(`[aria-label="${NAME_GRADIENTS[0].label} gradient"]`)!)
    expect(st.draft.colors).toEqual(NAME_GRADIENTS[0].colors)
    click(group.querySelector('[aria-label="Custom"]')!)
    expect(q('input[type=color]')).not.toBeNull()
    const c = q<HTMLInputElement>('input[type=color]')
    c.value = '#445566'
    c.dispatchEvent(new Event('input', { bubbles: true }))
    click(q('button[aria-label="Apply"]'))
    expect(st.draft.colors[0]).toBe('#445566')
  })
  it('preview_shows_card_chat_and_list_name', () => {
    expect(q('.card')).not.toBeNull()
    expect(q('[aria-label="In chat"] .name')).not.toBeNull()
    expect(q('[aria-label="In the channel list"] .name')).not.toBeNull()
  })
  it('cancel_restores_font_colour_effect', () => {
    click(q('[aria-label="Name font"] [aria-label="Script"]'))
    click(q('[aria-label="Name colour"]').querySelector(`[aria-label="${NAME_SOLIDS[0].label}"]`)!)
    click(q('[aria-label="Name effect"] [aria-label="Neon"]'))
    click(q('button[aria-label="Cancel"]'))
    expect([st.draft.font, st.draft.colors, st.draft.effect]).toEqual([null, [], null])
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
