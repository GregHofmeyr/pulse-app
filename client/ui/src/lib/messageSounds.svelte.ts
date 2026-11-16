// Message sounds per person (#312): who has their own "New message" sound on this device (the rules are in
// messageSounds.ts), set from "Message sound" in their right-click menu.
import { loadMixer, soundGain } from './mixer'
import { loadMessageSounds, messageSoundSet, saveMessageSounds, withMessageSound, type MessageSounds } from './messageSounds'
import { loadSoundPrefs, playSoundAt, type SoundSetId } from './voiceui'

const held = $state({ people: loadMessageSounds() })

export const messageSounds = {
  get people(): MessageSounds {
    return held.people
  },
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
