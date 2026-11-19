// Blocking a person (#474), the pure part: how someone you've blocked shows in the message list, replies, reactions
// and voice rows.
import { describe, expect, it } from 'vitest'
import { blockedCountText, blockedLabel, blockedReplyText, blockOf, isBlocked, isHidden, listRows } from './blocks'
import type { Message } from './protocol/Message'

const msg = (id: string, author: string | null, kind: Message['kind'] = 'normal'): Message =>
  ({ id, channel_id: 'C1', author_id: author, kind, content: id, reply_to_id: null, created_at: '2026-11-19T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [] }) as unknown as Message

const s = { blocks: [{ user_id: 'SAM', mode: 'line' as const }, { user_id: 'KIM', mode: 'hide' as const }] }

describe('blocks', () => {
  it('knows who is blocked and how', () => {
    expect(blockOf(s, 'SAM')).toBe('line')
    expect(blockOf(s, 'KIM')).toBe('hide')
    expect(blockOf(s, 'JO')).toBeNull()
    expect(blockOf(s, null)).toBeNull()
    expect(isBlocked(s, 'SAM') && isBlocked(s, 'KIM') && !isBlocked(s, 'JO')).toBe(true)
    expect(isHidden(s, 'KIM') && !isHidden(s, 'SAM')).toBe(true)
  })

  it('collapses a run of line-blocked messages into one row and leaves hidden ones out', () => {
    const list = [msg('1', 'JO'), msg('2', 'SAM'), msg('3', 'KIM'), msg('4', 'SAM'), msg('5', 'JO'), msg('6', 'SAM')]
    const rows = listRows(s, list)
    expect(rows.map((r) => (r.kind === 'blocked' ? `blocked:${r.messages.map((m) => m.id).join('+')}` : r.message.id))).toEqual(['1', 'blocked:2+4', '5', 'blocked:6'])
    // each message keeps its place in the list, for grouping
    expect(rows.flatMap((r) => (r.kind === 'message' ? [r.index] : []))).toEqual([0, 4])
  })

  it('Show draws a run as its messages', () => {
    const list = [msg('1', 'SAM'), msg('2', 'SAM'), msg('3', 'JO')]
    const rows = listRows(s, list, new Set(['1']))
    expect(rows.map((r) => (r.kind === 'message' ? r.message.id : 'run'))).toEqual(['1', '2', '3'])
  })

  it("system lines are never someone's, and nobody blocked changes nothing", () => {
    const list = [msg('1', null, 'system'), msg('2', 'JO')]
    expect(listRows(s, list).every((r) => r.kind === 'message')).toBe(true)
    const one = msg('1', 'SAM')
    expect(listRows({ blocks: [] }, [one])).toEqual([{ kind: 'message', message: one, index: 0 }])
  })

  it('says what a reply to them, a run of theirs and their name read as', () => {
    expect(blockedReplyText(s, 'SAM')).toBe('Replying to a blocked message')
    expect(blockedReplyText(s, 'KIM')).toBe("Replying to a message you can't see")
    expect(blockedReplyText(s, 'JO')).toBeNull()
    expect(blockedCountText(1)).toBe('1 blocked message')
    expect(blockedCountText(3)).toBe('3 blocked messages')
    expect(blockedLabel(s, 'SAM')).toBe('Blocked person')
    expect(blockedLabel(s, 'KIM')).toBe('')
    expect(blockedLabel(s, 'JO')).toBeNull()
  })
})
