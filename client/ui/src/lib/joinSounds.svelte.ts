// Join sounds (#121): the per-device mutes, the level, and playing one (the rules are in joinSounds.ts). A join sound
// plays like a soundboard play: locally, through the core's soundboard player, at Calls × Join sounds × the sound's
// own volume, and lights the joiner's ring while it plays. Anything that keeps it from playing falls back to the
// normal Join cue, so a join is never silent.
import { mixer } from './mixer.svelte'
import { lightRing } from './soundRings.svelte'
import { soundboardCmd } from './tauri'
import { loadAudioConfig, playSound } from './voiceui'
import { loadJoinSoundMutes, saveJoinSoundMutes, type JoinAction } from './joinSounds'

const muted = $state({ people: loadJoinSoundMutes() })

export const joinSounds = {
  /** People whose join sound you hear as the normal Join cue. */
  get mutes(): Record<string, true> {
    return muted.people
  },
  set mutes(m: Record<string, true>) {
    muted.people = m
  },
  /** The Join sounds level a join sound plays at: 0 when either it or the Calls master is off. */
  get level(): number {
    return mixer.calls.master > 0 ? mixer.calls.join_sounds : 0
  },
}

/** "Mute join sound" / "Unmute join sound" in someone's menu. */
export function setJoinSoundMute(userId: string, on: boolean) {
  const { [userId]: _off, ...rest } = muted.people
  muted.people = on ? { ...rest, [userId]: true } : rest
  saveJoinSoundMutes($state.snapshot(muted.people))
}

/** Play what `userId`'s join sounds like here: their join sound (cutting off their previous soundboard sound, as a play
 * does) or the normal Join cue. `stillIn` says whether you're still in that call once it's queued (the ring is lit
 * then). A join sound that can't be played (its file won't load) becomes the Join cue. */
export function playJoin(a: JoinAction, userId: string, stillIn: () => boolean) {
  if (a.kind === 'cue') {
    playSound('join')
    return
  }
  const { play } = a
  soundboardCmd
    .play(play.media_hash, mixer.calls.master, mixer.calls.join_sounds, play.volume, loadAudioConfig().output ?? null, play.sound_id, userId)
    .then(() => {
      if (stillIn()) lightRing(userId, play.duration_ms || 2000)
    })
    .catch(() => playSound('join'))
}
