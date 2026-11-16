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

const validPort = (p: string) => /^[1-9][0-9]{0,4}$/.test(p) && Number(p) <= 65535

/** An IPv6 address's eight groups, or null when it isn't one written in hex groups (no dotted tail, no zone). */
function ipv6Groups(a: string): number[] | null {
  const halves = a.split('::')
  if (halves.length > 2) return null
  const part = (x: string) => (x === '' ? [] : x.split(':'))
  const head = part(halves[0])
  const tail = halves.length === 2 ? part(halves[1]) : []
  if ([...head, ...tail].some((g) => !/^[0-9a-f]{1,4}$/.test(g))) return null
  const missing = 8 - head.length - tail.length
  if (halves.length === 1 ? missing !== 0 : missing < 1) return null
  return [...head, ...Array<string>(halves.length === 2 ? missing : 0).fill('0'), ...tail].map((g) => parseInt(g, 16))
}

/** The groups in their canonical form (RFC 5952, as the server writes them): lower case, no leading zeros, the
 *  longest run of two or more zero groups (the first, on a tie) as `::`. */
function ipv6Text(g: number[]): string {
  let best = { at: -1, len: 1 }
  for (let i = 0; i < 8;) {
    if (g[i] !== 0) { i++; continue }
    let j = i
    while (j < 8 && g[j] === 0) j++
    if (j - i > best.len) best = { at: i, len: j - i }
    i = j
  }
  const hex = (xs: number[]) => xs.map((x) => x.toString(16)).join(':')
  return best.at < 0 ? hex(g) : `${hex(g.slice(0, best.at))}::${hex(g.slice(best.at + best.len))}`
}

/** `[ipv6]:port` (#397): an address in its canonical form that could be reached over the internet (not unspecified,
 *  loopback, link-local, multicast, unique-local or IPv4-mapped) and a port from 1 to 65535. */
function validServer6(s: string): boolean {
  const m = /^\[([0-9a-f:]+)\]:([0-9]+)$/.exec(s)
  if (!m || !validPort(m[2])) return false
  const g = ipv6Groups(m[1])
  if (!g || ipv6Text(g) !== m[1]) return false
  const zeroUpTo = (n: number) => g.slice(0, n).every((x) => x === 0)
  if (zeroUpTo(8) || (zeroUpTo(7) && g[7] === 1)) return false
  if ((g[0] & 0xffc0) === 0xfe80 || (g[0] & 0xff00) === 0xff00 || (g[0] & 0xfe00) === 0xfc00) return false
  return !(zeroUpTo(5) && g[5] === 0xffff)
}

/** `a.b.c.d:port`: four decimal octets without leading zeros, a reachable address (not 0.0.0.0, loopback,
 *  link-local, broadcast or multicast) and a port from 1 to 65535; or (#397) `[ipv6]:port`, as strictly. */
export function validServer(s: string): boolean {
  if (s.startsWith('[')) return validServer6(s)
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
