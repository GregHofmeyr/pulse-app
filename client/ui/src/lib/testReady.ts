// A Ready snapshot for tests: one server you're a member of, nothing else. Not used by the app.
import type { Channel } from './protocol/Channel'
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
