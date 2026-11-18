// Message sounds per person (#312), the pure part: on this device, a person can have their own "New message" sound,
// taken from the sound sets (every set has its own message sound), so you can tell who wrote without looking. Anyone
// without one plays the set you picked in Settings. Only the sound changes: whether a message dings at all is
// still messageSound's (mutes, levels, DND, the gap). A channel can have its own too (#378): a message there dings with
// it unless its author has their own (who wrote is the more telling of the two).
import { SOUND_SETS, type SoundSetId } from './voiceui'

const KEY = 'pulse.messageSounds'
const CHANNEL_KEY = 'pulse.channelMessageSounds'

/** People (user id -> set), or channels (channel id -> set, #378), with their own message sound, on this device. */
export type MessageSounds = Record<string, SoundSetId>

const isSet = (v: unknown): v is SoundSetId => SOUND_SETS.some((s) => s.id === v)

export const loadMessageSounds = (): MessageSounds => load(KEY)
export const loadChannelMessageSounds = (): MessageSounds => load(CHANNEL_KEY)

function load(key: string): MessageSounds {
  try {
    const raw = localStorage.getItem(key)
    const r: unknown = raw ? JSON.parse(raw) : null
    if (typeof r !== 'object' || r === null || Array.isArray(r)) return {}
    return Object.fromEntries(Object.entries(r).filter(([, v]) => isSet(v))) as MessageSounds
  } catch {
    return {}
  }
}

export const saveMessageSounds = (m: MessageSounds) => save(KEY, m)
export const saveChannelMessageSounds = (m: MessageSounds) => save(CHANNEL_KEY, m)

function save(key: string, m: MessageSounds) {
  try {
    localStorage.setItem(key, JSON.stringify(m))
  } catch {
    // storage unavailable: the setting just won't survive a restart
  }
}

/** The set `authorId`'s message sound comes from: theirs, or `fallback` (the one picked in Settings). */
export const messageSoundSet = (m: MessageSounds, authorId: string, fallback: SoundSetId): SoundSetId => m[authorId] ?? fallback

/** The set a message by `authorId` (none: a system line) in `channelId` dings with (#378): its author's own, else the
 *  channel's own, else `fallback`. */
export const messageSoundIn = (people: MessageSounds, channels: MessageSounds, authorId: string | null, channelId: string, fallback: SoundSetId): SoundSetId =>
  (authorId ? people[authorId] : undefined) ?? channels[channelId] ?? fallback

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
