// Message sounds per person (#312): who has their own "New message" sound on this device (the rules are in
// messageSounds.ts), set from "Message sound" in their right-click menu; and channels with their own (#378), set from
// the channel's (or group's) right-click menu.
import { loadMixer, soundGain } from './mixer'
import { loadChannelMessageSounds, loadMessageSounds, messageSoundIn, messageSoundSet, saveChannelMessageSounds, saveMessageSounds, withMessageSound, type MessageSounds } from './messageSounds'
import { loadSoundPrefs, playSoundAt, type SoundSetId } from './voiceui'

const held = $state({ people: loadMessageSounds(), channels: loadChannelMessageSounds() })

export const messageSounds = {
  get people(): MessageSounds {
    return held.people
  },
  get channels(): MessageSounds {
    return held.channels
  },
}

/** The set a message by `authorId` (none: a system line) in `channelId` dings with: its author's, the channel's, or
 *  the one picked in Settings. */
export const messageSetIn = (authorId: string | null, channelId: string): SoundSetId =>
  messageSoundIn(held.people, held.channels, authorId, channelId, loadSoundPrefs().set)

/** Picked in a channel's menu (#378; null: back to the default). It plays the new sound once, as for a person. */
export function setChannelMessageSound(channelId: string, set: SoundSetId | null, { preview = true } = {}) {
  held.channels = withMessageSound(held.channels, channelId, set)
  saveChannelMessageSounds($state.snapshot(held.channels))
  if (preview) playSoundAt('message', soundGain(loadMixer(), 'message'), set ?? loadSoundPrefs().set)
}

/** The set a message from `authorId` dings with: theirs, or the one picked in Settings. */
export const messageSetFor = (authorId: string): SoundSetId => messageSoundSet(held.people, authorId, loadSoundPrefs().set)

/** Picked in someone's menu (null: back to the default). It plays the new sound once, at the mixer's message level, so
 * you hear what you picked. */
export function setMessageSound(userId: string, set: SoundSetId | null, { preview = true } = {}) {
  held.people = withMessageSound(held.people, userId, set)
  saveMessageSounds($state.snapshot(held.people))
  if (preview) playSoundAt('message', soundGain(loadMixer(), 'message'), messageSetFor(userId))
}
