// Message reminders (#300): your list (from Ready, then ReminderSaved / ReminderRemoved, so every device agrees), and
// what the message menu and the inbox do with it. When one comes due the server puts it in your inbox (its count
// arrives as InboxUpdated) and this device tells you: the message sound, and a notification while Pulse isn't in focus
// that opens the message (or a line in the app while it is), unless Reminder alerts is off. One that came due while
// none of your devices was connected (`missed`, #412) is told when the next one connects: each device claims it from
// the server first, and only the one whose claim wins tells you, so you hear it once.
import { jumpTo } from './jump'
import { notifyPrefs } from './notifyPrefs.svelte'
import { openMessage } from './openMessage'
import type { Event } from './protocol/Event'
import type { MessageReminder } from './protocol/MessageReminder'
import { reminderPrefs } from './reminderPrefs.svelte'
import { sortReminders, upsertReminder, whenLabel } from './schedule'
import { displayName, userOf } from './selectors'
import { app, onAppEvent, onAppReady, ui } from './store.svelte'
import { api, errorText } from './tauri'
import { playSound } from './voiceui'
import { windowFocus } from './windowFocus.svelte'
import { BODY_MAX } from './notify'

export const reminders = $state({ list: [] as MessageReminder[] })

/** Those come due you haven't opened: what the inbox counts. */
export const unseenDue = (list: readonly MessageReminder[]) => list.filter((r) => r.due && !r.seen)

function put(r: MessageReminder) {
  reminders.list = upsertReminder(reminders.list, r)
}

function drop(id: string) {
  reminders.list = reminders.list.filter((r) => r.id !== id)
}

/** "Remind me about this" at `at` (an instant). */
export async function remindMe(messageId: string, at: string): Promise<MessageReminder> {
  const made = await api.createReminder(messageId, at)
  put(made)
  return made
}

/** "Remind me about this" from a menu (a quote's, a clip's, #412): set, then a line in the app says when. */
export async function remindAndSay(messageId: string, at: string): Promise<void> {
  const r = await remindMe(messageId, at)
  ui.notice = `I'll remind you ${whenLabel(r.remind_at, Date.now())}`
}

/** Cancel one still to come, or mark a due one done. */
export async function removeReminder(id: string): Promise<void> {
  await api.deleteReminder(id)
  drop(id)
}

/** The person a reminder's message is from, by the name that applies where it was said. */
export function reminderAuthor(r: MessageReminder): string {
  const id = r.message.author_id
  if (!id) return 'Pulse'
  return displayName(app.state, app.state.channels[r.channel_id]?.server_id ?? null, id)
}

/** Show a message where it lives: a reply in its thread (or a clip in its panel) once fetched, anything else in its
 *  chat's history. */
export async function openMessageAt(channelId: string, messageId: string, threadRootId: string | null): Promise<void> {
  const ch = app.state.channels[channelId]
  if (!ch) return void (ui.notice = 'That channel was deleted.')
  if (!threadRootId && ch.kind !== 'clips' && ch.kind !== 'builds') return jumpTo(channelId, messageId)
  // a build or its comment: the build's page (#618)
  if (ch.kind === 'builds') {
    ui.goTo = channelId
    ui.openBuild = { channelId, messageId: threadRootId ?? messageId }
    return
  }
  if (threadRootId && ch.kind === 'quotes') {
    ui.goTo = channelId
    ui.openReplies = { channelId, rootId: threadRootId }
    return
  }
  try {
    const page = await api.listMessagesAround(channelId, threadRootId ?? messageId)
    const root = page?.find((m) => m.id === (threadRootId ?? messageId))
    if (!root || root.deleted) return void (ui.notice = 'That message was deleted.')
    openMessage(threadRootId ? { ...root, id: messageId, thread_root_id: threadRootId } : root, threadRootId ? root : null)
  } catch (e) {
    ui.notice = `Couldn't open that message: ${errorText(e)}`
  }
}

/** Open a reminder's message; a due one is read (here at once, then on every device). */
export function openReminder(r: MessageReminder): void {
  if (r.due && !r.seen) {
    put({ ...r, seen: true })
    api.reminderSeen(r.id).catch(() => {})
  }
  void openMessageAt(r.channel_id, r.message_id, r.thread_root_id)
}

/** Mark all read (the inbox's) reads every due reminder here too; the server does the same. */
export function seeAllReminders(): void {
  if (!unseenDue(reminders.list).length) return
  reminders.list = reminders.list.map((r) => (r.due && !r.seen ? { ...r, seen: true } : r))
}

/** What the reminder's line and notification say: who said it and the start of what they said. */
export function reminderText(r: MessageReminder): string {
  const said = r.message.content.replace(/\s+/g, ' ').trim()
  const cut = [...said].length > BODY_MAX ? [...said].slice(0, BODY_MAX - 1).join('') + '…' : said
  return cut ? `${reminderAuthor(r)}: ${cut}` : `a message from ${reminderAuthor(r)}`
}

function alert(r: MessageReminder) {
  if (!reminderPrefs.alerts) return
  playSound('message')
  if (!windowFocus.focused && notifyPrefs.enabled) {
    const author = r.message.author_id ? userOf(app.state, r.message.author_id) : null
    const avatar = author?.avatar_still ?? author?.avatar_hash
    void api.notifyMessage({
      title: 'Reminder',
      body: notifyPrefs.showText ? reminderText(r) : `A message from ${reminderAuthor(r)}`,
      channelId: r.channel_id,
      ...(r.thread_root_id ? { threadId: r.thread_root_id } : {}),
      messageId: r.message_id,
      ...(avatar ? { avatar } : {}),
      tag: `reminder-${r.id}`,
    })
  } else if (windowFocus.focused) {
    ui.notice = `Reminder: ${reminderText(r)}`
  }
}

/** Tell you about reminders you missed (#412), once: one as it would have been told, several in one line. */
function alertMissed(list: readonly MessageReminder[]) {
  if (list.length === 1) return alert(list[0])
  if (!list.length || !reminderPrefs.alerts) return
  const text = `${list.length} reminders came due while you were away`
  playSound('message')
  if (!windowFocus.focused && notifyPrefs.enabled) {
    void api.notifyMessage({ title: 'Reminders', body: text, channelId: list[0].channel_id, messageId: list[0].message_id, tag: 'reminders-missed' })
  } else if (windowFocus.focused) {
    ui.notice = `${text}: they're in your inbox.`
  }
}

/** On connecting: claim each missed reminder (#412) and tell you about the ones this device won. With Reminder alerts
 *  off here nothing is claimed, so a device with them on can still tell you. */
export async function claimMissed(list: readonly MessageReminder[]): Promise<void> {
  if (!reminderPrefs.alerts) return
  const missed = list.filter((r) => r.missed && r.due && !r.seen)
  if (!missed.length) return
  const won = await Promise.all(missed.map((r) => api.reminderAlerted(r.id).then((ok) => ok, () => false)))
  const mine = missed.filter((_, i) => won[i])
  reminders.list = reminders.list.map((r) => (missed.some((m) => m.id === r.id) ? { ...r, missed: false } : r))
  alertMissed(mine)
}

/** A live event: yours only. One newly come due tells you (once, however often it's resent). */
export function noteReminderEvent(e: Event): void {
  if (e.t === 'ReminderSaved') {
    const was = reminders.list.find((r) => r.id === e.d.reminder.id)
    put(e.d.reminder)
    if (e.d.reminder.due && !e.d.reminder.seen && !was?.due) alert(e.d.reminder)
  } else if (e.t === 'ReminderRemoved') {
    drop(e.d.id)
  }
  // a deleted message's reminders go in lib/inbox.svelte.ts, which recounts the inbox when one was unread
}

onAppReady((r) => {
  reminders.list = sortReminders(r.reminders ?? [])
  void claimMissed(reminders.list)
})
onAppEvent(noteReminderEvent)
