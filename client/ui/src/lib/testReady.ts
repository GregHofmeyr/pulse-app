// A Ready snapshot for tests: one server you're a member of, nothing else. Not used by the app.
import type { Channel } from './protocol/Channel'
import type { QotyEvent } from './protocol/QotyEvent'
import type { Ready } from './protocol/Ready'

export const testMe = { id: 'U1', username: 'alex', avatar_hash: null }

/** `channels` all belong to server S1 unless they say otherwise; `over` replaces any other field. */
export function testReady(channels: Channel[], over: Partial<Ready> = {}): Ready {
  return {
    me: testMe, servers: [{ id: 'S1', name: 'Main', icon_hash: null }], channels,
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
    dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
    ...over,
  }
}

/** A Quote of the Year vote in S1, open until ended and with no options; `over` replaces any field. */
export function testQoty(over: Partial<QotyEvent> = {}): QotyEvent {
  return {
    id: 'E1', server_id: 'S1', year: 2026, started_by: 'U1', started_at: '2026-10-27T10:00:00Z', ends_at: null, ended_at: null,
    ended_by: null, end_reason: null, reopen_until: null, options: [], voters: 0, members: 1, ...over,
  }
}
