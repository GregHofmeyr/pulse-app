// Forwarding (#302): where you can forward to, how the picker orders it, and how a forward names its origin.
import { describe, expect, it } from 'vitest'
import { canOpenOriginal, forwardList, forwardTargets, forwardedFrom } from './forward'
import { emptyState, type AppState } from './state'
import type { Forwarded } from './protocol/Forwarded'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const kim = { ...me, id: 'U3', username: 'kim', display_name: 'kim' }
function state(): AppState {
  const s = emptyState()
  return {
    ...s,
    me: me as never,
    servers: [{ id: 'S1', name: 'Main' } as never, { id: 'S2', name: 'Other' } as never],
    members: { S1: [{ user: me, nickname: null } as never], S2: [] },
    people: { U3: { user: kim } as never },
    channels: {
      C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      V1: { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 1 },
      Q1: { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 2 },
      K1: { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 3 },
      X1: { id: 'X1', server_id: 'S2', kind: 'text', name: 'theirs', position: 0 },
      D1: { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
      H1: { id: 'H1', server_id: null, kind: 'dm', name: null, position: 0 },
    },
    dmMembers: { D1: ['U1', 'U3'], H1: ['U1', 'U3'] },
    hidden: { H1: true },
  } as AppState
}
const fwd = (over: Partial<Forwarded> = {}): Forwarded => ({
  message_id: 'M1', channel_id: 'C1', channel_kind: 'text', channel_name: 'general', server_id: 'S1', thread_root_id: null,
  author_id: 'U3', created_at: '2026-11-16T09:00:00.000Z', content: 'hi', ...over,
})

describe('forwarding (#302)', () => {
  it('targets: your servers\' text and voice chats, your open DMs and groups; never quotes, clips or others\' servers', () => {
    const keys = forwardTargets(state()).map((p) => p.id).sort()
    expect(keys).toEqual(['C1', 'D1', 'V1'])
  })

  it('recent chats first with nothing typed; a query ranks by name', () => {
    const t = forwardTargets(state())
    expect(forwardList(t, '', ['c:V1']).map((p) => p.id)).toEqual(['V1', 'D1', 'C1'])
    expect(forwardList(t, 'gen', []).map((p) => p.id)).toEqual(['C1'])
    expect(forwardList(t, 'kim', []).map((p) => p.id)).toEqual(['D1'])
  })

  it('names a server channel with its server, never a DM or group', () => {
    const s = state()
    expect(forwardedFrom(s, fwd())).toBe('#general in Main')
    expect(forwardedFrom(s, fwd({ channel_kind: 'voice', channel_name: 'Lounge' }))).toBe('Lounge in Main')
    expect(forwardedFrom(s, fwd({ channel_kind: 'dm', channel_name: null, server_id: null }))).toBe('a DM')
    expect(forwardedFrom(s, fwd({ channel_kind: 'group', channel_name: null, server_id: null }))).toBe('a group')
  })

  it('the original opens only where you can see it, and not once it is gone', () => {
    const s = state()
    expect(canOpenOriginal(s, fwd())).toBe(true)
    expect(canOpenOriginal(s, fwd({ channel_id: 'NOPE' }))).toBe(false)
    expect(canOpenOriginal(s, fwd({ unavailable: true }))).toBe(false)
  })
})
