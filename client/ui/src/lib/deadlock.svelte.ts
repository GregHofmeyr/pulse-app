// Deadlock ranks for the member list (#390): one read per server, again after a few minutes at most when the list
// is shown again. Nothing shows while deadlock-api is unreachable (the server answers no ranks) or on an older server.
import { api } from './tauri'
import type { DeadlockRank } from './protocol/DeadlockRank'

/** How long a server's ranks are shown before they're read again. */
export const RANKS_TTL_MS = 5 * 60_000

export const deadlockRanks = $state<Record<string, Record<string, DeadlockRank>>>({})
const readAt = new Map<string, number>()

/** Read `serverId`'s ranks unless they were read lately. */
export async function loadDeadlockRanks(serverId: string, now = Date.now()): Promise<void> {
  const at = readAt.get(serverId)
  if (at != null && now - at < RANKS_TTL_MS) return
  readAt.set(serverId, now)
  try {
    const got = await api.deadlockRanks(serverId)
    deadlockRanks[serverId] = Object.fromEntries(got.entries.map((e) => [e.user_id, e.rank]))
  } catch {
    // an older server, offline: no badges
  }
}

export const rankOf = (serverId: string, userId: string): DeadlockRank | null => deadlockRanks[serverId]?.[userId] ?? null

/** Tests: forget every read. */
export function resetDeadlockRanks(): void {
  readAt.clear()
  for (const k of Object.keys(deadlockRanks)) delete deadlockRanks[k]
}
