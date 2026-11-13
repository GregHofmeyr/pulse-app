// Message search: the last search run and its results (a snapshot: nothing updates them live), and
// where a clicked result goes.
import { openMessage } from './openMessage'
import { api, errorText } from './tauri'
import { toRequest, type Chip } from './searchQuery'
import type { SearchHit } from './protocol/SearchHit'

/** In a server: all of it. In a DM or group: that conversation. */
export type SearchScope = { kind: 'server'; serverId: string } | { kind: 'conversation'; channelId: string }

class SearchStore {
  /** The results panel is showing. */
  open = $state(false)
  /** Kept behind a voice channel's chat (#230): a hit there opened the chat in the right column, and closing it
   *  brings the results back. A new search (or closing it) clears this. */
  behindChat = $state(false)
  scope = $state.raw<SearchScope | null>(null)
  /** What the shown results were searched for. */
  words = $state('')
  chips = $state.raw<Chip[]>([])
  status = $state<'idle' | 'loading' | 'done' | 'error'>('idle')
  hits = $state.raw<SearchHit[]>([])
  /** Matches counted, up to 51 (shown as "50+"). */
  total = $state<number | null>(null)
  nextCursor = $state<string | null>(null)
  error = $state('')
  loadingMore = $state(false)
}
export const search = new SearchStore()

/** Bumped by every search and close: an answer for an older one is dropped. */
let seq = 0

const scopeArgs = (s: SearchScope) => (s.kind === 'server' ? { serverId: s.serverId, channelId: null } : { serverId: null, channelId: s.channelId })

export async function runSearch(scope: SearchScope, words: string, chips: Chip[]): Promise<void> {
  const mine = ++seq
  Object.assign(search, { open: true, behindChat: false, scope, words, chips, status: 'loading', hits: [], total: null, nextCursor: null, error: '', loadingMore: false })
  try {
    const r = await api.searchMessages(scopeArgs(scope), toRequest(words, chips))
    if (mine !== seq) return
    Object.assign(search, { status: 'done', hits: r.hits, total: r.total, nextCursor: r.next_cursor })
  } catch (e) {
    if (mine !== seq) return
    Object.assign(search, { status: 'error', error: errorText(e) })
  }
}

/** The same search again (after a failure). */
export function retrySearch(): Promise<void> {
  return search.scope ? runSearch(search.scope, search.words, search.chips) : Promise.resolve()
}

/** The next (older) page, added below. A failure leaves the list as it is and says so. */
export async function loadMore(): Promise<void> {
  const { scope, nextCursor } = search
  if (!scope || !nextCursor || search.loadingMore || search.status !== 'done') return
  const mine = seq
  search.loadingMore = true
  try {
    const r = await api.searchMessages(scopeArgs(scope), toRequest(search.words, search.chips, nextCursor))
    if (mine !== seq) return
    Object.assign(search, { hits: [...search.hits, ...r.hits], nextCursor: r.next_cursor, error: '' })
  } catch (e) {
    if (mine === seq) search.error = errorText(e)
  } finally {
    if (mine === seq) search.loadingMore = false
  }
}

export function closeSearch(): void {
  seq++
  Object.assign(search, { open: false, behindChat: false, scope: null, words: '', chips: [], status: 'idle', hits: [], total: null, nextCursor: null, error: '', loadingMore: false })
}

/** Show a result where it lives (lib/openMessage.ts: a clip's panel, a quote's replies, or the message in its
 *  channel); the results stay open for the next one. */
export function openHit(hit: SearchHit): void {
  openMessage(hit.message, hit.root)
}
