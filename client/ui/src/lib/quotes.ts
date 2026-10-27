// Pure helpers for the quotes feature (speakers, scores, Quote of the Year). No UI, no state changes.
import type { Message } from './protocol/Message'
import type { QotyEvent } from './protocol/QotyEvent'
import type { QotyOption } from './protocol/QotyOption'
import type { QuoteLine } from './protocol/QuoteLine'
import { displayName } from './selectors'
import type { AppState } from './state'

/** What a plain speaker name is matched on: same rule as the server (lowercase, whitespace runs as one space). */
export function nameKey(name: string): string {
  return name.split(/\s+/).filter(Boolean).join(' ').toLowerCase()
}

const MARKS = '"“”\'‘’«»'
const isMark = (c: string) => MARKS.includes(c)

/** Line text without its surrounding quote marks, as the server stores it: only when both ends carry one. */
export function stripMarks(text: string): string {
  const t = text.trim()
  if (t.length < 2 || !isMark(t[0]) || !isMark(t[t.length - 1])) return t
  let a = 0
  let b = t.length
  while (a < b && isMark(t[a])) a++
  while (b > a && isMark(t[b - 1])) b--
  return t.slice(a, b).trim()
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

/** The option with the most votes (the first of a tie); none while nobody has voted. */
export function leader(e: QotyEvent): { option: QotyOption; votes: number } | null {
  let best: QotyOption | null = null
  for (const o of e.options) if (o.votes > (best?.votes ?? 0)) best = o
  return best ? { option: best, votes: best.votes } : null
}
