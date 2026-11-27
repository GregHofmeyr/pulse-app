// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import ShowUp from './ShowUp.svelte'
import TypingSection from './TypingSection.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import { TYPING_SUGGESTIONS, TYPING_TEXT_MAX } from '../../lib/typingLine'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const jo: User = { id: 'U1', username: 'jo', display_name: 'jo', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile = (): Profile => ({
  user: jo,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
})

let cmps: ReturnType<typeof mount>[] = []
let st: ProfileEditorState
beforeEach(() => {
  app.state = { ...emptyState(), me: jo } as never
})
afterEach(() => {
  cmps.forEach((c) => unmount(c))
  cmps = []
  document.body.innerHTML = ''
})
function show(withShowUp = false) {
  st = new ProfileEditorState(profile())
  cmps.push(mount(TypingSection, { target: document.body, props: { state: st } }))
  if (withShowUp) cmps.push(mount(ShowUp, { target: document.body, props: { state: st } }))
  flushSync()
}
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!
function type(el: HTMLInputElement, v: string) {
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}

describe('typing text section', () => {
  it('input_and_suggestion_chips_set_draft', () => {
    show()
    const input = q<HTMLInputElement>('input[aria-label="Typing text"]')
    expect(q('.typing-row').textContent).toContain('jo')
    type(input, 'plotting hard')
    expect(st.draft.typingText).toBe('plotting hard')
    expect(q('.typing-preview').textContent).toContain('jo is plotting hard')
    const chips = [...document.querySelectorAll<HTMLButtonElement>('[aria-label="Suggestions"] button')]
    expect(chips.map((c) => c.textContent)).toEqual([...TYPING_SUGGESTIONS])
    chips[0].click()
    flushSync()
    expect(st.draft.typingText).toBe(TYPING_SUGGESTIONS[0])
    expect(input.value).toBe(TYPING_SUGGESTIONS[0])
    expect(chips[0].getAttribute('aria-pressed')).toBe('true')
    expect(chips[1].getAttribute('aria-pressed')).toBe('false')
  })
  it('typing_problem_shows_under_input', () => {
    show()
    const input = q<HTMLInputElement>('input[aria-label="Typing text"]')
    expect(document.querySelector('[role="alert"]')).toBeNull()
    type(input, 'x'.repeat(TYPING_TEXT_MAX + 1))
    expect(q('[role="alert"]').textContent).toMatch(/\S/)
    expect(input.getAttribute('aria-invalid')).toBe('true')
    expect(st.invalid).toBe(true)
  })
  it('show_up_typing_line_follows', () => {
    show(true)
    type(q<HTMLInputElement>('input[aria-label="Typing text"]'), 'scheming')
    expect(q('[data-typing]').textContent).toContain('is scheming')
  })
  it('blank_display_name_falls_back_to_username', () => {
    show()
    st.draft.displayName = '  '
    flushSync()
    expect(q('.typing-who').textContent).toBe('jo')
  })
  it('problem_clears_once_valid', () => {
    show()
    const input = q<HTMLInputElement>('input[aria-label="Typing text"]')
    type(input, 'x'.repeat(TYPING_TEXT_MAX + 1))
    expect(document.querySelector('[role="alert"]')).toBeTruthy()
    type(input, 'ok')
    expect(document.querySelector('[role="alert"]')).toBeNull()
    expect(input.getAttribute('aria-invalid')).toBe('false')
  })
})
