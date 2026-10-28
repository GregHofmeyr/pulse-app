// The quality presets (mirror the core's encoder table) and the size estimate. `estimateBytes` is the one estimate:
// the core has none.

export type PresetId = 'light' | 'sharp'

/** `video_bps` is the encoder's own `-b:v`. */
export const PRESETS: Record<PresetId, { label: string; video_bps: number }> = {
  light: { label: 'Light', video_bps: 4_000_000 },
  sharp: { label: 'Sharp', video_bps: 8_000_000 },
}

export const DEFAULT_PRESET: PresetId = 'light'

const AUDIO_BPS = 160_000
/** Container overhead and a little rate wobble. */
const FACTOR = 1.02

/** The expected size of the sent clip in bytes. */
export function estimateBytes(preset: PresetId, keptMs: number): number {
  return Math.round(((PRESETS[preset].video_bps + AUDIO_BPS) * keptMs) / 8000 * FACTOR)
}
