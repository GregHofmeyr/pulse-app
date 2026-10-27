// The Quote of the Year "pixel spill": solid green at one edge breaking into chunky pixels that
// thin out into the black. Deterministic per seed (the event id), so it never jumps on rerender.

/** One cell of the spill. Shade 0 is the bright green, 1 the mid green, 2 the deep green
 *  (`--quote-green`, `--quote-green-mid`, `--quote-green-deep`). */
export type SpillPixel = { x: number; y: number; shade: 0 | 1 | 2 }

/** FNV-1a: a string to a 32-bit seed. */
function hash(s: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193)
  return h >>> 0
}

/** mulberry32: a small seeded PRNG, uniform in [0, 1). */
function mulberry32(seed: number): () => number {
  let a = seed
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** Where the solid run ends, as a fraction of the width. */
const SOLID = 0.25

/**
 * The spill on a `cols` × `rows` grid, solid along x = 0 and spilling towards x = cols - 1:
 * the further from the solid edge, the fewer pixels and the darker they get.
 */
export function pixelSpill(seed: string, cols: number, rows: number): SpillPixel[] {
  const rand = mulberry32(hash(seed))
  const out: SpillPixel[] = []
  for (let x = 0; x < cols; x++) {
    const d = cols > 1 ? x / (cols - 1) : 0 // 0 at the solid edge, 1 at the far one
    const keep = d <= SOLID ? 1 : (1 - (d - SOLID) / (1 - SOLID)) ** 1.6
    for (let y = 0; y < rows; y++) {
      const r = rand()
      const tone = rand()
      if (r >= keep) continue
      // mostly bright near the edge, mostly deep far out
      const shade: SpillPixel['shade'] = tone < 0.85 - d * 0.9 ? 0 : tone < 1 - d * 0.45 ? 1 : 2
      out.push({ x, y, shade })
    }
  }
  return out
}
