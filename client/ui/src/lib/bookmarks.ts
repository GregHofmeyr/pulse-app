// Saved messages (#315), the pure parts: the list's order, keeping it live from message events, and its search.
import type { MessageBookmark } from './protocol/MessageBookmark'
import type { Message } from './protocol/Message'
import { bookmark_limits } from './bookmarkLimits'

/** Newest saved first (the server's order). */
export function sortBookmarks(list: readonly MessageBookmark[]): MessageBookmark[] {
  return [...list].sort((a, b) => (a.created_at === b.created_at ? (a.id < b.id ? 1 : -1) : a.created_at < b.created_at ? 1 : -1))
}

export function upsertBookmark(list: readonly MessageBookmark[], b: MessageBookmark): MessageBookmark[] {
  return sortBookmarks([...list.filter((x) => x.id !== b.id), b])
}

/** A message deleted: its saved one shows as unavailable (nothing of it is kept here either). */
export function bookmarkGone(list: readonly MessageBookmark[], messageId: string): MessageBookmark[] {
  return list.some((b) => b.message_id === messageId && b.message) ? list.map((b) => (b.message_id === messageId ? { ...b, message: null } : b)) : [...list]
}

/** A chat you can no longer see (left, removed, deleted): its saved messages show as unavailable. */
export function bookmarksOutOfReach(list: readonly MessageBookmark[], channelId: string): MessageBookmark[] {
  return list.map((b) => (b.message?.channel_id === channelId ? { ...b, message: null } : b))
}

/** A message edited: its saved one shows the new text. */
export function bookmarkEdited(list: readonly MessageBookmark[], m: Message): MessageBookmark[] {
  return list.map((b) =>
    b.message_id === m.id && b.message
      ? m.deleted ? { ...b, message: null } : { ...b, message: { ...b.message, content: [...m.content].slice(0, bookmark_limits.SNIPPET_CHARS).join('') } }
      : b,
  )
}

/** Those whose note or text has every word of `q` (any case); everything for an empty search. Unavailable ones match
 *  by their note only. */
export function searchBookmarks(list: readonly MessageBookmark[], q: string, author: (b: MessageBookmark) => string = () => ''): MessageBookmark[] {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return [...list]
  return list.filter((b) => {
    const hay = `${b.note ?? ''}\n${b.message?.content ?? ''}\n${b.message ? author(b) : ''}`.toLowerCase()
    return words.every((w) => hay.includes(w))
  })
}
