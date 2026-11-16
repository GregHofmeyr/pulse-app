// Message sounds per person (#312), the pure part: on this device, a person can have their own "New message" sound,
// taken from the sound sets (every set has its own message sound), so you can tell who wrote without looking. Anyone
// without one plays the set you picked in Settings. Only the sound changes: whether a message dings at all is
// still messageSound's (mutes, levels, DND, the gap).
import { SOUND_SETS, type SoundSetId } from './voiceui'

const KEY = 'pulse.messageSounds'

/** People with their own message sound (user id -> set), on this device. */
export type MessageSounds = Record<string, SoundSetId>

const isSet = (v: unknown): v is SoundSetId => SOUND_SETS.some((s) => s.id === v)

export function loadMessageSounds(): MessageSounds {
  try {
    const raw = localStorage.getItem(KEY)
    const r: unknown = raw ? JSON.parse(raw) : null
    if (typeof r !== 'object' || r === null || Array.isArray(r)) return {}
    return Object.fromEntries(Object.entries(r).filter(([, v]) => isSet(v))) as MessageSounds
  } catch {
    return {}
  }
}

export function saveMessageSounds(m: MessageSounds) {
  try {
    localStorage.setItem(KEY, JSON.stringify(m))
  } catch {
    // storage unavailable: the setting just won't survive a restart
  }
}

/** The set `authorId`'s message sound comes from: theirs, or `fallback` (the one picked in Settings). */
export const messageSoundSet = (m: MessageSounds, authorId: string, fallback: SoundSetId): SoundSetId => m[authorId] ?? fallback

/** `m` with `userId`'s sound set to `set`, or cleared (null: back to the default). */
export function withMessageSound(m: MessageSounds, userId: string, set: SoundSetId | null): MessageSounds {
  const { [userId]: _old, ...rest } = m
  return set ? { ...rest, [userId]: set } : rest
}

/** The flyout's choices: "Default (<your set>)", then every set, with `current` ticked. */
export function messageSoundChoices(current: SoundSetId | null, fallback: SoundSetId): { label: string; set: SoundSetId | null; checked: boolean }[] {
  const def = SOUND_SETS.find((s) => s.id === fallback)?.label ?? 'Classic'
  return [
    { label: `Default (${def})`, set: null, checked: current === null },
    ...SOUND_SETS.map((s) => ({ label: s.label, set: s.id, checked: current === s.id })),
  ]
}
