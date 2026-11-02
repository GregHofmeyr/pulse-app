// The search box's little language: plain words (a "quoted phrase" stays together) plus filter words
// (`from:sam`, `in:#general`, `has:clip`, `mentions:sam`). Parsed here only; the server gets the result
// as structured parameters (SearchRequest).
import type { SearchHas } from './protocol/SearchHas'
import type { SearchRequest } from './protocol/SearchRequest'

export type FilterKey = 'from' | 'mentions' | 'has' | 'in'

/** A completed filter, shown as a chip. `label` is what the chip reads (a name, a channel, a kind). */
export type Chip = { key: 'from' | 'mentions' | 'in'; id: string; label: string } | { key: 'has'; value: SearchHas; label: string }

/** The filter rows of the suggestion list, in the order shown. */
export const FILTERS: { key: FilterKey; syntax: string; hint: string }[] = [
  { key: 'from', syntax: 'from: user', hint: 'From a specific user' },
  { key: 'mentions', syntax: 'mentions: user', hint: 'Mentions a specific user' },
  { key: 'has', syntax: 'has: link, clip or quote', hint: 'Includes a link, a clip or a quote' },
  { key: 'in', syntax: 'in: channel', hint: 'Sent in a specific channel' },
]

/** What `has:` can be, as offered in the list. */
export const HAS_VALUES: { value: SearchHas; hint: string }[] = [
  { value: 'link', hint: 'A web link' },
  { value: 'clip', hint: 'A video clip' },
  { value: 'quote', hint: 'A quote' },
]

const KEYS = new Set<string>(FILTERS.map((f) => f.key))
const FILTER_RE = /^(from|mentions|has|in):(.+)$/i

/** The filter rows matching the start of a new term (`typed`); `in:` only in a server. */
export function filterRows(typed: string, inServer: boolean) {
  const t = typed.toLowerCase()
  return FILTERS.filter((f) => (inServer || f.key !== 'in') && f.key.startsWith(t))
}

/** Split into terms, keeping a "quoted phrase" (quotes included) as one. */
function split(text: string): string[] {
  return text.match(/"[^"]*"?|[^\s"]+/g) ?? []
}

/** Words and filters out of typed text. A filter with no value, an unknown `foo:bar` and anything in
 *  quotes are words. `in:` takes an optional leading `#`. */
export function parseText(text: string): { words: string; filters: { key: FilterKey; value: string }[] } {
  const words: string[] = []
  const filters: { key: FilterKey; value: string }[] = []
  for (const term of split(text)) {
    const m = term.startsWith('"') ? null : FILTER_RE.exec(term)
    const key = m?.[1].toLowerCase()
    const value = m ? bare(key as FilterKey, m[2]) : undefined
    if (m && key && KEYS.has(key) && value) filters.push({ key: key as FilterKey, value })
    else words.push(term)
  }
  return { words: words.join(' '), filters }
}

/** The term the caret is in: from just after the space before it, up to the caret. `filter` is set
 *  while a filter's value is being typed (`from:sa` → value `sa`). */
export function activeTerm(text: string, caret: number): { start: number; filter: FilterKey | null; value: string; token: string } {
  const before = text.slice(0, caret)
  const start = Math.max(before.lastIndexOf(' '), before.lastIndexOf('\t'), before.lastIndexOf('\n')) + 1
  const token = before.slice(start)
  const m = /^(from|mentions|has|in):(.*)$/i.exec(token)
  if (!m) return { start, filter: null, value: '', token }
  const filter = m[1].toLowerCase() as FilterKey
  return { start, filter, value: bare(filter, m[2]), token }
}

/** A filter's value without the mark people type out of habit: `#` before a channel, `@` before a person. */
function bare(key: FilterKey, value: string): string {
  return key === 'in' ? value.replace(/^#/, '') : key === 'has' ? value : value.replace(/^@/, '')
}

/** Whether the words can find anything: the index holds letters and digits only, so emoji or
 *  punctuation alone match nothing. */
export function hasSearchableWords(words: string): boolean {
  return /[\p{L}\p{N}]/u.test(words)
}

/** Replace text[start, end) with `insert`; the caret goes after it. Dropping a term (`insert` '')
 *  also drops the space after it, so no double spaces pile up. */
export function replaceTerm(text: string, start: number, end: number, insert: string): { text: string; caret: number } {
  let rest = text.slice(end)
  if (!insert) rest = rest.replace(/^\s+/, '')
  return { text: text.slice(0, start) + insert + rest, caret: start + insert.length }
}

/** A chip as filter syntax, as it reads in the box. */
export function chipText(c: Chip): string {
  return c.key === 'in' ? `in: #${c.label}` : `${c.key}: ${c.label}`
}

/** The search to send: the words, and the chips as parameters. */
export function toRequest(words: string, chips: Chip[], cursor: string | null = null): SearchRequest {
  const one = (key: 'from' | 'mentions' | 'in') => {
    const c = chips.findLast((x) => x.key === key)
    return c && c.key !== 'has' ? c.id : null
  }
  const has: SearchHas[] = []
  for (const c of chips) if (c.key === 'has' && !has.includes(c.value)) has.push(c.value)
  return { q: words.trim(), from: one('from'), in_channel: one('in'), has, mentions: one('mentions'), cursor }
}
