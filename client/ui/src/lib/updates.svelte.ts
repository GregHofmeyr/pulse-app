// App updates (#82). The core checks (at launch, after login, every 30 min) and installs; this holds what the page
// shows: an offer ("Restart to update" in the title bar), and while installing, the "Updating Pulse…" screen with the
// download's progress. A failed install leaves Pulse exactly as it was, with the reason in `error`.
import { listen } from '@tauri-apps/api/event'
import { api, errorText, type UpdateOffer, type UpdateProgress } from './tauri'
import { voice } from './voice.svelte'
import { windowFocus } from './windowFocus.svelte'

export const updates = $state({
  offer: null as UpdateOffer | null,
  installing: false,
  progress: null as UpdateProgress | null,
  error: null as string | null,
})

/** The core's update events: an offer found while running (`null`: a check found nothing any more, so an offer the
 *  server withdrew goes away), and the download's progress. */
export async function startUpdateListening(): Promise<() => void> {
  const offs = await Promise.all([
    listen<UpdateOffer | null>('pulse://update', (e) => {
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

/** In a call the title bar says "Update ready" with this (#536): nothing restarts mid-call, and quitting Pulse installs
 *  it at the next launch. */
export const UPDATE_READY_TIP = 'It applies when you restart Pulse. Restart to update comes back when your call ends'

/** "Restart to update" waits while you're in a call (a stream needs one too): it never cuts a call. Joining one, or
 *  one that's reconnecting, counts (the core refuses then too). */
export function restartBlocked(): boolean {
  return voice.channelId !== null || voice.joining !== null || voice.stage.stage === 'reconnecting'
}

/** #460: an update found while running installs by itself once Pulse has been left alone (no input in its window, or
 *  not focused) this long, with nothing going on (`blocked`). */
export const IDLE_INSTALL_MS = 10 * 60_000
/** How often the idle install looks. */
export const IDLE_POLL_MS = 30_000

/** Whether to install now: an offer waits, nothing is installing, Pulse has been left alone long enough and nothing a
 *  restart would cut is going on. */
export function idleInstallDue(v: { offer: boolean; installing: boolean; quietMs: number; blocked: boolean }): boolean {
  return v.offer && !v.installing && !v.blocked && v.quietMs >= IDLE_INSTALL_MS
}

/** Install an update found while running once Pulse has been left alone [`IDLE_INSTALL_MS`] (#460). `blocked`: whether
 *  something a restart would cut is going on (a call, going live, a clip or message still sending, unsent text). The
 *  core checks the call again and refuses after a failed install; a refusal waits another full quiet spell. */
export function startIdleInstall(blocked: () => boolean): () => void {
  let lastActive = Date.now()
  const active = () => {
    lastActive = Date.now()
  }
  const events = ['pointerdown', 'pointermove', 'keydown', 'wheel'] as const
  for (const e of events) window.addEventListener(e, active, { passive: true, capture: true })
  let wasFocused = windowFocus.focused
  const timer = setInterval(() => {
    // coming back to the window counts as being there, even before any input
    if (windowFocus.focused && !wasFocused) active()
    wasFocused = windowFocus.focused
    const due = idleInstallDue({ offer: !!updates.offer, installing: updates.installing, quietMs: Date.now() - lastActive, blocked: blocked() })
    if (!due) return
    active() // a refusal or a failure waits another full quiet spell
    void installWhenIdle()
  }, IDLE_POLL_MS)
  return () => {
    clearInterval(timer)
    for (const e of events) window.removeEventListener(e, active, { capture: true })
  }
}

async function installWhenIdle(): Promise<void> {
  updates.installing = true
  updates.progress = null
  updates.error = null
  void api.uiLog(`update: left alone with ${updates.offer?.version ?? 'an update'} waiting; installing`, 'info')
  try {
    await api.updateInstallIdle()
  } catch (e) {
    // refused (a call started, a clip is uploading, a recent failed install) or failed: carry on as before, quietly
    updates.installing = false
    updates.progress = null
    void api.uiLog(`update: idle install didn't go ahead: ${errorText(e)}`)
  }
}
