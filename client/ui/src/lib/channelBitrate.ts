// A voice channel's bitrate (#75): Default (what the app has always sent) or a higher preset, up to 128 kbps. The mic
// is mono Opus in voice mode (#587), which is transparent for speech well below that; more only costs data.
// The server takes these numbers and the older 192 and 256 (CHANNEL_BITRATES_KBPS in protocol/src/rest.rs), which
// the app sends as 128 (MAX_MIC_BPS in client/src-tauri/src/voice/mod.rs).

/** The kbps Default sends (the client's 64 kbps). */
export const DEFAULT_KBPS = 64

/** The presets above Default, in kbps. */
export const BITRATES_KBPS = [96, 128] as const

/** The most the mic sends, whatever the channel holds. */
export const MAX_KBPS = 128

/** The choices for a Select (whose values are strings): null = Default. */
export const bitrateChoices: { value: string | null; label: string }[] = [
  { value: null, label: `Default (${DEFAULT_KBPS} kbps)` },
  ...BITRATES_KBPS.map((k) => ({ value: String(k), label: k === MAX_KBPS ? `${k} kbps (max)` : `${k} kbps` })),
]

/** A channel's bitrate as a choice's value, and back. A channel still at 192 or 256 (from when the mic was stereo)
 * shows as the max, which is what it sends. */
export const toChoice = (kbps: number | null | undefined): string | null =>
  kbps == null ? null : String(Math.min(kbps, MAX_KBPS))
export const fromChoice = (v: string | null): number | null => (v === null ? null : Number(v))
