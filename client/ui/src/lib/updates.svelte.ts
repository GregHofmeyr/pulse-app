// App updates (#82). The core checks (at launch, after login, every 30 min) and installs; this holds what the page
// shows: an offer ("Restart to update" in the title bar), and while installing, the "Updating Pulse…" screen with the
// download's progress. A failed install leaves Pulse exactly as it was, with the reason in `error`.
import { listen } from '@tauri-apps/api/event'
import { api, errorText, type UpdateOffer, type UpdateProgress } from './tauri'
import { voice } from './voice.svelte'

export const updates = $state({
  offer: null as UpdateOffer | null,
  installing: false,
  progress: null as UpdateProgress | null,
  error: null as string | null,
})

/** The core's update events: an offer found while running, and the download's progress. */
export async function startUpdateListening(): Promise<() => void> {
  const offs = await Promise.all([
    listen<UpdateOffer>('pulse://update', (e) => {
      updates.offer = e.payload
    }),
    listen<UpdateProgress>('pulse://update-progress', (e) => {
      updates.progress = e.payload
    }),
  ])
  return () => offs.forEach((off) => off())
}

/** Download and install the update (the "Updating Pulse…" screen shows meanwhile). On success Pulse closes and
 *  reopens on the new version, so this only ever comes back with a failure, which it rethrows. */
export async function installUpdate(restart: boolean): Promise<void> {
  updates.installing = true
  updates.progress = null
  updates.error = null
  try {
    await api.updateInstall(restart)
  } catch (e) {
    updates.installing = false
    updates.progress = null
    updates.error = errorText(e)
    void api.uiLog(`update: install failed: ${updates.error}`)
    throw e
  }
}

/** "Restart to update" waits while you're in a call (a stream needs one too): it never cuts a call. */
export function restartBlocked(): boolean {
  return voice.channelId !== null
}
