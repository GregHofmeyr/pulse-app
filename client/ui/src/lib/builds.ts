// Builds channels (#617): feeds of a channel's builds and pure helpers over them. The feeds live in `AppState.buildFeeds`
// (as clips' do in `clipFeeds`); a build's comments live in `threads` like any thread's replies.
import type { AppState } from './state'
import type { Attachment } from './protocol/Attachment'
import type { BuildFacets } from './protocol/BuildFacets'
import type { EmbedThumb } from './protocol/EmbedThumb'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'

/** Which builds of a channel a feed shows: one game (id), archived ones too, and/or ones with these words. */
export type BuildFilter = { game?: string; archived?: boolean; q?: string }

export type BuildFeed = {
  channelId: string
  filter: BuildFilter
  items: Message[] /* newest first by when saved */
  start: boolean
  loading: boolean
  /** Builds deleted while a page was being fetched (it may still carry them). */
  gone?: string[]
}

/** Builds per page of a feed (the server's page size). */
export const BUILD_PAGE = 30

/** The filter with empty parts unset (the one shape every key and comparison uses). */
export function buildFilter(f: BuildFilter): BuildFilter {
  const q = f.q?.trim()
  return { ...(f.game ? { game: f.game } : {}), ...(f.archived ? { archived: true } : {}), ...(q ? { q } : {}) }
}

/** The key of a channel's feed for a filter; every feed lookup goes through it. */
export function buildFeedKey(channelId: string, f: BuildFilter): string {
  const n = buildFilter(f)
  return JSON.stringify([channelId, n.game ?? '', n.archived ? 1 : 0, n.q ?? ''])
}

/** A channel's feed for a filter, if started. */
export function buildFeedOf(s: AppState, channelId: string, f: BuildFilter): BuildFeed | undefined {
  return s.buildFeeds[buildFeedKey(channelId, f)]
}

/** Whether a build belongs in a feed with this filter, by its game and archived state (a feed with words is fetched
 * again rather than matched here). */
export function matchesBuildFilter(m: Message, f: BuildFilter): boolean {
  if (!m.build) return false
  return (!f.game || m.build.game_id === f.game) && (!!f.archived || !m.build.archived_at)
}

export const isQuickDrop = (m: Message): boolean => !!m.build?.quick_drop
export const isArchived = (m: Message): boolean => !!m.build?.archived_at

/** What a card shows as its picture: the first screenshot, else the link's thumbnail, else a plain tile. */
export type BuildThumb = { kind: 'shot'; attachment: Attachment } | { kind: 'video'; thumb: EmbedThumb } | { kind: 'none' }

export function buildThumb(m: Message): BuildThumb {
  const shot = m.attachments?.find((a) => a.kind === 'image' && !a.expired)
  if (shot) return { kind: 'shot', attachment: shot }
  const thumb = m.build?.link?.thumb
  return thumb ? { kind: 'video', thumb } : { kind: 'none' }
}

const newestFirst = (a: Message, b: Message) => (a.id < b.id ? 1 : a.id > b.id ? -1 : 0)

/** Start (or keep) a channel's feed for a filter, ready to be filled by `loadBuildPage`. */
export function openBuildFeed(s: AppState, channelId: string, f: BuildFilter): AppState {
  const key = buildFeedKey(channelId, f)
  const feed = s.buildFeeds[key]
  if (feed) return feed.loading ? s : { ...s, buildFeeds: { ...s.buildFeeds, [key]: { ...feed, loading: true } } }
  return { ...s, buildFeeds: { ...s.buildFeeds, [key]: { channelId, filter: buildFilter(f), items: [], start: false, loading: true } } }
}

/** A fetched page (the newest, or one older than what's held: merged by id). Builds that arrived live meanwhile stay,
 * once each; ones deleted while it was in flight stay out; a short page reaches the start. */
export function loadBuildPage(s: AppState, key: string, page: Message[]): AppState {
  const feed = s.buildFeeds[key]
  if (!feed) return s
  const seen = new Set(feed.items.map((m) => m.id))
  const gone = new Set(feed.gone)
  const items = [...feed.items, ...page.filter((m) => !seen.has(m.id) && !gone.has(m.id))].sort(newestFirst)
  return { ...s, buildFeeds: { ...s.buildFeeds, [key]: { ...feed, items, start: page.length < BUILD_PAGE, loading: false, gone: undefined } } }
}

/** A feed's fetch failed: it can be tried again. */
export function buildFeedFailed(s: AppState, key: string): AppState {
  const feed = s.buildFeeds[key]
  return feed?.loading ? { ...s, buildFeeds: { ...s.buildFeeds, [key]: { ...feed, loading: false } } } : s
}

/** A channel's feed thrown away, so the next look fetches it fresh (a feed with words, after a build event). */
export function dropBuildFeed(s: AppState, key: string): AppState {
  if (!(key in s.buildFeeds)) return s
  const { [key]: _gone, ...rest } = s.buildFeeds
  return { ...s, buildFeeds: rest }
}

const facetsKey = (channelId: string, archived: boolean) => `${channelId}|${archived ? 1 : 0}`

/** A channel's per-game counts as last fetched, with or without archived builds. */
export function buildFacetsOf(s: AppState, channelId: string, archived: boolean): BuildFacets | undefined {
  return s.buildFacets[facetsKey(channelId, archived)]
}

export function setBuildFacets(s: AppState, channelId: string, archived: boolean, f: BuildFacets): AppState {
  return { ...s, buildFacets: { ...s.buildFacets, [facetsKey(channelId, archived)]: f } }
}

/** `m` as an event carries it, with what only the held copy knows: your seen point, your reactions and its pin. */
function withHeld(m: Message, old: Message): Message {
  const { pin: _p, ...rest } = m
  const next: Message = { ...rest, reactions: old.reactions, ...(old.pin ? { pin: old.pin } : {}) }
  return m.build ? { ...next, build: { ...m.build, replies_seen: old.build?.replies_seen ?? m.build.replies_seen } } : next
}

/** Every feed of `channel` through `f` (the same state when nothing changed). */
function mapFeeds(s: AppState, channel: string, f: (feed: BuildFeed) => BuildFeed): AppState {
  let feeds = s.buildFeeds
  for (const [k, feed] of Object.entries(s.buildFeeds)) {
    if (feed.channelId !== channel) continue
    const next = f(feed)
    if (next !== feed) feeds = { ...feeds, [k]: next }
  }
  return feeds === s.buildFeeds ? s : { ...s, buildFeeds: feeds }
}

function created(s: AppState, m: Message): AppState {
  return mapFeeds(s, m.channel_id, (feed) =>
    feed.filter.q || !matchesBuildFilter(m, feed.filter) || feed.items.some((x) => x.id === m.id)
      ? feed
      : { ...feed, items: [...feed.items, m].sort(newestFirst) },
  )
}

/** Replaced where it still fits, dropped where it no longer does, added where its edit now fits (only as far back as
 * the feed reaches). A feed with words keeps its copy fresh but is never joined here. */
function updated(s: AppState, m: Message): AppState {
  return mapFeeds(s, m.channel_id, (feed) => {
    const old = feed.items.find((x) => x.id === m.id)
    const fits = matchesBuildFilter(m, feed.filter)
    if (!old) {
      const reaches = feed.start || (feed.items.length > 0 && m.id > feed.items[feed.items.length - 1].id)
      return fits && reaches && !feed.filter.q ? { ...feed, items: [...feed.items, m].sort(newestFirst) } : feed
    }
    const items = fits ? feed.items.map((x) => (x.id === m.id ? withHeld(m, x) : x)) : feed.items.filter((x) => x.id !== m.id)
    return { ...feed, items }
  })
}

function deleted(s: AppState, channel: string, id: string): AppState {
  return mapFeeds(s, channel, (feed) => {
    const held = feed.items.some((m) => m.id === id)
    if (!held && !feed.loading) return feed
    return { ...feed, items: feed.items.filter((m) => m.id !== id), gone: feed.loading ? [...(feed.gone ?? []), id] : feed.gone }
  })
}

function archived(s: AppState, channel: string, ids: string[], at: string, by: string): AppState {
  const listed = new Set(ids)
  return mapFeeds(s, channel, (feed) => {
    if (!feed.items.some((m) => listed.has(m.id))) return feed
    const items = feed.filter.archived
      ? feed.items.map((m) => (listed.has(m.id) && m.build ? { ...m, build: { ...m.build, archived_at: at, archived_by: by } } : m))
      : feed.items.filter((m) => !listed.has(m.id))
    return { ...feed, items }
  })
}

/** The build feeds after a gateway event (`applyEvent` calls it for builds and their deletes). */
export function buildsOnEvent(s: AppState, e: Event): AppState {
  switch (e.t) {
    case 'MessageCreated':
      return e.d.message.build && !e.d.message.thread_root_id ? created(s, e.d.message) : s
    case 'MessageUpdated':
      return e.d.message.build && !e.d.message.thread_root_id ? updated(s, e.d.message) : s
    case 'MessageDeleted':
      return e.d.thread_root_id ? s : deleted(s, e.d.channel_id, e.d.message_id)
    case 'BuildsArchived':
      return archived(s, e.d.channel_id, e.d.message_ids, e.d.archived_at, e.d.archived_by)
    default:
      return s
  }
}

/** A build's game, by name in its server's registry (null with no game, or one not arrived yet). */
export function buildGameName(s: AppState, serverId: string, m: Message): string | null {
  const id = m.build?.game_id
  return id ? ((s.games[serverId] ?? []).find((g) => g.id === id)?.name ?? null) : null
}

/** "1 comment", "3 comments"; nothing with none. */
export function commentCount(m: Message): string | null {
  const n = m.build?.reply_count ?? 0
  return n === 0 ? null : n === 1 ? '1 comment' : `${n} comments`
}

/** A screenshot as the build page shows it: its link once signed (null until then) and its size. */
export type BuildScreenshot = { id: string; url: string | null; w: number | null; h: number | null }

/** A build's screenshots: its image files, in order (an expired one is left out). */
export function buildShots(m: Message): Attachment[] {
  return (m.attachments ?? []).filter((a) => a.kind === 'image' && !a.expired)
}

/** Whether you may edit, archive (or restore) and delete a build: its author all three, the server's owner the last two
 *  (spec §5.1; the server checks again). */
export function buildRights(s: AppState, serverId: string, m: Message): { edit: boolean; archive: boolean; delete: boolean } {
  const me = s.me?.id
  const author = !!me && m.author_id === me
  const owner = !!me && s.servers.find((x) => x.id === serverId)?.owner_id === me
  return { edit: author, archive: author || owner, delete: author || owner }
}

/** The build page's quick-access menu entries: Pin to side (#619) and Pop out (#620). */
export type BuildActionItem = 'pin' | 'popout'
