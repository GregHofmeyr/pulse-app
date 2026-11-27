// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import ProfileEditor from './ProfileEditor.svelte'
import { closeProfileEditor, openProfileEditor, profileEditor, ProfileEditorState } from '../../lib/profileEditor.svelte'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const saved = (): Profile => ({
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: 'they/them', bio: 'hello' },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
})

let cmp: ReturnType<typeof mount> | undefined
let loadFails = false
beforeEach(() => {
  localStorage.clear()
  loadFails = false
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async (cmd: string) => {
    if (cmd === 'get_profile') {
      if (loadFails) throw 'offline'
      return saved()
    }
    return undefined
  }) as never)
  app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
  openProfileEditor()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  closeProfileEditor()
  document.body.innerHTML = ''
})

let held: ProfileEditorState | undefined
async function open(section: 'looks' | 'appearance' = 'looks', withState = false) {
  openProfileEditor(section)
  held = withState ? new ProfileEditorState(saved()) : undefined
  cmp = mount(ProfileEditor, { target: document.body, props: { state: held } })
  if (section === 'looks' && !loadFails) {
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.card')) throw new Error('not loaded')
    })
  }
  flushSync()
}
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)
describe('the profile editor', () => {
  it('menu_lists_sections_in_order', async () => {
    await open()
    const nav = document.querySelector('nav[aria-label="Profile editor sections"]')!
    expect([...nav.children].map((c) => (c.getAttribute('role') === 'separator' ? '---' : c.textContent?.trim()))).toEqual(['Looks', 'Badges', 'Card stats', 'Typing text', '---', 'App appearance'])
    expect(nav.querySelectorAll('button svg').length).toBe(5)
    expect(nav.querySelector('[aria-current=page]')?.textContent?.trim()).toBe('Looks')
  })

  it('picking_an_entry_changes_the_section', async () => {
    await open()
    button('Badges')!.click()
    flushSync()
    expect(profileEditor.section).toBe('badges')
    expect(document.querySelector('nav [aria-current=page]')?.textContent?.trim()).toBe('Badges')
  })

  it('badges_section_shows_the_card_and_the_badge_panel', async () => {
    await open()
    expect(document.querySelector('aside[aria-label="Badges"]')).toBeNull()
    button('Badges')!.click()
    flushSync()
    expect(document.querySelector('.card')).not.toBeNull()
    expect(document.querySelector('aside[aria-label="Badges"]')).not.toBeNull()
    expect(document.querySelector('aside[aria-label="Looks"]')).toBeNull()
  })

  it('close_when_clean_closes_without_asking', async () => {
    await open()
    document.querySelector<HTMLButtonElement>('[aria-label="Close editor"]')!.click()
    flushSync()
    expect(profileEditor.open).toBe(false)
    expect(document.querySelector('[role=alert]')).toBeNull()
  })

  it('close_with_unsaved_changes_asks_once', async () => {
    await open('looks', true)
    dirty()
    document.querySelector<HTMLButtonElement>('[aria-label="Close editor"]')!.click()
    flushSync()
    expect(document.querySelector('[role=alert]')?.textContent).toContain('Discard your profile changes?')
    expect(profileEditor.open).toBe(true)
    // Keep editing: stays open, and the prompt is gone
    button('Keep editing')!.click()
    flushSync()
    expect(profileEditor.open).toBe(true)
    expect(document.querySelector('[role=alert]')).toBeNull()
    // Escape asks again; Escape again means Keep editing, still open
    document.querySelector<HTMLButtonElement>('[aria-label="Close editor"]')!.click()
    flushSync()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(document.querySelector('[role=alert]')).toBeNull()
    expect(profileEditor.open).toBe(true)
    // Discard: closes and resets
    document.querySelector<HTMLButtonElement>('[aria-label="Close editor"]')!.click()
    flushSync()
    button('Discard')!.click()
    flushSync()
    expect(profileEditor.open).toBe(false)
  })

  it('escape_and_outside_click_use_the_same_guard_and_a_nested_layer_can_stop_them', async () => {
    await open('looks', true)
    dirty()
    // a popover handled Escape itself: the editor leaves it alone
    const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    e.preventDefault()
    window.dispatchEvent(e)
    flushSync()
    expect(document.querySelector('[role=alert]')).toBeNull()
    // an open popover keeps the outside click for itself
    const layer = document.body.appendChild(document.createElement('div'))
    layer.setAttribute('data-editor-layer', '')
    document.querySelector<HTMLElement>('[data-backdrop]')!.click()
    flushSync()
    expect(document.querySelector('[role=alert]')).toBeNull()
    layer.remove()
    // otherwise the outside click asks, like ×
    document.querySelector<HTMLElement>('[data-backdrop]')!.click()
    flushSync()
    expect(document.querySelector('[role=alert]')?.textContent).toContain('Discard')
  })

  it('appearance_works_when_profile_fails_to_load', async () => {
    loadFails = true
    await open('looks')
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.error')) throw new Error('no error yet')
    })
    expect(document.querySelector('[role=alert]')?.textContent).toContain('offline')
    expect(button('Retry')).toBeTruthy()
    button('App appearance')!.click()
    flushSync()
    // AppearanceSettings, unchanged
    expect(document.querySelector('.stage h2')?.textContent).toBe('Appearance')
    expect(document.querySelector('.stage [role=alert]')).toBeNull()
  })

  it('appearance_hides_reset_and_save', async () => {
    await open()
    expect(button('Save')).toBeTruthy()
    expect(button('Reset')).toBeTruthy()
    button('App appearance')!.click()
    flushSync()
    expect(button('Save')).toBeUndefined()
    expect(button('Reset')).toBeUndefined()
  })

  it('save_is_enabled_only_when_dirty_and_reset_puts_it_back', async () => {
    await open('looks', true)
    expect(button('Save')!.disabled).toBe(true)
    dirty()
    expect(button('Save')!.disabled).toBe(false)
    expect(document.body.textContent).toContain('Unsaved changes')
    button('Reset')!.click()
    flushSync()
    expect(button('Save')!.disabled).toBe(true)
  })
})

// Ported from ProfileSettings.test.ts / Settings.test.ts when the old page went (#727, task 10).
describe('closing around a Save, an upload and a failed load', () => {
  const closeBtn = () => document.querySelector<HTMLButtonElement>('[aria-label="Close editor"]')!
  const prompt = () => document.querySelector('[role=alert]')?.textContent ?? ''
  const answers: ((p: Profile) => void)[] = []
  const settle = () => vi.waitFor(() => { flushSync() })
  beforeEach(() => {
    answers.length = 0
  })

  it('closing_during_a_save_does_nothing_and_no_prompt_shows', async () => {
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'update_profile' ? new Promise<Profile>((r) => answers.push(r)) : undefined)) as never)
    await open('looks', true)
    dirty()
    button('Save')!.click()
    flushSync()
    closeBtn().click()
    await settle()
    // the Save may yet fail and keep the draft: not closed, nothing to discard yet
    expect(profileEditor.open).toBe(true)
    expect(prompt()).not.toContain('Discard')
    const stored = { ...saved(), card: { ...saved().card, bio: 'edited' } }
    answers[0](stored)
    await vi.waitFor(() => { flushSync(); expect(document.body.textContent).toContain('Saved') })
    expect(app.state.profiles.U1).toEqual(stored)
    // the draft is saved: closing now asks nothing
    closeBtn().click()
    flushSync()
    expect(profileEditor.open).toBe(false)
  })

  it('closing_after_a_save_that_fails_asks_with_the_error_shown', async () => {
    vi.mocked(invoke).mockImplementation((async (cmd: string) => { if (cmd === 'update_profile') throw 'Colours must look like #a1b2c3.' }) as never)
    await open('looks', true)
    dirty()
    button('Save')!.click()
    await vi.waitFor(() => { flushSync(); expect(document.querySelector('footer [role=alert]')?.textContent).toBe('Colours must look like #a1b2c3.') })
    closeBtn().click()
    flushSync()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    expect(profileEditor.open).toBe(true)
  })

  it('a_save_started_while_the_discard_prompt_shows_puts_the_prompt_away', async () => {
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'update_profile' ? new Promise<Profile>((r) => answers.push(r)) : undefined)) as never)
    await open('looks', true)
    dirty()
    closeBtn().click()
    flushSync()
    expect(prompt()).toContain('Discard your profile changes?')
    button('Save')!.click()
    flushSync()
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    answers[0]({ ...saved(), card: { ...saved().card, bio: 'edited' } })
    await vi.waitFor(() => { flushSync(); expect(document.body.textContent).toContain('Saved') })
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(profileEditor.open).toBe(true)
  })

  it('closing_during_an_upload_asks_first', async () => {
    await open('looks', true)
    held!.uploading.avatar = true
    flushSync()
    expect(button('Save')!.disabled).toBe(true)
    closeBtn().click()
    flushSync()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    expect(profileEditor.open).toBe(true)
  })

  it('an_edit_on_one_section_is_kept_on_another_and_nothing_is_saved_until_Save', async () => {
    await open('looks', true)
    dirty()
    button('Badges')!.click()
    flushSync()
    button('Looks')!.click()
    flushSync()
    expect(held!.draft.bio).toBe('edited')
    expect(document.body.textContent).toContain('Unsaved changes')
    expect(vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'update_profile')).toHaveLength(0)
  })

  it('loads_once_you_are_known_and_a_failed_load_offers_Retry', async () => {
    loadFails = true
    app.state = emptyState()
    openProfileEditor('looks')
    cmp = mount(ProfileEditor, { target: document.body, props: {} })
    flushSync()
    // waiting: nothing is asked for before you are known
    expect(vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'get_profile')).toHaveLength(0)
    app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
    await vi.waitFor(() => {
      flushSync()
      expect(document.querySelector('.stage [role=alert]')?.textContent).toContain('offline')
    })
    loadFails = false
    button('Retry')!.click()
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.card')) throw new Error('not loaded')
    })
    expect(vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'get_profile')).toHaveLength(2)
  })
})

/** Makes the open editor's draft dirty through its shared state (what the section controls do from Task 3 on). */
function dirty() {
  held!.draft.bio = 'edited'
  flushSync()
}
