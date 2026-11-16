// Saved messages (#315): the list's order, and keeping it live from message events, and its search.
import { describe, expect, it } from 'vitest'
import { bookmarkEdited, bookmarkGone, bookmarksOutOfReach, searchBookmarks, sortBookmarks, upsertBookmark } from './bookmarks'
import type { MessageBookmark } from './protocol/MessageBookmark'
import type { Message } from './protocol/Message'

const b = (id: string, created_at: string, content: string | null = 'hello', note: string | null = null, channel_id = 'C1'): MessageBookmark => ({
  id, message_id: `M-${id}`, note, created_at,
  message: content === null ? null : { channel_id, thread_root_id: null, author_id: 'U2', content, created_at },
})

describe('saved messages (#315)', () => {
  it('newest saved first; a save of the same one replaces it', () => {
    const list = sortBookmarks([b('B1', '2026-11-01T00:00:00.000Z'), b('B2', '2026-11-03T00:00:00.000Z'), b('B3', '2026-11-02T00:00:00.000Z')])
    expect(list.map((x) => x.id)).toEqual(['B2', 'B3', 'B1'])
    const next = upsertBookmark(list, { ...b('B1', '2026-11-01T00:00:00.000Z'), note: 'n' })
    expect(next.map((x) => x.id)).toEqual(['B2', 'B3', 'B1'])
    expect(next[2].note).toBe('n')
  })

  it('a deleted message, or a chat out of reach, leaves nothing of the message', () => {
    const list = [b('B1', '1', 'secret'), b('B2', '2', 'other', null, 'C2')]
    expect(bookmarkGone(list, 'M-B1')[0].message).toBeNull()
    expect(bookmarkGone(list, 'M-B1')[1].message).not.toBeNull()
    expect(bookmarksOutOfReach(list, 'C2').map((x) => !!x.message)).toEqual([true, false])
  })

  it('an edit shows the new text, cut to the snippet length; a deleted update leaves nothing', () => {
    const list = [b('B1', '1', 'old')]
    const m = { id: 'M-B1', content: 'x'.repeat(400), deleted: false } as Message
    expect(bookmarkEdited(list, m)[0].message!.content).toHaveLength(300)
    expect(bookmarkEdited(list, { ...m, deleted: true })[0].message).toBeNull()
  })

  it('search matches every word in the note, text or author, any case; an unavailable one by its note only', () => {
    const list = [b('B1', '1', 'Gate code 4321', 'saturday'), b('B2', '2', null, 'recipe'), b('B3', '3', 'pizza on main')]
    expect(searchBookmarks(list, '').map((x) => x.id)).toEqual(['B1', 'B2', 'B3'])
    expect(searchBookmarks(list, 'gate SATURDAY').map((x) => x.id)).toEqual(['B1'])
    expect(searchBookmarks(list, 'recipe').map((x) => x.id)).toEqual(['B2'])
    expect(searchBookmarks(list, 'sam', (x) => (x.id === 'B3' ? 'sam' : 'kim')).map((x) => x.id)).toEqual(['B3'])
  })
})
