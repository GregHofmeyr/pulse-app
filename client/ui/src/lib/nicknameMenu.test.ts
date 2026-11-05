import { describe, expect, it } from 'vitest'
import { emptyState, type AppState } from './state'
import { canServerNick, nickItems, nickLabel } from './nicknameMenu'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const sam = user('U2', 'sam')
const kim = user('U3', 'kim')
const state = (over: Partial<AppState> = {}): AppState => ({
  ...emptyState(),
  me: alex,
  members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: null }], S2: [{ user: kim, nickname: null }] },
  ...over,
}) as AppState

describe('nickItems (#137: the nickname items a person menu offers, by where it opens)', () => {
  it('in a server: their server nickname, then your private nickname for them', () => {
    expect(nickItems(state(), 'S1', 'U2')).toEqual(['nick', 'private-nick'])
    const s = state({ members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: 'Sammy' }] }, privateNicks: { U2: 'Bestie' } })
    expect(nickItems(s, 'S1', 'U2')).toEqual(['nick', 'private-nick'])
  })
  it('in a server, yourself: your server nickname, never a private one', () => {
    expect(nickItems(state(), 'S1', 'U1')).toEqual(['nick'])
  })
  it('outside servers (the DM list): your private nickname for them, never a server one; none for yourself', () => {
    expect(nickItems(state(), null, 'U2')).toEqual(['private-nick'])
    expect(nickItems(state({ privateNicks: { U2: 'Bestie' } }), null, 'U2')).toEqual(['private-nick'])
    expect(nickItems(state(), null, 'U1')).toEqual([])
  })
  it('no reset or clear items: those live in the nickname dialog', () => {
    const s = state({ members: { S1: [{ user: alex, nickname: 'Al' }, { user: sam, nickname: 'Sammy' }] }, privateNicks: { U2: 'Bestie' } })
    expect([...nickItems(s, 'S1', 'U2'), ...nickItems(s, null, 'U2')]).toEqual(['nick', 'private-nick', 'private-nick'])
  })
  it('no server nickname when you are not in the server, or they are not', () => {
    expect(canServerNick(state(), 'S2', 'U3')).toBe(false) // you aren't in S2
    expect(canServerNick(state(), 'S1', 'U3')).toBe(false) // they aren't in S1
    expect(canServerNick(state(), 'S1', 'U2')).toBe(true)
    expect(canServerNick(state(), null, 'U2')).toBe(false)
    // the private one still applies: it's yours, wherever they are
    expect(nickItems(state(), 'S1', 'U3')).toEqual(['private-nick'])
  })
  it('labels: in a server the private one says whether you have one; outside servers it is just your nickname for them', () => {
    expect(nickLabel(state(), 'S1', 'U2', 'nick')).toBe('Change server nickname')
    expect(nickLabel(state(), 'S1', 'U2', 'private-nick')).toBe('Set private nickname')
    expect(nickLabel(state({ privateNicks: { U2: 'Bestie' } }), 'S1', 'U2', 'private-nick')).toBe('Change private nickname')
    expect(nickLabel(state(), null, 'U2', 'private-nick')).toBe('Change nickname')
    expect(nickLabel(state({ privateNicks: { U2: 'Bestie' } }), null, 'U2', 'private-nick')).toBe('Change nickname')
  })
})
