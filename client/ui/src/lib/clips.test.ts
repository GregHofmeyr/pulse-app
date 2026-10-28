import { describe, expect, it } from 'vitest'
import { clipFilter, feedKey, formatBytes, formatDuration, matchesFilter, clipView, isNewClip, matchGame, hasNewClipReplies, gameName } from './clips'
import type { Game } from './protocol/Game'
import { applyEvent, applyReady, emptyState } from './state'
import type { Message } from './protocol/Message'

const clip = (game: string | null, author = 'U2'): Message => ({
  id: 'M1', channel_id: 'K1', author_id: author, kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
  clip: { thumb_hash: '', duration_ms: 1000, width: 1, height: 1, size: 1, game_id: game, reply_count: 0, last_reply_id: null, replies_seen: null },
})

describe('formatDuration', () => {
  it('shows minutes and seconds', () => {
    expect(formatDuration(65_000)).toBe('1:05')
    expect(formatDuration(9_400)).toBe('0:09')
    expect(formatDuration(0)).toBe('0:00')
    expect(formatDuration(3_723_000)).toBe('1:02:03')
  })
})

describe('formatBytes', () => {
  it('uses decimal units', () => {
    expect(formatBytes(47_736_000)).toBe('47.7 MB')
    expect(formatBytes(1_200_000_000)).toBe('1.2 GB')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(2_500)).toBe('3 KB')
  })
})

describe('matchesFilter', () => {
  it('needs every set part to match', () => {
    expect(matchesFilter(clip('G1'), {})).toBe(true)
    expect(matchesFilter(clip('G1'), { game: 'G1' })).toBe(true)
    expect(matchesFilter(clip('G2'), { game: 'G1' })).toBe(false)
    expect(matchesFilter(clip(null), { game: 'G1' })).toBe(false)
    expect(matchesFilter(clip('G1', 'U3'), { game: 'G1', author: 'U2' })).toBe(false)
    expect(matchesFilter(clip('G1', 'U2'), { game: 'G1', author: 'U2' })).toBe(true)
  })
  it('keys differ per filter', () => {
    expect(feedKey('K1', {})).not.toBe(feedKey('K1', { game: 'G1' }))
    expect(feedKey('K1', { game: 'G1' })).toBe(feedKey('K1', { game: 'G1' }))
    expect(feedKey('K1', clipFilter('', null))).toBe(feedKey('K1', {}))
    expect(clipFilter('G1', undefined)).toEqual({ game: 'G1' })
  })
})

describe('clipView / isNewClip', () => {
  it('defaults to wall', () => expect(clipView(emptyState(), 'K1')).toBe('wall'))
  it('is new when after the read point', () => {
    let s = applyReady(emptyState(), { me: { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, servers: [], channels: [], members: [], dm_members: [], voice: [], people: [], read_states: [{ channel_id: 'K1', last_read_message_id: 'M5', unread: 0, mentions: 0 }], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [] })
    expect(isNewClip(s, 'K1', 'M6')).toBe(true)
    expect(isNewClip(s, 'K1', 'M5')).toBe(false)
    expect(isNewClip(s, 'K1', 'M6', 'M9')).toBe(false)
    s = applyEvent(s, { t: 'ClipViewChanged', d: { user_id: 'U1', channel_id: 'K1', view: 'list' } }, 0)
    expect(clipView(s, 'K1')).toBe('list')
  })
})

describe('matchGame', () => {
  const g = (id: string, name: string): Game => ({ id, server_id: 'S1', name, created_by: null, created_at: '2026-10-01T00:00:00.000Z' })
  const games = [g('G1', 'Overwatch 2'), g('G2', 'Valorant')]
  it('finds a game by its name key', () => {
    expect(matchGame('overwatch  2', games)).toEqual({ game: games[0] })
  })
  it('offers to add an unknown game, keeping the guess', () => {
    expect(matchGame('Deadlock', games)).toEqual({ add: 'Deadlock' })
  })
  it('uses the same whitespace rule as nameKey', () => {
    expect(matchGame('\u0085Overwatch\u00a02\u0085', games)).toEqual({ game: games[0] })
    expect(matchGame('\uFEFFDeadlock', games)).toEqual({ add: '\uFEFFDeadlock' })
  })
  it('caps the added name at 40 characters', () => {
    const r = matchGame('x'.repeat(30) + ' ' + '🎮'.repeat(20), games) as { add: string }
    expect([...r.add]).toHaveLength(40)
    expect(r.add.endsWith('🎮')).toBe(true)
    expect(matchGame('y'.repeat(39) + ' zzz', games)).toEqual({ add: 'y'.repeat(39) })
  })
  it('is null without a guess', () => {
    expect(matchGame(null, games)).toBeNull()
    expect(matchGame('   ', games)).toBeNull()
  })
})

describe('hasNewClipReplies / gameName', () => {
  const withThread = (last: string | null, seen: string | null): Message => ({ ...clip(null), clip: { ...clip(null).clip!, last_reply_id: last, replies_seen: seen } })
  it('is new when the newest reply is past the one you saw', () => {
    expect(hasNewClipReplies(withThread(null, null))).toBe(false)
    expect(hasNewClipReplies(withThread('R2', null))).toBe(true)
    expect(hasNewClipReplies(withThread('R2', 'R1'))).toBe(true)
    expect(hasNewClipReplies(withThread('R2', 'R2'))).toBe(false)
  })
  it('names the game from the server registry', () => {
    const g: Game = { id: 'G1', server_id: 'S1', name: 'Apex', created_by: null, created_at: '2026-10-01T00:00:00Z' }
    const s = { ...emptyState(), games: { S1: [g] } }
    expect(gameName(s, 'S1', clip('G1'))).toBe('Apex')
    expect(gameName(s, 'S1', clip('G9'))).toBeNull()
    expect(gameName(s, 'S1', clip(null))).toBeNull()
  })
})
