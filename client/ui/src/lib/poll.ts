// Polls' pure parts (#157): `/poll`'s arguments (the command itself is in lib/commands.ts), a post's checks,
// voting choices and how a poll reads.
import { timeLeft } from './pins'
import type { Message } from './protocol/Message'
import type { Poll } from './protocol/Poll'
import type { PollDuration } from './protocol/PollDuration'

/** As the server's `poll_limits` (protocol/src/rest.rs). */
export const POLL_LIMITS = { questionChars: 200, optionChars: 80, minOptions: 2, maxOptions: 10 } as const

/** The composer's Ends in choices, in order (1 day is the default). */
export const POLL_ENDS: readonly { value: PollDuration; label: string }[] = [
  { value: 'hour', label: '1 hour' },
  { value: 'day', label: '1 day' },
  { value: 'week', label: '1 week' },
  { value: 'never', label: 'No end' },
]

/** What follows `/poll` (`Question? a, b, c`) → the question (up to and with the first `?`) and the options after
 *  it (comma-separated, blanks dropped). Without a `?` the whole text is the question. It only fills the poll
 *  composer: nothing posts until Post poll. */
export function parsePollArgs(args: string): { question: string; options: string[] } {
  const rest = args.trim()
  const q = rest.indexOf('?')
  if (q < 0) return { question: rest, options: [] }
  const options = rest
    .slice(q + 1)
    .split(',')
    .map((o) => o.trim())
    .filter(Boolean)
  return { question: rest.slice(0, q + 1).trim(), options }
}

const chars = (s: string) => [...s].length

/** Post poll's gate: a question and at least two filled options, none blank, none the same (any case), all within
 *  the limits. */
export function pollReady(question: string, options: string[]): boolean {
  const q = question.trim()
  if (!q || chars(q) > POLL_LIMITS.questionChars) return false
  const opts = options.map((o) => o.trim())
  if (opts.length < POLL_LIMITS.minOptions || opts.length > POLL_LIMITS.maxOptions) return false
  if (opts.some((o) => !o || chars(o) > POLL_LIMITS.optionChars)) return false
  return new Set(opts.map((o) => o.toLowerCase())).size === opts.length
}

/** Your choices after a click on option `i`: single choice moves to it (or, on yours, takes it back); multiple
 *  choice toggles it. */
export function nextChoices(mine: number[], i: number, multi: boolean): number[] {
  if (!multi) return mine.includes(i) ? [] : [i]
  return mine.includes(i) ? mine.filter((x) => x !== i) : [...mine, i].sort((a, b) => a - b)
}

/** Whole-number percent of the people who voted (a multiple-choice poll's can add up past 100). */
export const percent = (votes: number, voters: number) => (voters ? Math.round((votes / voters) * 100) : 0)

/** The options with the most votes (all of a tie); none before anyone voted or while results are hidden. */
export function winners(p: Poll): number[] {
  const r = p.results
  if (!r) return []
  const top = Math.max(0, ...r.map((t) => t.votes))
  return top ? r.flatMap((t, i) => (t.votes === top ? [i] : [])) : []
}

/** Over: closed, or its time is up (before the server's sweep has closed it). */
export const pollOver = (p: Poll, now: number) => p.closed || (!!p.ends_at && Date.parse(p.ends_at) <= now)

/** "One choice · Anonymous · 3 hours left" / "… · No end" / "… · Closed". */
export function pollStatus(p: Poll, now: number): string {
  const parts = [p.multi ? 'Multiple choice' : 'One choice']
  if (p.anonymous) parts.push('Anonymous')
  parts.push(pollOver(p, now) ? 'Closed' : p.ends_at ? timeLeft(p.ends_at, now) : 'No end')
  return parts.join(' · ')
}

const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`

/** The total under the options: people who voted ("4 votes"; "4 voters" on multiple choice, where a person may
 *  have several votes). */
export const totalLabel = (p: Poll) => plural(p.voters, p.multi ? 'voter' : 'vote')

/** A message's text for one-line previews (the conversation list, notifications, pins, reply snippets): a poll's
 *  question, not the fuller text its `content` carries for clients that don't know polls. */
export const shownText = (m: Message): string => (m.poll ? `📊 ${m.poll.question}` : m.content)

/** "a", "a and b", "a, b and c". */
const joinNames = (names: string[]) => (names.length < 2 ? (names[0] ?? '') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`)

/** What follows "alex's poll ended" on its line: ": Valheim won (4 votes)", ": a and b tied (2 votes each)", or
 *  " with no votes" (as the server words the line's text). */
export function endedSummary(winners: string[], votes: number): string {
  if (!winners.length) return ' with no votes'
  if (winners.length === 1) return `: ${winners[0]} won (${plural(votes, 'vote')})`
  return `: ${joinNames(winners)} tied (${plural(votes, 'vote')} each)`
}
