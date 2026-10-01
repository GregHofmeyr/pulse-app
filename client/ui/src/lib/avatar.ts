// Stable avatar colour from a user id (until real avatars land). White text passes contrast on all.
const PALETTE = ['#4a5bd4', '#b5562a', '#a23d86', '#2d7d6b', '#6b5bb5', '#8a6a1f', '#2f6fa8', '#9b3b3b']

export function avatarColor(id: string): string {
  let h = 0
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0
  return PALETTE[h % PALETTE.length]
}

export const initial = (name: string) => name.trim().charAt(0).toUpperCase() || '?'
