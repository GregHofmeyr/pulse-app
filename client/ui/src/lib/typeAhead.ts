// Type-ahead for a list (#455): typing jumps to the next option whose label starts with what was typed, as a native
// <select> does. A leading '#' (channel names) is skipped, so typing "g" finds "#general".

const norm = (s: string) => s.replace(/^#/, '').trim().toLowerCase()

/** The index of the option `query` lands on, or -1. Searching starts after `from` for a single repeated letter
 *  (pressing "g" again cycles through the g's), at `from` itself otherwise (typing on narrows the same match).
 *  Disabled options are skipped. */
export function typeAheadIndex(labels: string[], query: string, from: number, disabled: (i: number) => boolean = () => false): number {
  const q = query.toLowerCase()
  if (!q || !labels.length) return -1
  const repeat = q.length > 1 && [...q].every((c) => c === q[0])
  const needle = repeat ? q[0] : q
  const start = q.length === 1 || repeat ? from + 1 : Math.max(from, 0)
  for (let k = 0; k < labels.length; k++) {
    const i = (((start + k) % labels.length) + labels.length) % labels.length
    if (!disabled(i) && norm(labels[i]).startsWith(needle)) return i
  }
  return -1
}

/** How long a pause ends a type-ahead word. */
export const TYPE_AHEAD_MS = 600
