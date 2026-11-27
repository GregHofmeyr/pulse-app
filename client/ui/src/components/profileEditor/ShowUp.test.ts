// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import ShowUp from './ShowUp.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile = (): Profile => ({
  user: { ...alex, username: 'jo', display_name: 'jo' },
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
  const sam: User = { ...alex, id: 'U2', username: 'sam', display_name: 'sam' }
  app.state = { ...emptyState(), me: { ...alex, username: 'jo', display_name: 'jo' }, people: { U2: { user: sam, online: true, last_seen_at: null } } } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
function show() {
  st = new ProfileEditorState(profile())
  cmp = mount(ShowUp, { target: document.body, props: { state: st } })
  flushSync()
}
const q = (sel: string) => document.querySelector<HTMLElement>(sel)!

describe('where you show up', () => {
  it('voice_list_shows_you_lit_with_your_ring', () => {
    show()
    const list = q('[aria-label="Voice channel"]')
    expect(list.textContent).toContain('VOICE')
    expect(list.textContent).toContain('Lounge')
    expect(list.querySelector('svg')).toBeTruthy() // the speaker icon
    const you = list.querySelector('[data-you]')!
    expect(you.textContent).toContain('jo')
    const ring = you.querySelector('.ring-box')!
    expect(ring.getAttribute('data-place')).toBe('list')
    expect(ring.getAttribute('data-ring')).not.toBe('off')
    expect(list.textContent).toContain('alex')
  })
  it('chat_shows_alex_then_you', () => {
    show()
    const lines = [...document.querySelectorAll('[aria-label="Chat"] [data-line]')]
    expect(lines.map((l) => l.textContent)).toEqual([expect.stringMatching(/alex.*new banner\?\?/), expect.stringMatching(/jo.*Looking good!/)])
  })
  it('typing_line_uses_draft_typing_text', () => {
    show()
    st.draft.typingText = 'plotting'
    flushSync()
    const t = q('[data-typing]')
    expect(t.textContent).toContain('jo')
    expect(t.textContent).toMatch(/is plotting/)
    expect(t.querySelectorAll('.dots i').length).toBe(3)
    st.draft.typingText = ''
    flushSync()
    expect(q('[data-typing]').textContent).toMatch(/is typing…/)
  })
  it('uses_draft_name_style_and_avatar', () => {
    show()
    st.draft.displayName = 'Joanna'
    st.draft.avatarHash = 'abc123'
    st.draft.avatarStill = 'abc123'
    st.draft.font = 'fredoka' as never
    flushSync()
    expect(q('[data-you]').textContent).toContain('Joanna')
    expect(q('[data-line="you"]').textContent).toContain('Joanna')
    expect(q('[data-typing]').textContent).toContain('Joanna')
    expect(document.querySelector('[data-you] img')?.getAttribute('src') ?? '').toContain('abc123')
    expect(q('[data-line="you"] .name').getAttribute('style') ?? '').toMatch(/font/)
    expect(q('[data-typing] .name').classList.contains('flow')).toBe(true)
  })
})
