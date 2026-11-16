// Saved messages (#315): your list (from Ready, then BookmarkSaved / BookmarkRemoved, so every device agrees), and what
// the message menu and the Saved view do with it. A message deleted (or a chat you can no longer see) turns its saved
// one unavailable here at once; the server says the same on the next Ready.
import { bookmarkEdited, bookmarkGone, bookmarksOutOfReach, sortBookmarks, upsertBookmark } from './bookmarks'
import type { Event } from './protocol/Event'
import type { MessageBookmark } from './protocol/MessageBookmark'
import { displayName } from './selectors'
import { app, onAppEvent, onAppReady } from './store.svelte'
import { api } from './tauri'

export const bookmarks = $state({ list: [] as MessageBookmark[] })

/** Your saved one of a message, if any. */
export const bookmarkOf = (messageId: string) => bookmarks.list.find((b) => b.message_id === messageId) ?? null

function put(b: MessageBookmark) {
  bookmarks.list = upsertBookmark(bookmarks.list, b)
}

/** Save a message (the one you already have when it was saved before). */
export async function saveMessage(messageId: string, note: string | null = null): Promise<MessageBookmark> {
  const b = await api.saveMessage(messageId, note)
  put(b)
  return b
}

export async function editBookmarkNote(id: string, note: string): Promise<void> {
  put(await api.editBookmark(id, note))
}

export async function removeBookmark(id: string): Promise<void> {
  await api.deleteBookmark(id)
  bookmarks.list = bookmarks.list.filter((b) => b.id !== id)
}

/** Who a saved message is from, by the name that applies where it was said. */
export function bookmarkAuthor(b: MessageBookmark): string {
  const id = b.message?.author_id
  if (!id) return b.message ? 'Pulse' : 'Unavailable'
  return displayName(app.state, app.state.channels[b.message!.channel_id]?.server_id ?? null, id)
}

/** A live event: yours only for the bookmark events; message events keep each saved one's text (or its absence). */
export function noteBookmarkEvent(e: Event): void {
  switch (e.t) {
    case 'BookmarkSaved':
      return put(e.d.bookmark)
    case 'BookmarkRemoved':
      bookmarks.list = bookmarks.list.filter((b) => b.id !== e.d.id)
      return
    case 'MessageDeleted':
      if (bookmarks.list.some((b) => b.message_id === e.d.message_id)) bookmarks.list = bookmarkGone(bookmarks.list, e.d.message_id)
      return
    case 'MessageUpdated':
      if (bookmarks.list.some((b) => b.message_id === e.d.message.id)) bookmarks.list = bookmarkEdited(bookmarks.list, e.d.message)
      return
    case 'ChannelRemoved':
      if (e.d.user_id === app.state.me?.id && bookmarks.list.some((b) => b.message?.channel_id === e.d.channel_id)) {
        bookmarks.list = bookmarksOutOfReach(bookmarks.list, e.d.channel_id)
      }
      return
  }
}

onAppReady((r) => (bookmarks.list = sortBookmarks(r.bookmarks ?? [])))
onAppEvent(noteBookmarkEvent)
