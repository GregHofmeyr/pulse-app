// Pure helpers for the quotes feature (speakers, scores, Quote of the Year). No UI, no state changes.
import type { Message } from './protocol/Message'
import type { QotyEvent } from './protocol/QotyEvent'
import type { QotyOption } from './protocol/QotyOption'
import type { QuoteLine } from './protocol/QuoteLine'
import type { QuoteSpeaker } from './protocol/QuoteSpeaker'
import { displayName } from './selectors'
import type { AppState } from './state'

/** What a plain speaker name is matched on: same rule as the server (lowercase, whitespace runs as one space). */
// Unicode White_Space exactly (Rust's split_whitespace); JS `\s` differs (it includes U+FEFF, and lacks U+0085).
const WHITE_SPACE = /[\t-\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+/

export function nameKey(name: string): string {
  return name.split(WHITE_SPACE).filter(Boolean).join(' ').toLowerCase()
}

/** Quote marks that wrap a line, opening mark to closing mark (the server's `QUOTE_PAIRS`). */
const PAIRS: Record<string, string> = { '"': '"', '“': '”', '‘': '’', "'": "'", '«': '»' }
const WORD = /[\p{Alphabetic}\p{N}]/u // Rust's char::is_alphanumeric
const trimWs = (s: string) => s.replace(new RegExp(`^${WHITE_SPACE.source}|${WHITE_SPACE.source}$`, 'g'), '')

/** Line text as the server stores it: trimmed, and without the one matching pair of quote marks
 *  that wraps it, if any. A pair wraps the text only if the opening mark's first closing mark is
 *  the last character (so `"Yes" and "no"` keeps its marks); an apostrophe inside a word (`don't`)
 *  doesn't close a single quote. */
export function stripMarks(text: string): string {
  const t = trimWs(text)
  const cs = [...t]
  const close = PAIRS[cs[0]]
  if (!close) return t
  const word = (i: number) => i < cs.length && WORD.test(cs[i])
  for (let i = 1; i < cs.length; i++) {
    if (cs[i] !== close || ((close === "'" || close === '’') && word(i - 1) && word(i + 1))) continue
    return i === cs.length - 1 ? trimWs(cs.slice(1, -1).join('')) : t
  }
  return t
}

/** Who a line is credited to now: its member, or the member a plain name is linked to, else the plain name. */
export function speakerOf(s: AppState, serverId: string, line: QuoteLine): { userId: string | null; name: string } {
  if (line.speaker_id) return { userId: line.speaker_id, name: displayName(s, serverId, line.speaker_id) }
  const plain = line.speaker_name ?? ''
  const key = nameKey(plain)
  const link = (s.speakerLinks[serverId] ?? []).find((l) => l.name_key === key)
  return link ? { userId: link.user_id, name: displayName(s, serverId, link.user_id) } : { userId: null, name: plain }
}

/** A quote's reaction score: all its reactions added up. */
export function score(m: Message): number {
  return m.reactions.reduce((n, r) => n + r.count, 0)
}

/** Whether the quote's thread has a reply newer than the one you last saw. */
export function hasNewReplies(q: Message): boolean {
  const quote = q.quote
  return !!quote?.last_reply_id && (quote.replies_seen === null || quote.last_reply_id > quote.replies_seen)
}

/** The server's vote that is open and has not run out, if any. */
export function liveQoty(s: AppState, serverId: string, now: number): QotyEvent | null {
  const e = s.qoty[serverId]
  if (!e || e.ended_at !== null) return null
  return e.ends_at === null || Date.parse(e.ends_at) > now ? e : null
}

/** The year a new vote offers first: last year in January, this year otherwise. */
export function defaultQotyYear(now: Date): number {
  return now.getMonth() === 0 ? now.getFullYear() - 1 : now.getFullYear()
}

/** "2d 3h", "2h 5m", "4m", "<1m"; "until ended" for a vote without an end time. */
export function timeLeft(endsAt: string | null, nowMs: number): string {
  if (endsAt === null) return 'until ended'
  const mins = Math.floor((Date.parse(endsAt) - nowMs) / 60_000)
  if (mins < 1) return '<1m'
  const [d, h, m] = [Math.floor(mins / 1440), Math.floor((mins % 1440) / 60), mins % 60]
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`
  return `${m}m`
}

/** The option (its quote's message id) you voted for. */
export function myVote(e: QotyEvent, meId: string): string | null {
  return e.options.find((o) => o.voters.includes(meId))?.message.id ?? null
}

/** What clicking `clicked` does: vote, move your vote there, or take it back. */
export function voteAction(myVote: string | null, clicked: string): 'vote' | 'move' | 'unvote' {
  return myVote === null ? 'vote' : myVote === clicked ? 'unvote' : 'move'
}

/** Who said a quote (or several): each person once, in line order ("sam & kim"). */
export function speakersOf(s: AppState, serverId: string, m: Message | Message[]): string {
  const lines = (Array.isArray(m) ? m : [m]).flatMap((q) => q.quote?.lines ?? [])
  return [...new Set(lines.map((l) => speakerOf(s, serverId, l).name))].join(' & ')
}

/** A stored speaker (a win's snapshot): the member's name, else the plain name. */
export function snapshotName(s: AppState, serverId: string, who: QuoteSpeaker): string {
  return who.user_id ? displayName(s, serverId, who.user_id) : (who.name ?? '')
}

/** A win's snapshot speakers as one label ("sam & A Friend"); '' when there is none (an old card). */
export function snapshotSpeakers(s: AppState, serverId: string, speakers: QuoteSpeaker[]): string {
  return [...new Set(speakers.map((w) => snapshotName(s, serverId, w)))].join(' & ')
}

/** A quote in one line: its first line (an ellipsis when there are more) and that line's speaker,
 *  or, for a win, everyone who said it when the vote ended (`speakers`, the snapshot: later edits
 *  and links don't move it; empty for an old win without one). */
export function quotePreview(s: AppState, serverId: string, m: Message, speakers: QuoteSpeaker[] = []): { said: string; by: string } {
  const lines = m.quote?.lines ?? []
  const first = lines[0]
  if (!first) return { said: m.content, by: '' }
  const said = first.text ? `“${first.text}”` : (first.caption ?? 'Image')
  const by = speakers.length ? snapshotSpeakers(s, serverId, speakers) : speakerOf(s, serverId, first).name
  return { said: lines.length > 1 ? `${said} …` : said, by }
}

/** "2h 5m left", or "until ended" for a vote without an end time. */
export function leftLabel(endsAt: string | null, nowMs: number): string {
  const t = timeLeft(endsAt, nowMs)
  return endsAt === null ? t : `${t} left`
}

/** Whether the ended vote `eventId` can still be reopened (`e` is the server's latest event). */
export function canReopen(e: QotyEvent | undefined, eventId: string, nowMs: number): boolean {
  return !!e && e.id === eventId && e.ended_at !== null && e.reopen_until !== null && Date.parse(e.reopen_until) > nowMs
}

/** "1 vote", "2 votes". */
export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`
}

/** Every option tied for the most votes; none while nobody has voted. */
export function leaders(e: QotyEvent): QotyOption[] {
  const top = Math.max(0, ...e.options.map((o) => o.votes))
  return top > 0 ? e.options.filter((o) => o.votes === top) : []
}
