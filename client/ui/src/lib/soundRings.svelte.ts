// "Who is making that sound": while someone's soundboard play or join sound (#121) plays here, their id is in
// `rings.users`, and the voice list and call tiles light their ring for it (unioned at render time). It is never mixed
// into `voice.speaking`, which is the real voice activity that speaking-based features read. Its own module so the
// soundboard and join sounds share it without importing the voice store.

export const rings = $state({ users: new Set<string>() })
/** When each lit ring goes out (ms since epoch) and its timer. */
const ringEnds = new Map<string, { until: number; timer: ReturnType<typeof setTimeout> }>()

/** Light `userId`'s ring for `ms`. A new play of theirs cuts off the one before (#214), so it restarts the ring for
 * its own length, shorter or not. */
export function lightRing(userId: string, ms: number) {
  const until = Date.now() + ms
  const prev = ringEnds.get(userId)
  if (prev) clearTimeout(prev.timer)
  const timer = setTimeout(() => {
    ringEnds.delete(userId)
    const next = new Set(rings.users)
    next.delete(userId)
    rings.users = next
  }, ms)
  ringEnds.set(userId, { until, timer })
  if (!rings.users.has(userId)) rings.users = new Set([...rings.users, userId])
}

export function clearRings() {
  for (const { timer } of ringEnds.values()) clearTimeout(timer)
  ringEnds.clear()
  if (rings.users.size) rings.users = new Set()
}
