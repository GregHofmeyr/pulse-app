// Stable avatar colour from a user id (until real avatars land). White text passes contrast on all.
const PALETTE = ['#4a5bd4', '#b5562a', '#a23d86', '#2d7d6b', '#6b5bb5', '#8a6a1f', '#2f6fa8', '#9b3b3b']

export function avatarColor(id: string): string {
  let h = 0
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0
  return PALETTE[h % PALETTE.length]
}

/** Two stable colours from an id (#475, #55): its avatar colour and the next in the palette. A server's banner
 *  fallback is a gradient of the two, solid end to end. */
export function avatarColors(id: string): [string, string] {
  const first = avatarColor(id)
  return [first, PALETTE[(PALETTE.indexOf(first) + 3) % PALETTE.length]]
}

export const initial = (name: string) => name.trim().charAt(0).toUpperCase() || '?'
