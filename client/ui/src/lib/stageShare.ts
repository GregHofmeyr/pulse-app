// Spotify on the voice stage (#234): how a shared card reads, and who may stop it. Pure; the owner's open choices
// that the client mirrors are the constants here (the server's are in server/src/stage/mod.rs).
import { spotifyCard } from './embeds'
import type { StageShare } from './protocol/StageShare'

/** Anyone in the call may stop a share for everyone, not just its sharer (pick; the server enforces it too). */
export const ANYONE_IN_CALL_MAY_STOP = true

/** A Jam card says "may have ended" once it's been up this long: Pulse can't tell whether a Jam is still live. */
export const JAM_MAY_HAVE_ENDED_MS = 60 * 60 * 1000

export type StageCard = {
  title: string
  subtitle: string
  /** The link-out's wording, one of Spotify's approved labels (#196). */
  action: 'Join the Jam' | 'Listen on Spotify' | 'Open Spotify'
  jam: boolean
}

/** How `share`'s card reads, shared by `sharer` (a display name), at server time `now` (ms). */
export function stageCard(share: StageShare, sharer: string, now: number): StageCard {
  const sp = spotifyCard(share.embed)
  const by = `Shared by ${sharer}`
  if (sp?.jam) {
    const old = now - Date.parse(share.shared_at) >= JAM_MAY_HAVE_ENDED_MS
    return { title: 'Spotify Jam', subtitle: old ? `${by} · may have ended` : `Listen together · ${by}`, action: 'Join the Jam', jam: true }
  }
  const e = share.embed
  const what = e.author?.trim() || sp?.label || null
  return {
    title: e.title?.trim() || e.url,
    subtitle: what ? `${what} · ${by}` : by,
    action: sp?.action ?? 'Open Spotify',
    jam: false,
  }
}

/** Whether `me` may stop `share` for everyone: in the call, and anyone may (or it's theirs). */
export function mayStop(share: StageShare, me: string, inCall: boolean): boolean {
  return inCall && (ANYONE_IN_CALL_MAY_STOP || share.shared_by === me)
}

const PHRASES: Record<string, string> = {
  jam: 'a Jam', track: 'a song', album: 'an album', playlist: 'a playlist', episode: 'an episode', show: 'a podcast', artist: 'an artist',
}

/** How the share's chat line names what was shared ("alex shared a Jam to the stage"), as the server words it. */
export const sharePhrase = (kind: string): string => PHRASES[kind] ?? 'something'
