// Speaking rings (#651): the styles and colours someone can pick for the ring others see while they talk. The keys
// are the server's (`RING_STYLES` / `RING_COLORS` in server/src/profiles/mod.rs); a literal copy is in
// speakingRing.test.ts.

export type RingStyle =
  | 'classic'
  | 'glow'
  | 'pulse'
  | 'ripple'
  | 'sweep'
  | 'orbit'
  | 'halo'
  | 'viewfinder'
  | 'liquid'
  | 'dots'
  | 'bars'

export type RingColor = 'green' | 'blue' | 'gold' | 'pink' | 'cyan' | 'orange' | 'white' | 'purple' | 'profile'

/** Every style in picker order. `reactive`: it follows the speaker's loudness. */
export const RING_STYLES: { key: RingStyle; label: string; reactive: boolean }[] = [
  { key: 'classic', label: 'Classic', reactive: false },
  { key: 'glow', label: 'Soft glow', reactive: false },
  { key: 'pulse', label: 'Pulse', reactive: false },
  { key: 'ripple', label: 'Ripple', reactive: false },
  { key: 'sweep', label: 'Sweep', reactive: false },
  { key: 'orbit', label: 'Orbit', reactive: false },
  { key: 'halo', label: 'Halo', reactive: false },
  { key: 'viewfinder', label: 'Viewfinder', reactive: false },
  { key: 'liquid', label: 'Liquid', reactive: true },
  { key: 'dots', label: 'Wave of dots', reactive: true },
  { key: 'bars', label: 'Bars round the ring', reactive: true },
]

/** Every colour in picker order. `profile`: the first colour of the person's name style. */
export const RING_COLORS: { key: RingColor; label: string }[] = [
  { key: 'green', label: 'Green' },
  { key: 'blue', label: 'Pulse blue' },
  { key: 'gold', label: 'Gold' },
  { key: 'pink', label: 'Pink' },
  { key: 'cyan', label: 'Cyan' },
  { key: 'orange', label: 'Orange' },
  { key: 'white', label: 'White' },
  { key: 'purple', label: 'Purple' },
  { key: 'profile', label: 'Profile colour' },
]
