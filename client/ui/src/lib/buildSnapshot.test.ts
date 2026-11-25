import { beforeEach, describe, expect, it } from 'vitest'
import { snapshotOf } from './buildSnapshot'
import { applyReady, emptyState, type AppState } from './state'
import { testMe, testReady } from './testReady'
import type { Message } from './protocol/Message'
import type { Attachment } from './protocol/Attachment'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const shot = (n: number, expired = false): Attachment => ({
  id: `A${n}`, name: `shot${n}.webp`, content_type: 'image/webp', kind: 'image', size: 100, width: 1600, height: 900, duration_ms: null, expires_at: null, expired,
})
const build = (m: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  attachments: [shot(1), { ...shot(2), kind: 'file' }, shot(3, true), shot(4)],
  build: {
    title: 'Bunny crit', title_auto: false, game_id: 'G1', game_guessed: true, notes: 'Max crit', quick_drop: false,
    link: { url: 'https://example.com/planner', title: 'Planner', thumb: null, provider: 'generic', provider_id: null },
    updated_at: '2026-10-02T10:00:00.000Z', archived_at: '2026-10-03T10:00:00.000Z', archived_by: 'U2', reply_count: 4, last_reply_id: null, replies_seen: null,
  },
  ...m,
})

let s: AppState
beforeEach(() => {
  s = applyReady(emptyState(), testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  }))
})

describe('snapshotOf', () => {
  it('is what the pop-out shows: title, game, author, times, notes, link, screenshots and badges', () => {
    expect(snapshotOf(build(), s)).toEqual({
      message_id: 'M1', title: 'Bunny crit', game_name: 'The First Descendant', game_guessed: true, author_name: 'sam',
      saved_at: '2026-10-01T10:00:00.000Z', updated_at: '2026-10-02T10:00:00.000Z', notes: 'Max crit',
      link: { url: 'https://example.com/planner', title: 'Planner', thumb: null, provider: 'generic', provider_id: null },
      screenshots: [{ id: 'A1', width: 1600, height: 900 }, { id: 'A4', width: 1600, height: 900 }],
      archived: true, quick_drop: false,
    })
  })

  it('carries no comments, reactions or anything of the channel', () => {
    const snap = snapshotOf(build(), s) as unknown as Record<string, unknown>
    for (const k of ['reply_count', 'reactions', 'channel_id', 'content', 'mentions']) expect(snap).not.toHaveProperty(k)
  })
})
