// A voice channel's bitrate (#75): Default (what the app has always sent) or a higher preset, up to 256 kbps: as much
// as the mic really sends, whatever it is allowed (client/src-tauri/tests/voice_bitrate_it.rs).
// The server takes only these numbers (CHANNEL_BITRATES_KBPS in protocol/src/rest.rs).

/** The kbps Default sends (the client's 64 kbps). */
export const DEFAULT_KBPS = 64

/** The presets above Default, in kbps. */
export const BITRATES_KBPS = [96, 128, 192, 256] as const

/** The choices for a Select (whose values are strings): null = Default. */
export const bitrateChoices: { value: string | null; label: string }[] = [
  { value: null, label: `Default (${DEFAULT_KBPS} kbps)` },
  ...BITRATES_KBPS.map((k) => ({ value: String(k), label: k === 256 ? `${k} kbps (max)` : `${k} kbps` })),
]

/** A channel's bitrate as a choice's value, and back. */
export const toChoice = (kbps: number | null | undefined): string | null => (kbps == null ? null : String(kbps))
export const fromChoice = (v: string | null): number | null => (v === null ? null : Number(v))
