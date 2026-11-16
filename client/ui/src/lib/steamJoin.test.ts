// One-click Join (#381): the client's own strict check of what the server says can be joined, and the URL from it.
import { describe, expect, it } from 'vitest'
import { joinOf, joinUrl, validLobby, validServer } from './steamJoin'
import { emptyState } from './state'
import type { SteamJoin } from './protocol/SteamJoin'

// obviously fake: lobby account 1, the fake SteamIDs the server tests use, and a documentation address
const LOBBY = '109775240917155841'
const ALEX = '76500000000000001'
const SERVER = '203.0.113.7:27015'

describe('steam join', () => {
  it('lobby ids', () => {
    expect(validLobby(LOBBY)).toBe(true)
    for (const bad of ['', '0109775240917155841', '+109775240917155841', '109775240917155841 ', '1097752409171558o1',
      '76561197960265729', '108086391056891905', '109775240917155840', '99999999999999999999', '18446744073709551616']) {
      expect([bad, validLobby(bad)]).toEqual([bad, false])
    }
  })

  it('server addresses', () => {
    expect(validServer(SERVER)).toBe(true)
    expect(validServer('192.168.1.20:27016')).toBe(true)
    for (const bad of ['', '203.0.113.7', '203.0.113.7:', '203.0.113.7:0', '203.0.113.7:65536', '203.0.113.7:027015',
      '203.0.113.007:27015', '203.0.113:27015', '256.0.113.7:27015', '0.0.0.0:27015', '127.0.0.1:27015', '169.254.1.1:27015',
      '255.255.255.255:27015', '224.0.0.1:27015', '[2001:db8::1]:27015', 'example.com:27015', '203.0.113.7:27015/x',
      '203.0.113.7:27015 ', ' 203.0.113.7:27015']) {
      expect([bad, validServer(bad)]).toEqual([bad, false])
    }
  })

  it('urls only from parts that check out', () => {
    const lobby = (app_id: number, lobby_id: string, steam_id: string): SteamJoin => ({ kind: 'lobby', app_id, lobby_id, steam_id })
    const server = (app_id: number, address: string): SteamJoin => ({ kind: 'server', app_id, address })
    expect(joinUrl(lobby(440, LOBBY, ALEX))).toBe(`steam://joinlobby/440/${LOBBY}/${ALEX}`)
    expect(joinUrl(server(440, SERVER))).toBe(`steam://connect/${SERVER}`)
    expect(joinUrl(lobby(0, LOBBY, ALEX))).toBeNull()
    expect(joinUrl(lobby(1.5, LOBBY, ALEX))).toBeNull()
    expect(joinUrl(lobby(440, '1/../../x', ALEX))).toBeNull()
    expect(joinUrl(lobby(440, LOBBY, '7650000000000000a'))).toBeNull()
    expect(joinUrl(server(440, `${SERVER}/+quit`))).toBeNull()
    expect(joinUrl({ kind: 'other', app_id: 440 } as never)).toBeNull()
    expect(joinUrl(null)).toBeNull()
  })

  it("someone else's that checks out, never your own", () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const playing = (join?: SteamJoin) => ({ name: 'Space Game', steam_app_id: 440, started_at: null, art: null, join })
    const s = (join?: SteamJoin) => ({
      ...emptyState(), me,
      people: { U1: { user: me, online: true, last_seen_at: null, playing: playing(join) }, U2: { user: { ...me, id: 'U2' }, online: true, last_seen_at: null, playing: playing(join) } },
    }) as never
    const j: SteamJoin = { kind: 'server', app_id: 440, address: SERVER }
    expect(joinOf(s(j), 'U2')).toEqual(j)
    expect(joinOf(s(j), 'U1')).toBeNull()
    expect(joinOf(s(), 'U2')).toBeNull()
    expect(joinOf(s({ ...j, address: '127.0.0.1:1' }), 'U2')).toBeNull()
    expect(joinOf(s(j), 'U9')).toBeNull()
  })
})
