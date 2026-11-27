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

// Ported from ProfileSettings.test.ts when the old page went (#727, task 10): what the working copy does around a Save
// and an upload, now that it lives in ProfileEditorState instead of the page.
describe('saving and uploading (ported from the old profile page)', () => {
  type Handler = (args: unknown, opts?: unknown) => unknown
  let handlers: Record<string, Handler>
  const calls = (cmd: string) => vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd)
  const media = (hash: string, still: string | null = null) => ({ hash, content_type: 'image/webp', width: 256, height: 256, frames: still ? 10 : 1, animated: !!still, still_hash: still })
  const crop = { x: 100, y: 0, w: 600, h: 600 }
  const file = () => new File([new Uint8Array([1, 2, 3])], 'a.png', { type: 'image/png' })
  const badge = (key: string, rarity: string) => ({ key, earned_at: '2026-11-01T00:00:00Z', first_earner: false, tier: 0, rarity, first_tiers: [], count: 1, hidden: false, seen: true }) as never
  const held = [badge('a_common', 'common'), badge('b_rare', 'rare'), badge('c_epic', 'epic'), badge('d_legend', 'legendary')]

  beforeEach(() => {
    handlers = {}
    vi.mocked(invoke).mockImplementation((async (cmd: string, args: unknown, opts: unknown) => handlers[cmd]?.(args, opts)) as never)
  })

  it('save_sends_the_whole_profile_once_and_takes_the_answer', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const st = new ProfileEditorState(profile())
    st.draft.themeTop = '#1f7a5a'
    st.draft.themeBottom = '#0f2b3a'
    st.draft.font = 'fredoka'
    st.draft.colors = ['#000000']
    st.draft.pronouns = ''
    const first = st.save()
    void st.save() // a second Save while saving sends nothing
    expect(calls('update_profile')).toHaveLength(1)
    expect(st.saving).toBe(true)
    expect((calls('update_profile')[0][1] as { request: unknown }).request).toEqual({
      avatar_hash: null, banner_hash: null, theme_top: '#1f7a5a', theme_bottom: '#0f2b3a',
      style: { font: 'fredoka', colors: ['#000000'], effect: null }, pronouns: null, bio: 'hello', display_name: 'alex', glow: true, typing_text: '',
      ring_style: 'classic', ring_color: 'green',
    })
    const stored = profile({
      user: { ...alex, style: { font: 'fredoka', colors: ['#a8a8a8'] } },
      card: { ...profile().card, theme_top: '#1f7a5a', theme_bottom: '#0f2b3a', pronouns: null },
    })
    answer(stored)
    await first
    expect(st.dirty).toBe(false)
    // the draft took the server's answer (the lightened colour), and the cache has it
    expect(st.draft.colors).toEqual(['#a8a8a8'])
    expect(app.state.profiles.U1).toEqual(stored)
    expect(st.lastSave).not.toBeNull()
  })

  it('a_refused_save_keeps_the_draft_and_shows_the_message', async () => {
    handlers.update_profile = () => Promise.reject('Pronouns can be up to 40 characters.')
    const st = new ProfileEditorState(profile())
    st.draft.themeTop = '#c2457a'
    await st.save()
    expect(st.error).toBe('Pronouns can be up to 40 characters.')
    expect(st.draft.themeTop).toBe('#c2457a')
    expect(st.dirty).toBe(true)
  })

  it('edits_made_while_a_save_is_on_its_way_are_kept_on_top_of_the_answer', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const st = new ProfileEditorState(profile())
    st.draft.colors = ['#000000']
    const saving = st.save()
    st.draft.themeTop = '#c2457a' // while saving
    st.draft.bio = 'typed meanwhile'
    answer(profile({ user: { ...alex, style: { font: null, colors: ['#a8a8a8'] } } }))
    await saving
    expect(st.draft.themeTop).toBe('#c2457a')
    expect(st.draft.bio).toBe('typed meanwhile')
    // what wasn't touched since takes the server's answer (the lightened colour)
    expect(st.draft.colors).toEqual(['#a8a8a8'])
    expect(st.dirty).toBe(true)
    expect(st.saving).toBe(false)
    expect(app.state.profiles.U1.user.style.colors).toEqual(['#a8a8a8'])
  })

  it('the_answer_reaches_the_cache_even_if_the_editor_has_gone', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const st = new ProfileEditorState(profile())
    st.draft.themeTop = '#c2457a'
    const saving = st.save()
    st.dispose()
    const stored = profile({ card: { ...profile().card, theme_top: '#c2457a' } })
    answer(stored)
    await saving
    expect(app.state.profiles.U1).toEqual(stored)
  })

  it('badge_order_saves_alone_and_rarest_five_sends_none', async () => {
    const chosen = (keys: string[]) => profile({ badges: held, showcase: keys, showcase_chosen: true })
    handlers.get_profile = () => chosen(['b_rare', 'a_common'])
    const st = new ProfileEditorState(chosen(['a_common', 'b_rare']))
    st.showcase = ['b_rare', 'a_common']
    await st.save()
    // only the order, when nothing else changed
    expect(calls('set_badge_showcase')).toHaveLength(1)
    expect(calls('set_badge_showcase')[0][1]).toEqual({ keys: ['b_rare', 'a_common'] })
    expect(calls('update_profile')).toHaveLength(0)
    expect(st.dirty).toBe(false)
    // Rarest five (auto) sends none: the server's own default follows
    handlers.get_profile = () => profile({ badges: held, showcase: ['d_legend', 'c_epic', 'b_rare', 'a_common'], showcase_chosen: false })
    st.showcase = 'auto'
    expect(st.shown).toEqual(['d_legend', 'c_epic', 'b_rare', 'a_common'])
    await st.save()
    expect(calls('set_badge_showcase')).toHaveLength(2)
    expect(calls('set_badge_showcase')[1][1]).toEqual({ keys: [] })
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('invalid_drafts_do_not_save', async () => {
    const st = new ProfileEditorState(profile())
    const refused = async (set: () => void, undo: () => void) => {
      set()
      expect(st.invalid).toBe(true)
      await st.save()
      expect(calls('update_profile')).toHaveLength(0)
      undo()
      expect(st.invalid).toBe(false)
    }
    // bio over 190 (190 is fine)
    st.draft.bio = 'x'.repeat(190)
    expect(st.invalid).toBe(false)
    await refused(() => (st.draft.bio = 'x'.repeat(191)), () => (st.draft.bio = 'hello'))
    // more than 4 line breaks, blank leading lines not counted
    st.draft.bio = '\n\n\na\nb\nc\nd\ne'
    expect(st.invalid).toBe(false)
    await refused(() => (st.draft.bio = 'a\nb\nc\nd\ne\nf'), () => (st.draft.bio = 'hello'))
    // pronouns count characters, not UTF-16 units, up to 40
    st.draft.pronouns = '😀'.repeat(40)
    expect(st.invalid).toBe(false)
    await refused(() => (st.draft.pronouns = '😀'.repeat(41)), () => (st.draft.pronouns = 'they/them'))
    // display name 1 to 32, typing text up to 32 and no links
    await refused(() => (st.draft.displayName = '   '), () => (st.draft.displayName = 'alex'))
    await refused(() => (st.draft.displayName = 'x'.repeat(33)), () => (st.draft.displayName = 'alex'))
    await refused(() => (st.draft.typingText = 'x'.repeat(33)), () => (st.draft.typingText = ''))
    await refused(() => (st.draft.typingText = 'at pulse.app'), () => (st.draft.typingText = ''))
  })

  it('an_upload_stages_the_hash_without_saving', async () => {
    let finish!: (m: ReturnType<typeof media>) => void
    handlers.upload_avatar_media = () => new Promise((r) => (finish = r))
    const st = new ProfileEditorState(profile())
    const up = st.upload('avatar', file(), crop)
    await vi.waitFor(() => expect(calls('upload_avatar_media')).toHaveLength(1))
    const [, bytes, opts] = calls('upload_avatar_media')[0] as unknown as [string, Uint8Array, { headers: Record<string, string> }]
    expect([...bytes]).toEqual([1, 2, 3])
    expect(opts.headers['x-pulse-crop']).toBe('100,0,600,600')
    expect(st.uploading.avatar).toBe(true)
    expect(st.busy).toBe(true)
    await st.save() // an upload in flight blocks Save
    st.draft.bio = 'edited'
    await st.save()
    expect(calls('update_profile')).toHaveLength(0)
    finish(media('AV', 'AVS'))
    await up
    expect([st.draft.avatarHash, st.draft.avatarStill]).toEqual(['AV', 'AVS'])
    expect(st.busy).toBe(false)
    expect(st.dirty).toBe(true)
    expect(calls('update_profile')).toHaveLength(0)
    expect(app.state.me!.avatar_hash).toBeNull()
    handlers.upload_banner_media = () => media('BN')
    await st.upload('banner', file(), crop)
    expect(st.draft.bannerHash).toBe('BN')
    st.remove('banner')
    expect(st.draft.bannerHash).toBeNull()
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('a_refused_upload_keeps_the_draft_and_says_why', async () => {
    const st = new ProfileEditorState(profile({ user: { ...alex, avatar_hash: 'OLD' } }))
    const refusals = [
      'that file is too big (8 MB at most)',
      'that animation has too many frames (200 at most)',
      "that image's dimensions are too large (4096×4096 at most)",
      "that isn't a supported image: use PNG, JPEG, GIF or WebP",
      'The crop is outside the image.',
    ]
    for (const msg of refusals) {
      handlers.upload_avatar_media = () => Promise.reject(msg)
      await st.upload('avatar', file(), crop)
      expect(st.uploadError).toBe(msg)
      expect(st.draft.avatarHash).toBe('OLD')
      expect(st.dirty).toBe(false)
      expect(st.busy).toBe(false)
    }
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('reset_during_an_upload_drops_it_when_it_lands', async () => {
    let finish!: (m: ReturnType<typeof media>) => void
    handlers.upload_avatar_media = () => new Promise((r) => (finish = r))
    const st = new ProfileEditorState(profile())
    const up = st.upload('avatar', file(), crop)
    await vi.waitFor(() => expect(calls('upload_avatar_media')).toHaveLength(1))
    expect(st.busy).toBe(true)
    st.reset()
    expect(st.busy).toBe(false)
    finish(media('LATE'))
    await up
    expect(st.draft.avatarHash).toBeNull()
    expect(st.dirty).toBe(false)
  })

  it('remove_during_an_upload_cancels_it', async () => {
    let finish!: (m: ReturnType<typeof media>) => void
    handlers.upload_avatar_media = () => new Promise((r) => (finish = r))
    const st = new ProfileEditorState(profile({ user: { ...alex, avatar_hash: 'OLD' } }))
    const up = st.upload('avatar', file(), crop)
    await vi.waitFor(() => expect(calls('upload_avatar_media')).toHaveLength(1))
    st.remove('avatar')
    finish(media('LATE'))
    await up
    expect(st.draft.avatarHash).toBeNull()
    expect(st.busy).toBe(false)
  })

  it('reset_of_a_staged_upload_restores_the_saved_picture', async () => {
    handlers.upload_avatar_media = () => media('STAGED')
    const st = new ProfileEditorState(profile({ user: { ...alex, avatar_hash: 'OLD' } }))
    await st.upload('avatar', file(), crop)
    expect(st.draft.avatarHash).toBe('STAGED')
    st.reset()
    expect(st.draft.avatarHash).toBe('OLD')
    expect(st.dirty).toBe(false)
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('discard_leaves_the_saved_profile_and_the_store_untouched', async () => {
    handlers.upload_avatar_media = () => media('STAGED')
    const st = new ProfileEditorState(profile())
    await st.upload('avatar', file(), crop)
    st.draft.colors = ['#ffb6d9', '#a3d8ff']
    st.reset()
    expect(calls('update_profile')).toHaveLength(0)
    expect(app.state.me).toBe(alex)
    expect(app.state.me!.avatar_hash).toBeNull()
    expect(app.state.people.U1.user).toBe(alex)
    expect(app.state.people.U1.user.style).toEqual({ font: null, colors: [] })
  })
})
