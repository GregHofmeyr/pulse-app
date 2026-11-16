// One-click Join (#381): a friend's public Steam lobby or game server, which the server found and checked and put in
// their "Playing". Checked again here, as strictly, before Join shows; the core builds and opens the steam:// URL from
// the same parts after checking them a third time. Pure.
import type { AppState } from './state'
import type { SteamJoin } from './protocol/SteamJoin'
import { playingOf } from './game'

const digits = (s: string) => /^[0-9]+$/.test(s)

/** A Steam app id: 1 to 2^31 - 1. */
const validApp = (id: number) => Number.isInteger(id) && id >= 1 && id <= 0x7fffffff

/** A person's SteamID as Pulse stores a linked one: exactly 17 digits. */
export const validSteamId = (s: string): boolean => s.length === 17 && digits(s)

/** A lobby's SteamID: digits, no leading zero, a u64 in the public universe of the chat type with the lobby flag. */
export function validLobby(s: string): boolean {
  if (!s || s.length > 20 || s.startsWith('0') || !digits(s)) return false
  const id = BigInt(s)
  if (id > 0xffffffffffffffffn) return false
  const universe = id >> 56n
  const kind = (id >> 52n) & 0xfn
  const instance = (id >> 32n) & 0xfffffn
  return universe === 1n && kind === 8n && (instance & 0x40000n) !== 0n && (id & 0xffffffffn) !== 0n
}

/** `a.b.c.d:port`: four decimal octets without leading zeros, a reachable address (not 0.0.0.0, loopback,
 *  link-local, broadcast or multicast) and a port from 1 to 65535. */
export function validServer(s: string): boolean {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3}):(\d{1,5})$/.exec(s)
  if (!m) return false
  const parts = m.slice(1, 6)
  if (parts.some((p) => p.length > 1 && p.startsWith('0'))) return false
  const [a, b, c, d, port] = parts.map(Number)
  if ([a, b, c, d].some((o) => o > 255) || port < 1 || port > 65535) return false
  if (a === 0 || a === 127 || a >= 224 || (a === 169 && b === 254)) return false
  return !(a === 255 && b === 255 && c === 255 && d === 255)
}

/** The steam:// URL for `j`, or null when anything in it doesn't check out. */
export function joinUrl(j: SteamJoin | null | undefined): string | null {
  if (!j || !validApp(j.app_id)) return null
  if (j.kind === 'lobby') return validLobby(j.lobby_id) && validSteamId(j.steam_id) ? `steam://joinlobby/${j.app_id}/${j.lobby_id}/${j.steam_id}` : null
  if (j.kind === 'server') return validServer(j.address) ? `steam://connect/${j.address}` : null
  return null
}

/** What `userId`'s game can be joined at, for you: someone else's that checks out. */
export function joinOf(s: AppState, userId: string): SteamJoin | null {
  if (userId === s.me?.id) return null
  const j = playingOf(s, userId)?.join ?? null
  return joinUrl(j) ? j : null
}
