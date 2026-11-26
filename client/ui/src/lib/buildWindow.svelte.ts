// The build pop-out (#620, spec §4.4), from the main window's side. Pop out hands the core one build's snapshot and
// opens the window (always on top to start); from then on this module, not the build page (which may be gone), sends
// that build's edits and archive on, and closes the window when the build is deleted or out of reach (its channel
// left the data: you left the server, it was deleted). The core closes it too on a delete it sees and on a 403 or 404
// for its screenshots, so a reload of this page or a delete while it was away can't leave it open. Signing out closes
// it in the core and forgets it here.
import { snapshotOf } from './buildSnapshot'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import { app, onAppEvent, onAppReady, onAppReset } from './store.svelte'
import { api } from './tauri'

/** The build in the pop-out (null: none, as far as this window knows). */
export const poppedOut = $state({ messageId: null as string | null, channelId: null as string | null })
/** Its copy, kept current by events (to make the next snapshot from). */
let held: Message | null = null
/** Pop outs so far: of two at once, the core shows the later in its one window (#639), so only the later is noted. */
let opens = 0

/** Pop `m` out, replacing any build already popped out (in the same window). */
export async function popOutBuild(m: Message): Promise<void> {
  if (!m.build || m.deleted) return
  const n = ++opens
  await api.openBuildWindow(snapshotOf(m, app.state), true)
  if (n !== opens) return
  held = m
  poppedOut.messageId = m.id
  poppedOut.channelId = m.channel_id
}

function send(m: Message) {
  held = m
  void api.updateBuildWindow(snapshotOf(m, app.state)).catch(() => {})
}

function close() {
  const id = poppedOut.messageId
  forgetBuildWindow()
  if (id) void api.closeBuildWindow(id).catch(() => {})
}

/** A live event (after the state applied it): the popped-out build's edit, archive or delete, or its channel leaving. */
export function noteBuildWindowEvent(e: Event): void {
  const id = poppedOut.messageId
  if (!id) return
  if (e.t === 'MessageUpdated' && !e.d.message.thread_root_id && e.d.message.id === id) {
    if (e.d.message.deleted || !e.d.message.build) return close()
    send(e.d.message)
  } else if (e.t === 'MessageDeleted' && !e.d.thread_root_id && e.d.message_id === id) {
    return close()
  } else if (e.t === 'BuildsArchived' && e.d.message_ids.includes(id) && held?.build) {
    send({ ...held, build: { ...held.build, archived_at: e.d.archived_at, archived_by: e.d.archived_by } })
  }
  if (poppedOut.channelId && !app.state.channels[poppedOut.channelId]) close()
}

/** A Ready: a pop-out whose channel is no longer in the data closes. */
export function noteBuildWindowReady(): void {
  if (poppedOut.channelId && !app.state.channels[poppedOut.channelId]) close()
}

/** Nothing popped out (its build closed; tests). A Pop out of another build still in flight is noted when it lands. */
export function forgetBuildWindow(): void {
  held = null
  poppedOut.messageId = null
  poppedOut.channelId = null
}

/** Signed out: nothing of the last account's pop-out is kept, nor noted when a Pop out still in flight lands. */
export function resetBuildWindow(): void {
  opens++
  forgetBuildWindow()
}

onAppReady(() => noteBuildWindowReady())
onAppEvent(noteBuildWindowEvent)
// signed out: nothing of the last account's pop-out is kept here (the core has closed the window)
onAppReset(resetBuildWindow)
