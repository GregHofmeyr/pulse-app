// Scheduled messages (#299): your list (from Ready, then ScheduledSaved / ScheduledRemoved, so every device agrees),
// and what the composer and the Scheduled lists do with it. One the server couldn't post when its time came comes back
// failed: you're told (a line in the app, and a desktop notification while Pulse isn't in focus), and it stays listed
// until you delete it or send it again.
import type { Event } from './protocol/Event'
import type { ScheduledMessage } from './protocol/ScheduledMessage'
import { notifyPrefs } from './notifyPrefs.svelte'
import { sortScheduled, upsertScheduled } from './schedule'
import { conversationName } from './conversations'
import { channelLabel } from './selectors'
import { app, onAppEvent, onAppReady, ui } from './store.svelte'
import { api } from './tauri'
import { windowFocus } from './windowFocus.svelte'

export const scheduled = $state({ list: [] as ScheduledMessage[] })

/** Your scheduled messages in one chat, soonest first. */
export function scheduledIn(channelId: string): ScheduledMessage[] {
  return scheduled.list.filter((s) => s.channel_id === channelId)
}

/** Where a scheduled message goes, as the lists name it: "#general", or a DM's or group's name. */
export function whereTo(channelId: string): string {
  const ch = app.state.channels[channelId]
  if (!ch) return 'a deleted chat'
  return ch.server_id ? channelLabel(ch) : conversationName(app.state, channelId)
}

function put(item: ScheduledMessage) {
  scheduled.list = upsertScheduled(scheduled.list, item)
}

/** Schedule `text` for `channelId` at `sendAt` (an instant); shown at once (its ScheduledSaved only confirms it). */
export async function scheduleMessage(channelId: string, text: string, replyToId: string | null, sendAt: string): Promise<ScheduledMessage> {
  const made = await api.scheduleMessage(channelId, text, replyToId, sendAt)
  put(made)
  return made
}

export async function editScheduled(id: string, change: { content?: string; sendAt?: string }): Promise<void> {
  put(await api.editScheduled(id, change))
}

/** Post it now: it leaves the list (a failed one that still can't go stays, and the error says why). */
export async function sendScheduledNow(id: string): Promise<void> {
  await api.sendScheduledNow(id)
  scheduled.list = scheduled.list.filter((s) => s.id !== id)
}

export async function deleteScheduled(id: string): Promise<void> {
  await api.deleteScheduled(id)
  scheduled.list = scheduled.list.filter((s) => s.id !== id)
}

/** A live event: yours only (the server sends them to your own devices). */
export function noteScheduledEvent(e: Event): void {
  if (e.t === 'ScheduledSaved') {
    const was = scheduled.list.find((s) => s.id === e.d.scheduled.id)
    put(e.d.scheduled)
    const failed = e.d.scheduled.failed
    if (failed && !was?.failed) tellFailed(e.d.scheduled, failed)
  } else if (e.t === 'ScheduledRemoved') {
    scheduled.list = scheduled.list.filter((s) => s.id !== e.d.id)
  }
}

function tellFailed(s: ScheduledMessage, why: string) {
  const where = whereTo(s.channel_id)
  ui.notice = `A scheduled message to ${where} couldn't be sent: ${why}`
  if (notifyPrefs.enabled && !windowFocus.focused) {
    void api.notifyMessage({ title: `Couldn't send your scheduled message to ${where}`, body: why, channelId: s.channel_id, tag: `scheduled-${s.id}` })
  }
}

onAppReady((r) => (scheduled.list = sortScheduled(r.scheduled ?? [])))
onAppEvent(noteScheduledEvent)
