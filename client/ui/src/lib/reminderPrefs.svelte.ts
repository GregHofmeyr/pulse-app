// The per-device "Reminder alerts" setting (#300, Settings → Notifications): when a reminder comes due, the message
// sound and (while Pulse isn't in focus, with desktop notifications on) a notification. Off: it only lands in your
// inbox. On unless switched off.
const KEY = 'pulse.reminderAlerts'

function savedOn(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'off'
  } catch {
    return true
  }
}

export const reminderPrefs = $state({ alerts: savedOn() })

export function setReminderAlerts(on: boolean): void {
  reminderPrefs.alerts = on
  try {
    localStorage.setItem(KEY, on ? 'on' : 'off')
  } catch {
    /* ignore */
  }
}

/** Read back from storage (tests: as after a restart). */
export function reloadReminderPrefs(): void {
  reminderPrefs.alerts = savedOn()
}
