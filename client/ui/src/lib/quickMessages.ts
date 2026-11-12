// Message search in the quick switcher (#320): the slow section. The instant sections (lib/quickSwitch.ts) answer on
// every key; this one waits until typing pauses, asks the existing search route, and shows at most a few hits under
// them. A newer query drops an older one's answer, so a slow reply can never land on the wrong text.
import { hasSearchableWords } from './searchQuery'
import { parseQuery } from './quickSwitch'
import type { SearchHit } from './protocol/SearchHit'

/** How long typing has to pause before messages are searched. */
export const MESSAGE_DELAY_MS = 200
/** At most this many message hits are listed. */
export const MESSAGES_SHOWN = 8

/** What to search messages for, or null: not after `>` (actions), not for a single character, and not for text with
 *  no letters or digits (the index holds words: the route can only refuse it). */
export function messageQuery(raw: string): string | null {
  const q = parseQuery(raw)
  if (q.actions || q.text.length < 2 || !hasSearchableWords(q.text)) return null
  return q.text
}

export interface MessageResults {
  /** `idle`: nothing asked (or no longer); `loading`: waiting for the pause or the answer. */
  status: 'idle' | 'loading' | 'done' | 'error'
  /** What `hits` (or the wait) are for. */
  query: string
  hits: SearchHit[]
}

export const NO_MESSAGES: MessageResults = { status: 'idle', query: '', hits: [] }

/** Debounced, cancellable message search. `set` with each new query (null: none); `onChange` hears every state.
 *  Only the newest query's answer is ever reported; `dispose` drops whatever is pending. */
export function messageSearcher(o: {
  search: (query: string) => Promise<SearchHit[]>
  onChange: (r: MessageResults) => void
  delay?: number
  limit?: number
}) {
  const delay = o.delay ?? MESSAGE_DELAY_MS
  const limit = o.limit ?? MESSAGES_SHOWN
  let seq = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  let current: string | null = null

  const cancel = () => {
    seq++
    if (timer !== undefined) clearTimeout(timer)
    timer = undefined
  }

  return {
    set(query: string | null) {
      // the same words (a trailing space, say): keep what's there, or on its way
      if (query === current) return
      current = query
      cancel()
      if (query === null) return o.onChange(NO_MESSAGES)
      const mine = seq
      o.onChange({ status: 'loading', query, hits: [] })
      timer = setTimeout(() => {
        timer = undefined
        o.search(query).then(
          (hits) => {
            if (mine === seq) o.onChange({ status: 'done', query, hits: hits.slice(0, limit) })
          },
          () => {
            if (mine === seq) o.onChange({ status: 'error', query, hits: [] })
          },
        )
      }, delay)
    },
    dispose() {
      cancel()
      current = null
    },
  }
}
