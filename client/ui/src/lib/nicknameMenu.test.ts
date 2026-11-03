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

describe('nickItems (#113: the profile card rules, in a menu)', () => {
  it('someone else in a server you share: change nickname and set a private one', () => {
    expect(nickItems(state(), 'S1', 'U2')).toEqual(['nick', 'private-nick'])
  })
  it('reset only when they have a server nickname; clear only when you gave a private one', () => {
    const s = state({ members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: 'Sammy' }] }, privateNicks: { U2: 'Bestie' } })
    expect(nickItems(s, 'S1', 'U2')).toEqual(['nick', 'nick-reset', 'private-nick', 'private-nick-clear'])
  })
  it('yourself: your server nickname only, never a private one', () => {
    expect(nickItems(state(), 'S1', 'U1')).toEqual(['nick'])
    const s = state({ members: { S1: [{ user: alex, nickname: 'Al' }, { user: sam, nickname: null }] } })
    expect(nickItems(s, 'S1', 'U1')).toEqual(['nick', 'nick-reset'])
  })
  it('no server (a DM or group call): private nickname only', () => {
    expect(nickItems(state(), null, 'U2')).toEqual(['private-nick'])
    expect(nickItems(state(), null, 'U1')).toEqual([])
  })
  it('no server nickname when you are not in the server, or they are not', () => {
    expect(canServerNick(state(), 'S2', 'U3')).toBe(false) // you aren't in S2
    expect(canServerNick(state(), 'S1', 'U3')).toBe(false) // they aren't in S1
    expect(canServerNick(state(), 'S1', 'U2')).toBe(true)
    expect(canServerNick(state(), null, 'U2')).toBe(false)
    expect(nickItems(state(), 'S1', 'U3')).toEqual(['private-nick'])
  })
  it('labels follow the card', () => {
    expect(nickLabel('nick', false)).toBe('Change nickname')
    expect(nickLabel('nick-reset', false)).toBe('Reset nickname')
    expect(nickLabel('private-nick', false)).toBe('Set private nickname…')
    expect(nickLabel('private-nick', true)).toBe('Change private nickname…')
    expect(nickLabel('private-nick-clear', true)).toBe('Clear private nickname')
  })
})
