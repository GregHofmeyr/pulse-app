// Settings → Privacy → Activity (#275): "Read media activity" and "Show my activity", per device (each profile has
// its own webview storage), both off until turned on. Reading is the master switch: off, the core runs no watcher
// at all; on with Show off, what Spotify plays only feeds a Jam you put on the stage. The core is told at boot and on
// every change, and tells back what it reads (`pulse://media-activity`) for Settings to show.
import { listen } from '@tauri-apps/api/event'
import { api, type MediaState } from './tauri'

const READ_KEY = 'pulse.activity.read'
const SHOW_KEY = 'pulse.activity.show'

function saved(key: string): boolean {
  try {
    return localStorage.getItem(key) === 'true'
  } catch {
    return false
  }
}

function save(key: string, on: boolean) {
  try {
    localStorage.setItem(key, String(on))
  } catch {
    /* ignore */
  }
}

export const activityPrefs = $state({ read: saved(READ_KEY), show: saved(SHOW_KEY) })

/** What the core reads right now (Settings' "Spotify is playing …"). */
export const mediaActivity = $state<MediaState>({ reading: false, playing: null })

function tell(): void {
  void api.setMediaActivity(activityPrefs.read, activityPrefs.show).catch(() => {})
}

export function setReadMedia(on: boolean): void {
  activityPrefs.read = on
  save(READ_KEY, on)
  tell()
}

export function setShowActivity(on: boolean): void {
  activityPrefs.show = on
  save(SHOW_KEY, on)
  tell()
}

/** At boot: follow what the core reads, and tell it the saved switches (nothing runs until then). */
export async function startMediaActivity(): Promise<() => void> {
  const off = await listen<MediaState>('pulse://media-activity', (e) => {
    mediaActivity.reading = e.payload.reading
    mediaActivity.playing = e.payload.playing
  })
  tell()
  try {
    const now = await api.mediaActivity()
    mediaActivity.reading = now.reading
    mediaActivity.playing = now.playing
  } catch {
    /* stays as it was */
  }
  return off
}
