// Blocking a person (#474), the pure part: what you see of someone you've blocked. The server keeps their DMs, calls,
// invites and pings from you and shows you offline and emptied to them; what's left is how they show in servers you
// share, per block:
//   - 'line': a run of their messages collapses into one "N blocked messages · Show" row; their voice row reads
//     "Blocked person · can't hear"; their reactions count without their name; a reply to them reads "Replying to a
//     blocked message".
//   - 'hide': nothing of them: no messages, member or voice row, reactions or typing, and counts leave them out; a
//     reply to them reads "Replying to a message you can't see".
// Either way you can't hear them in voice, and nothing of theirs alerts you (sounds, pings, soundboard).
import type { Block } from './protocol/Block'
import type { BlockMode } from './protocol/BlockMode'
import type { Message } from './protocol/Message'
import type { ReactionSummary } from './protocol/ReactionSummary'

type WithBlocks = { blocks: readonly Block[] }

/** How someone you've blocked shows, or null when you haven't blocked them. */
export function blockOf(s: WithBlocks, userId: string | null | undefined): BlockMode | null {
  if (!userId) return null
  return s.blocks.find((b) => b.user_id === userId)?.mode ?? null
}

export const isBlocked = (s: WithBlocks, userId: string | null | undefined): boolean => blockOf(s, userId) !== null

/** Someone blocked with Hide completely: nothing of them shows. */
export const isHidden = (s: WithBlocks, userId: string | null | undefined): boolean => blockOf(s, userId) === 'hide'

/** A row of the message list: a message, or a run of messages from people you've blocked ('line') collapsed into one. */
export type ListRow =
  | { kind: 'message'; message: Message; index: number }
  | { kind: 'blocked'; key: string; messages: Message[] }

/** `list` as rows to draw: messages from people blocked with Hide completely are left out; consecutive ones from
 *  people blocked with a line become one 'blocked' row (keyed by its first message), unless `shown` has that key (its
 *  Show was pressed: they're drawn as messages). `index` is the message's place in `list`, for grouping. */
export function listRows(s: WithBlocks, list: readonly Message[], shown: ReadonlySet<string> = new Set()): ListRow[] {
  if (!s.blocks.length) return list.map((message, index) => ({ kind: 'message', message, index }))
  const rows: ListRow[] = []
  let run: Message[] = []
  const flush = () => {
    if (!run.length) return
    const key = run[0].id
    if (shown.has(key)) for (const m of run) rows.push({ kind: 'message', message: m, index: list.indexOf(m) })
    else rows.push({ kind: 'blocked', key, messages: run })
    run = []
  }
  list.forEach((m, index) => {
    const mode = m.kind === 'normal' ? blockOf(s, m.author_id) : null
    if (mode === 'hide') return
    if (mode === 'line') {
      run.push(m)
      return
    }
    flush()
    rows.push({ kind: 'message', message: m, index })
  })
  flush()
  return rows
}

/** "1 blocked message" / "3 blocked messages". */
export const blockedCountText = (n: number): string => `${n} blocked message${n === 1 ? '' : 's'}`

/** What a reply to someone you've blocked says instead of their words, or null when it isn't one. */
export function blockedReplyText(s: WithBlocks, repliedToAuthor: string | null | undefined): string | null {
  const mode = blockOf(s, repliedToAuthor)
  if (mode === 'line') return 'Replying to a blocked message'
  if (mode === 'hide') return "Replying to a message you can't see"
  return null
}

/** A reaction's count as you see it (#508): Hide completely leaves out the people you hide. 0: its chip doesn't show.
 *  An older server doesn't say who reacted: its count as it is. */
export function shownCount(s: WithBlocks, r: ReactionSummary): number {
  if (!r.users || !s.blocks.some((b) => b.mode === 'hide')) return r.count
  return Math.max(0, r.count - r.users.filter((id) => isHidden(s, id)).length)
}
