// "Add to quotes…" (#316): a message's menu turns it into a quote in one of its server's quotes channels. The speaker
// is its author (by name when they've left the server), the text its content; it goes through the quote composer's
// own route (POST /channels/{id}/quotes).
import type { AppState } from './state'
import type { Channel } from './protocol/Channel'
import type { Message } from './protocol/Message'
import type { PostQuoteRequest } from './protocol/PostQuoteRequest'
import { displayName } from './selectors'

/** A quote speaker's plain name is at most this long (the server's MAX_NAME). */
const MAX_NAME = 32

/** Whether a message has words to quote: someone's typed text, not a poll, GIF, clip or call/system line. */
export function quotable(m: Message): boolean {
  return m.kind === 'normal' && !m.deleted && !!m.author_id && !m.poll && !m.gif && !m.clip && !m.quote && !m.caption_is_fallback && !!m.content.trim()
}

/** The quotes channels `m` could be added to: its server's, in the channel list's order. None in a DM or group (no
 *  server), and none for a message with nothing to quote. */
export function quotesChannelsFor(s: AppState, m: Message): Channel[] {
  const server = s.channels[m.channel_id]?.server_id
  if (!server || !quotable(m)) return []
  return Object.values(s.channels)
    .filter((c) => c.server_id === server && c.kind === 'quotes')
    .sort((a, b) => a.position - b.position || a.id.localeCompare(b.id))
}

/** The local date (`YYYY-MM-DD`) a quote counts for, as the quote composer sends it. */
export function localDay(d = new Date()): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
}

/** The quote request for `m` in `serverId`: one line, its author as the speaker (a member of the server), or by the
 *  name you know them by when they've left it. */
export function quoteRequest(s: AppState, m: Message, serverId: string, nonce: string, today = localDay()): PostQuoteRequest {
  const author = m.author_id!
  const member = (s.members[serverId] ?? []).some((x) => x.user.id === author)
  const name = member ? null : [...displayName(s, serverId, author).trim()].slice(0, MAX_NAME).join('') || 'someone'
  return {
    lines: [{ id: null, speaker_id: member ? author : null, speaker_name: name, text: m.content.trim(), media_hash: null, caption: null }],
    today,
    nonce,
  }
}
