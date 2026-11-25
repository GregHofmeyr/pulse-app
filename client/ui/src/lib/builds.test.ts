import { describe, expect, it } from 'vitest'
import { buildFeedKey, buildThumb, isArchived, isQuickDrop, loadBuildPage, matchesBuildFilter, openBuildFeed, buildFeedFailed, setBuildFacets, buildFacetsOf } from './builds'
import { applyEvent, applyReady, emptyState, type AppState } from './state'
import type { Message } from './protocol/Message'
import type { Build } from './protocol/Build'
import type { Ready } from './protocol/Ready'

const build = (id: string, b: Partial<Build> = {}, m: Partial<Message> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: 'Raven bunny', title_auto: false, game_id: 'G1', game_guessed: false, link: null, notes: null, quick_drop: false,
    updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
  ...m,
})

const created = (m: Message) => ({ t: 'MessageCreated' as const, d: { message: m, nonce: null } })
const updated = (m: Message) => ({ t: 'MessageUpdated' as const, d: { message: m } })

const ids = (s: AppState, key: string) => s.buildFeeds[key]?.items.map((m) => m.id)

/** Feeds of channel B1 for: everything live, game G1, and everything with archived. */
function withFeeds(page: Message[] = []): AppState {
  let s = emptyState()
  for (const f of [{}, { game: 'G1' }, { archived: true }]) {
    s = openBuildFeed(s, 'B1', f)
    s = loadBuildPage(s, buildFeedKey('B1', f), page.filter((m) => matchesBuildFilter(m, f)))
  }
  return s
}

describe('build feeds', () => {
  it('a new build joins only feeds it matches', () => {
    let s = withFeeds([build('M1')])
    s = applyEvent(s, created(build('M2', { game_id: 'G2' })), 0)
    expect(ids(s, buildFeedKey('B1', {}))).toEqual(['M2', 'M1'])
    expect(ids(s, buildFeedKey('B1', { game: 'G1' }))).toEqual(['M1'])
    expect(ids(s, buildFeedKey('B1', { archived: true }))).toEqual(['M2', 'M1'])
    // a build is never a text message of its channel
    expect(s.messages['B1']).toBeUndefined()
  })

  it('a search feed is refetched, not joined', () => {
    let s = openBuildFeed(emptyState(), 'B1', { q: 'raven' })
    s = loadBuildPage(s, buildFeedKey('B1', { q: 'raven' }), [])
    s = applyEvent(s, created(build('M2')), 0)
    expect(ids(s, buildFeedKey('B1', { q: 'raven' }))).toEqual([])
  })

  it('an update keeps its place and its seen point', () => {
    let s = withFeeds([build('M3'), build('M2', { replies_seen: 'R5', reply_count: 2 }, { reactions: [{ emoji: '👍', count: 1, me: true, users: [] } as never] }), build('M1')])
    s = applyEvent(s, updated(build('M2', { title: 'Raven bunny v2', updated_at: '2026-10-09T00:00:00.000Z', reply_count: 2 })), 0)
    const feed = s.buildFeeds[buildFeedKey('B1', {})]
    expect(feed.items.map((m) => m.id)).toEqual(['M3', 'M2', 'M1'])
    const m2 = feed.items[1]
    expect(m2.build?.title).toBe('Raven bunny v2')
    expect(m2.build?.replies_seen).toBe('R5')
    expect(m2.reactions).toHaveLength(1)
  })

  it('a game change moves it between game feeds', () => {
    let s = withFeeds([build('M2'), build('M1')])
    s = applyEvent(s, updated(build('M2', { game_id: 'G2' })), 0)
    expect(ids(s, buildFeedKey('B1', { game: 'G1' }))).toEqual(['M1'])
  })

  it('archiving drops it from the default feed and greys it in the archived one', () => {
    let s = withFeeds([build('M2'), build('M1')])
    s = applyEvent(s, updated(build('M2', { archived_at: '2026-10-09T00:00:00.000Z', archived_by: 'U2' })), 0)
    expect(ids(s, buildFeedKey('B1', {}))).toEqual(['M1'])
    const shown = s.buildFeeds[buildFeedKey('B1', { archived: true })].items
    expect(shown.map((m) => m.id)).toEqual(['M2', 'M1'])
    expect(isArchived(shown[0])).toBe(true)
  })

  it('BuildsArchived updates every listed build', () => {
    let s = withFeeds([build('M3'), build('M2'), build('M1')])
    s = applyEvent(s, { t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M1', 'M2'], archived_at: '2026-10-09T00:00:00.000Z', archived_by: 'U9' } }, 0)
    expect(ids(s, buildFeedKey('B1', {}))).toEqual(['M3'])
    expect(ids(s, buildFeedKey('B1', { game: 'G1' }))).toEqual(['M3'])
    const all = s.buildFeeds[buildFeedKey('B1', { archived: true })].items
    expect(all.map((m) => [m.id, m.build?.archived_by ?? null])).toEqual([['M3', null], ['M2', 'U9'], ['M1', 'U9']])
  })

  it('BuildFavouriteChanged stars or unstars your copies in every feed of the channel (#617)', () => {
    let s = withFeeds([build('M3'), build('M2'), build('M1')])
    s = applyEvent(s, { t: 'BuildFavouriteChanged', d: { user_id: 'U1', channel_id: 'B1', message_ids: ['M1', 'M3'], favourite: true } }, 0)
    const stars = (k: string) => s.buildFeeds[k].items.map((m) => [m.id, !!m.build?.favourite])
    expect(stars(buildFeedKey('B1', {}))).toEqual([['M3', true], ['M2', false], ['M1', true]])
    expect(stars(buildFeedKey('B1', { archived: true }))).toEqual([['M3', true], ['M2', false], ['M1', true]])
    s = applyEvent(s, { t: 'BuildFavouriteChanged', d: { user_id: 'U1', channel_id: 'B1', message_ids: ['M3'], favourite: false } }, 0)
    expect(stars(buildFeedKey('B1', {}))).toEqual([['M3', false], ['M2', false], ['M1', true]])
  })

  it("an edit's event (which carries no one's star) keeps yours", () => {
    let s = withFeeds([build('M1')])
    s = applyEvent(s, { t: 'BuildFavouriteChanged', d: { user_id: 'U1', channel_id: 'B1', message_ids: ['M1'], favourite: true } }, 0)
    s = applyEvent(s, updated(build('M1', { title: 'Raven v2' })), 0)
    const m = s.buildFeeds[buildFeedKey('B1', {})].items[0]
    expect([m.build?.title, m.build?.favourite]).toEqual(['Raven v2', true])
  })

  it('a deleted build leaves every feed', () => {
    let s = withFeeds([build('M2'), build('M1')])
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M2', author_id: 'U2', mentions: [], thread_root_id: null } } as never, 0)
    for (const f of [{}, { game: 'G1' }, { archived: true }]) expect(ids(s, buildFeedKey('B1', f))).toEqual(['M1'])
  })

  it('a comment counts on its build', () => {
    let s = withFeeds([build('M1')])
    s = applyEvent(s, created({ ...build('R1'), build: undefined, thread_root_id: 'M1', author_id: 'U3' }), 0)
    expect(s.buildFeeds[buildFeedKey('B1', {})].items[0].build?.reply_count).toBe(1)
  })

  it('a short page reaches the start; a failed fetch can be tried again', () => {
    let s = openBuildFeed(emptyState(), 'B1', {})
    const key = buildFeedKey('B1', {})
    expect(s.buildFeeds[key].loading).toBe(true)
    s = buildFeedFailed(s, key)
    expect(s.buildFeeds[key].loading).toBe(false)
    s = loadBuildPage(openBuildFeed(s, 'B1', {}), key, [build('M1')])
    expect(s.buildFeeds[key].start).toBe(true)
  })

  it('a Ready clears feeds and counts', () => {
    let s = setBuildFacets(withFeeds([build('M1')]), 'B1', false, { total: 1, games: [{ game_id: 'G1', count: 1 }] })
    expect(buildFacetsOf(s, 'B1', false)?.total).toBe(1)
    s = applyReady(s, { me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, servers: [], channels: [], members: [], dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [] } as unknown as Ready)
    expect(s.buildFeeds).toEqual({})
    expect(s.buildFacets).toEqual({})
  })

  it('a build created, updated, deleted or bulk-archived drops both of its channel\'s counts', () => {
    const counts = { total: 1, games: [{ game_id: 'G1', count: 1 }] }
    const counted = () => setBuildFacets(setBuildFacets(setBuildFacets(withFeeds([build('M1')]), 'B1', false, counts), 'B1', true, counts), 'B2', false, counts)
    const events = [
      created(build('M2')),
      updated(build('M1', { title: 'Raven bunny v2' })),
      { t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M1', author_id: 'U2', mentions: [], thread_root_id: null } },
      { t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M1'], archived_at: '2026-10-09T00:00:00.000Z', archived_by: 'U2' } },
    ]
    for (const e of events) {
      const s = applyEvent(counted(), e as never, 0)
      expect([e.t, buildFacetsOf(s, 'B1', false), buildFacetsOf(s, 'B1', true)]).toEqual([e.t, undefined, undefined])
      // another channel's counts stay
      expect(buildFacetsOf(s, 'B2', false)?.total).toBe(1)
    }
    // a comment on a build doesn't change the counts
    const s = applyEvent(counted(), created({ ...build('R1'), build: undefined, thread_root_id: 'M1', author_id: 'U3' }), 0)
    expect(buildFacetsOf(s, 'B1', false)?.total).toBe(1)
  })

  it('keys differ per filter, and an empty part is unset', () => {
    expect(buildFeedKey('B1', {})).toBe(buildFeedKey('B1', { game: '', q: '  ', archived: false }))
    expect(buildFeedKey('B1', { archived: true })).not.toBe(buildFeedKey('B1', {}))
    expect(buildFeedKey('B1', { q: 'raven' })).not.toBe(buildFeedKey('B1', {}))
  })
})

describe('build helpers', () => {
  it('quick drop and archived read the build', () => {
    expect(isQuickDrop(build('M1', { quick_drop: true }))).toBe(true)
    expect(isQuickDrop(build('M1'))).toBe(false)
    expect(isArchived(build('M1', { archived_at: '2026-10-09T00:00:00.000Z' }))).toBe(true)
  })

  it('thumb prefers a screenshot then the video', () => {
    const shot = { id: 'A1', name: 'a.png', content_type: 'image/png', kind: 'image', size: 1, width: 10, height: 10, duration_ms: null, expires_at: null, expired: false } as never
    const link = { url: 'https://youtu.be/x', title: 'x', thumb: { hash: 'h', width: 16, height: 9 }, provider: 'youtube', provider_id: 'x' } as never
    expect(buildThumb(build('M1', { link }, { attachments: [shot] }))).toEqual({ kind: 'shot', attachment: shot })
    expect(buildThumb(build('M1', { link }))).toEqual({ kind: 'video', thumb: { hash: 'h', width: 16, height: 9 } })
    expect(buildThumb(build('M1', { link: { ...(link as object), thumb: null } as never }))).toEqual({ kind: 'none' })
    expect(buildThumb(build('M1'))).toEqual({ kind: 'none' })
  })
})

describe('a channel kind this client does not know', () => {
  const later = { id: 'X1', server_id: 'S1', kind: 'unknown' as const, name: 'later', position: 9 }
  const ready = { me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, servers: [{ id: 'S1', name: 'Main', icon_hash: null }], channels: [{ id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }, later], members: [{ server_id: 'S1', members: [{ user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, nickname: null }] }], dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [] } as unknown as Ready

  it('is left out of a Ready', () => {
    const s = applyReady(emptyState(), ready)
    expect(Object.keys(s.channels)).toEqual(['T1'])
  })

  it('is never added by an event, and one that becomes unknown goes', () => {
    let s = emptyState()
    s = applyEvent(s, { t: 'ChannelCreated', d: { channel: later } } as never, 0)
    expect(s.channels['X1']).toBeUndefined()
    s = applyEvent(s, { t: 'ChannelCreated', d: { channel: { ...later, kind: 'text' } } } as never, 0)
    expect(s.channels['X1']).toBeDefined()
    s = applyEvent(s, { t: 'ChannelUpdated', d: { channel: later } } as never, 0)
    expect(s.channels['X1']).toBeUndefined()
  })
})
