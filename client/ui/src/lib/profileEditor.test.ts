// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import { closeProfileEditor, openProfileEditor, profileEditor, ProfileEditorState } from './profileEditor.svelte'
import { app } from './store.svelte'
import { emptyState } from './state'
import { draftFrom, toRequest } from './profileDraft'
import type { Profile } from './protocol/Profile'
import type { User } from './protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile = (over: Partial<Profile> = {}): Profile => ({
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: 'they/them', bio: 'hello' },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
  ...over,
})

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  closeProfileEditor()
  profileEditor.section = 'looks'
  app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
})

describe('the editor state', () => {
  it('open_defaults_to_looks', () => {
    profileEditor.section = 'typing'
    openProfileEditor()
    expect(profileEditor.open).toBe(true)
    expect(profileEditor.section).toBe('looks')
    openProfileEditor('appearance')
    expect(profileEditor.section).toBe('appearance')
    closeProfileEditor()
    expect(profileEditor.open).toBe(false)
  })

  it('state_dirty_after_edit_and_clean_after_reset', () => {
    const st = new ProfileEditorState(profile())
    expect(st.dirty).toBe(false)
    st.draft.bio = 'changed'
    expect(st.dirty).toBe(true)
    st.reset()
    expect(st.dirty).toBe(false)
    expect(st.draft.bio).toBe('hello')
    // the card's badges count too
    st.showcase = []
    expect(st.dirty).toBe(true)
    st.reset()
    expect(st.dirty).toBe(false)
  })

  it('save_sends_toRequest_of_the_draft', async () => {
    const p = profile()
    const answered = profile({ card: { ...p.card, pronouns: 'she/her' } })
    vi.mocked(invoke).mockImplementation((async (cmd: string) => (cmd === 'update_profile' ? answered : undefined)) as never)
    const st = new ProfileEditorState(p)
    st.draft.pronouns = 'she/her'
    const sent = toRequest(JSON.parse(JSON.stringify(st.draft)))
    await st.save()
    const call = vi.mocked(invoke).mock.calls.find((c) => c[0] === 'update_profile')!
    expect(call[1]).toEqual({ request: sent })
    expect(st.dirty).toBe(false)
    expect(st.saved).toEqual(draftFrom(answered))
    expect(st.error).toBe('')
  })

  it('a_failed_save_keeps_the_draft_and_says_why', async () => {
    vi.mocked(invoke).mockImplementation((async () => { throw 'nope' }) as never)
    const st = new ProfileEditorState(profile())
    st.draft.bio = 'edited'
    await st.save()
    expect(st.error).not.toBe('')
    expect(st.draft.bio).toBe('edited')
    expect(st.dirty).toBe(true)
    expect(st.saving).toBe(false)
  })
})
