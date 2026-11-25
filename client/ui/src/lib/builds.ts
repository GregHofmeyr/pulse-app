// Builds channels (#617): feeds of a channel's builds and pure helpers over them. The feeds live in `AppState.buildFeeds`
// (as clips' do in `clipFeeds`); a build's comments live in `threads` like any thread's replies.
import type { AppState } from './state'
import type { Attachment } from './protocol/Attachment'
import type { BuildFacets } from './protocol/BuildFacets'
import type { EmbedThumb } from './protocol/EmbedThumb'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'

/** Which builds of a channel a feed shows: one game (id), archived ones too, ones with these words, and/or only the ones
 *  you starred (the Favourites section's own feed, #617). */
export type BuildFilter = { game?: string; archived?: boolean; q?: string; favourite?: boolean }

export type BuildFeed = {
  channelId: string
  filter: BuildFilter
  items: Message[] /* newest first by when saved */
  start: boolean
  loading: boolean
  /** Builds deleted while a page was being fetched (it may still carry them). */
  gone?: string[]
  /** Builds an event added with no copy of yours to take your star and seen point from: to be fetched as yours
   *  (`refreshedBuild`). */
  unsure?: string[]
}

/** Builds per page of a feed (the server's page size). */
export const BUILD_PAGE = 30

/** The filter with empty parts unset (the one shape every key and comparison uses). */
export function buildFilter(f: BuildFilter): BuildFilter {
  const q = f.q?.trim()
  return { ...(f.game ? { game: f.game } : {}), ...(f.archived ? { archived: true } : {}), ...(q ? { q } : {}), ...(f.favourite ? { favourite: true } : {}) }
}

/** The key of a channel's feed for a filter; every feed lookup goes through it. */
export function buildFeedKey(channelId: string, f: BuildFilter): string {
  const n = buildFilter(f)
  return JSON.stringify([channelId, n.game ?? '', n.archived ? 1 : 0, n.q ?? '', ...(n.favourite ? [1] : [])])
}

/** A channel's feed for a filter, if started. */
export function buildFeedOf(s: AppState, channelId: string, f: BuildFilter): BuildFeed | undefined {
  return s.buildFeeds[buildFeedKey(channelId, f)]
}

/** Whether a build belongs in a feed with this filter, by its game, archived state and your star (a feed with words is
 * fetched again rather than matched here). */
export function matchesBuildFilter(m: Message, f: BuildFilter): boolean {
  if (!m.build) return false
  return (!f.game || m.build.game_id === f.game) && (!!f.archived || !m.build.archived_at) && (!f.favourite || !!m.build.favourite)
}

export const isQuickDrop = (m: Message): boolean => !!m.build?.quick_drop
export const isArchived = (m: Message): boolean => !!m.build?.archived_at

/** What a build shows as its picture: its YouTube video's thumbnail, else the first screenshot, else another link's
 *  preview image, else a plain tile. */
export type BuildThumb = { kind: 'shot'; attachment: Attachment } | { kind: 'video'; thumb: EmbedThumb } | { kind: 'none' }

export function buildThumb(m: Message): BuildThumb {
  const link = m.build?.link
  if (link?.provider === 'youtube' && link.thumb) return { kind: 'video', thumb: link.thumb }
  const shot = m.attachments?.find((a) => a.kind === 'image' && !a.expired)
  if (shot) return { kind: 'shot', attachment: shot }
  return link?.thumb ? { kind: 'video', thumb: link.thumb } : { kind: 'none' }
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

/** `m` (a build as an event carries it, with no one's seen point or star) with yours from `old`, a copy of yours. */
export function withMyBuildView(m: Message, old: Message | undefined): Message {
  if (!m.build || !old?.build) return m
  return { ...m, build: { ...m.build, replies_seen: old.build.replies_seen ?? m.build.replies_seen, favourite: old.build.favourite ?? m.build.favourite } }
}

/** `m` as an event carries it, with what only the held copy knows: your seen point, your star, your reactions and its
 *  pin. */
function withHeld(m: Message, old: Message): Message {
  const { pin: _p, ...rest } = m
  return withMyBuildView({ ...rest, reactions: old.reactions, ...(old.pin ? { pin: old.pin } : {}) }, old)
}

/** A copy of yours of a channel's build: in any of its feeds, else the open build page's. */
function heldCopy(s: AppState, channel: string, id: string): Message | undefined {
  for (const feed of Object.values(s.buildFeeds)) {
    if (feed.channelId !== channel) continue
    const m = feed.items.find((x) => x.id === id)
    if (m) return m
  }
  return s.openRoot?.id === id && s.openRoot.build ? s.openRoot : undefined
}

/** Whether a feed holds builds back to `id` (so one added there leaves no gap before its next page). */
const reaches = (feed: BuildFeed, id: string) => feed.start || (feed.items.length > 0 && id > feed.items[feed.items.length - 1].id)

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
  // Joining a feed it wasn't in (a restore, a game change): with your star and seen point from any copy of yours, else
  // marked to be fetched as yours (the event's copy says not starred, whoever you are).
  const held = heldCopy(s, m.channel_id, m.id)
  const mine = held ? withHeld(m, held) : m
  return mapFeeds(s, m.channel_id, (feed) => {
    const old = feed.items.find((x) => x.id === m.id)
    if (!old) {
      const join = matchesBuildFilter(mine, feed.filter) && reaches(feed, m.id) && !feed.filter.q
      if (!join) return feed
      return { ...feed, items: [...feed.items, mine].sort(newestFirst), ...(held ? {} : { unsure: [...(feed.unsure ?? []), m.id] }) }
    }
    const next = withHeld(m, old)
    const items = matchesBuildFilter(next, feed.filter) ? feed.items.map((x) => (x.id === m.id ? next : x)) : feed.items.filter((x) => x.id !== m.id)
    return { ...feed, items }
  })
}

/** The builds of a channel's feeds still to be fetched as yours (see `BuildFeed.unsure`), once each. */
export function unsureBuilds(s: AppState, channel: string): string[] {
  const out = new Set<string>()
  for (const feed of Object.values(s.buildFeeds)) if (feed.channelId === channel) for (const id of feed.unsure ?? []) out.add(id)
  return [...out]
}

/** A build fetched as yours (`GET /builds/{id}`) after an event added it unsure: your star and seen point on every
 *  copy, the build in or out of the Favourites feeds by your star, and no longer unsure. */
export function refreshedBuild(s: AppState, m: Message): AppState {
  if (!m.build) return s
  const seen = m.build.replies_seen
  const next = starred(s, m.channel_id, [m.id], !!m.build.favourite, m)
  return mapFeeds(next, m.channel_id, (feed) => {
    const unsure = feed.unsure?.includes(m.id)
    const held = feed.items.some((x) => x.id === m.id && x.build && x.build.replies_seen !== seen)
    if (!unsure && !held) return feed
    const items = held ? feed.items.map((x) => (x.id === m.id && x.build ? { ...x, build: { ...x.build, replies_seen: seen } } : x)) : feed.items
    const rest = feed.unsure?.filter((id) => id !== m.id)
    return { ...feed, items, unsure: rest?.length ? rest : undefined }
  })
}

/** A build no longer unsure in any feed (its fetch failed: it stays as the event had it). */
export function settleBuild(s: AppState, channel: string, id: string): AppState {
  return mapFeeds(s, channel, (feed) => {
    if (!feed.unsure?.includes(id)) return feed
    const rest = feed.unsure.filter((x) => x !== id)
    return { ...feed, unsure: rest.length ? rest : undefined }
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

/** Your star on these builds (#617), in every feed of the channel: a starred one joins the Favourites feeds it fits
 *  (from a copy held anywhere, or `fetched`; held nowhere, it's marked to be fetched), an unstarred one leaves them. A Favourites feed with words is fetched again
 *  rather than joined here, as any feed with words. */
function starred(s: AppState, channel: string, ids: string[], favourite: boolean, fetched?: Message): AppState {
  const listed = new Set(ids)
  const star = (m: Message): Message => (m.build ? { ...m, build: { ...m.build, favourite } } : m)
  const copies = new Map<string, Message>()
  if (favourite) {
    for (const id of ids) {
      const m = fetched?.id === id ? fetched : heldCopy(s, channel, id)
      if (m) copies.set(id, star(m))
    }
  }
  return mapFeeds(s, channel, (feed) => {
    let items = feed.items.some((m) => listed.has(m.id)) ? feed.items.map((m) => (listed.has(m.id) ? star(m) : m)) : feed.items
    if (feed.filter.favourite) {
      if (!favourite) items = items.filter((m) => !listed.has(m.id))
      else if (!feed.filter.q) {
        const have = new Set(items.map((m) => m.id))
        const join = [...copies.values()].filter((m) => !have.has(m.id) && matchesBuildFilter(m, feed.filter) && reaches(feed, m.id))
        if (join.length) items = [...items, ...join].sort(newestFirst)
        // starred elsewhere, held nowhere: fetched as yours (`unsureBuilds`), then joined by `refreshedBuild`
        const unheld = ids.filter((id) => !copies.has(id) && !have.has(id) && !feed.unsure?.includes(id))
        if (unheld.length) return { ...feed, items, unsure: [...(feed.unsure ?? []), ...unheld] }
      }
    }
    return items === feed.items ? feed : { ...feed, items }
  })
}

/** Whether you starred a build (absent: from an older server, not starred). */
export const isFavourite = (m: Message): boolean => !!m.build?.favourite

/** A channel's counts thrown away, both with and without archived builds, so the next look fetches them fresh (after
 * any change to its builds, whether its view is open or not). */
export function dropBuildFacets(s: AppState, channelId: string): AppState {
  const a = facetsKey(channelId, false)
  const b = facetsKey(channelId, true)
  if (!(a in s.buildFacets) && !(b in s.buildFacets)) return s
  const { [a]: _a, [b]: _b, ...rest } = s.buildFacets
  return { ...s, buildFacets: rest }
}

/** The build feeds and counts after a gateway event (`applyEvent` calls it for builds and their deletes). */
export function buildsOnEvent(s: AppState, e: Event): AppState {
  switch (e.t) {
    case 'MessageCreated':
      return e.d.message.build && !e.d.message.thread_root_id ? dropBuildFacets(created(s, e.d.message), e.d.message.channel_id) : s
    case 'MessageUpdated':
      return e.d.message.build && !e.d.message.thread_root_id ? dropBuildFacets(updated(s, e.d.message), e.d.message.channel_id) : s
    case 'MessageDeleted':
      return e.d.thread_root_id ? s : dropBuildFacets(deleted(s, e.d.channel_id, e.d.message_id), e.d.channel_id)
    case 'BuildsArchived':
      return dropBuildFacets(archived(s, e.d.channel_id, e.d.message_ids, e.d.archived_at, e.d.archived_by), e.d.channel_id)
    // a star changes no counts
    case 'BuildFavouriteChanged':
      return starred(s, e.d.channel_id, e.d.message_ids, e.d.favourite)
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
