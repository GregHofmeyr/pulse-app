import { describe, expect, it } from 'vitest'
import { applyReady, emptyState } from './state'
import { groupLineText, memberLineText } from './groupLines'
import { previewText } from './conversations'
import { testReady } from './testReady'
import type { Message } from './protocol/Message'
import type { GroupLine } from './protocol/GroupLine'

const user = (id: string, username: string, display_name = username) => ({ id, username, display_name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const GROUP = { id: 'G1', server_id: null, kind: 'group' as const, name: 'Friends', position: 0 }
const state = () =>
  applyReady(emptyState(), testReady([GROUP], {
    me: user('U1', 'alex'),
    dm_members: [{ channel_id: 'G1', user_ids: ['U1', 'U2', 'U3'] }],
    people: [
      { user: user('U1', 'alex'), online: true, last_seen_at: null, status: 'online' },
      // renamed since the lines were written
      { user: user('U2', 'sam', 'Sammy'), online: true, last_seen_at: null, status: 'online' },
      { user: user('U3', 'kim'), online: true, last_seen_at: null, status: 'online' },
    ],
  }))
const line = (content: string, group_line?: GroupLine): Message => ({
  id: 'L1', channel_id: 'G1', author_id: null, kind: 'system', content, reply_to_id: null,
  created_at: '2026-11-05T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null,
  qoty_card: null, clip: null, embeds: [], group_line,
})

describe('group lines (#114)', () => {
  it('writes each line with the names that apply now, not as stored', () => {
    const s = state()
    expect(groupLineText(s, line('alex created the group with sam and kim', { kind: 'created', by: 'U1', users: ['U2', 'U3'] })))
      .toBe('alex created the group with Sammy and kim')
    expect(groupLineText(s, line('sam added kim', { kind: 'members_added', by: 'U2', users: ['U3'] }))).toBe('Sammy added kim')
    expect(groupLineText(s, line('sam left', { kind: 'member_left', by: 'U2', users: [] }))).toBe('Sammy left')
    expect(groupLineText(s, line('alex removed sam', { kind: 'member_removed', by: 'U1', users: ['U2'] }))).toBe('alex removed Sammy')
    expect(groupLineText(s, line('sam renamed the group to Trip', { kind: 'renamed', by: 'U2', users: [], name: 'Trip' })))
      .toBe('Sammy renamed the group to Trip')
    expect(groupLineText(s, line('sam removed the group name', { kind: 'name_removed', by: 'U2', users: [] }))).toBe('Sammy removed the group name')
    expect(groupLineText(s, line('sam changed the group icon', { kind: 'icon_changed', by: 'U2', users: [] }))).toBe('Sammy changed the group icon')
    expect(groupLineText(s, line('sam removed the group icon', { kind: 'icon_removed', by: 'U2', users: [] }))).toBe('Sammy removed the group icon')
  })
  it('applies your private nickname for someone', () => {
    const s = { ...state(), privateNicks: { U3: 'Kimmy' } }
    expect(groupLineText(s, line('sam added kim', { kind: 'members_added', by: 'U2', users: ['U3'] }))).toBe('Sammy added Kimmy')
  })
  it('keeps the stored text for an older line, or one naming someone this client does not know', () => {
    const s = state()
    expect(groupLineText(s, line('sam added kim'))).toBe('sam added kim')
    expect(groupLineText(s, line('sam added jo', { kind: 'members_added', by: 'U2', users: ['U9'] }))).toBe('sam added jo')
  })
  it('reads the same in the conversation list preview', () => {
    const s = state()
    expect(previewText(s, line('sam left', { kind: 'member_left', by: 'U2', users: [] }))).toEqual({ text: 'Sammy left', system: true })
  })
})

describe('server membership lines (#418)', () => {
  const member = (removed_by?: string): Message => ({
    ...line('sam left the server'), channel_id: 'C1',
    member_line: { user_id: 'U2', ...(removed_by ? { removed_by } : {}) },
  })

  it('names who left or was removed as they\'re called now', () => {
    const s = state()
    expect(memberLineText(s, member(), null)).toBe('Sammy left the server')
    expect(memberLineText(s, member('U1'), null)).toBe('Sammy was removed')
  })

  it('keeps the stored text for someone this client doesn\'t know', () => {
    const s = state()
    const m: Message = { ...member(), content: 'jo left the server', member_line: { user_id: 'U9' } }
    expect(memberLineText(s, m, null)).toBe('jo left the server')
  })
})
