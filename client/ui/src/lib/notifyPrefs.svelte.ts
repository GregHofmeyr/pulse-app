// "Desktop notifications" and "Show message text" (Settings → Notifications): per device, and
// per profile since each profile has its own webview storage. Both on unless switched off.
import type { NotifyPrefs } from './notify'

const ENABLED_KEY = 'pulse.notifications'
const TEXT_KEY = 'pulse.notificationText'

function saved(key: string): boolean {
  try {
    return localStorage.getItem(key) !== 'false'
  } catch {
    return true
  }
}

function save(key: string, on: boolean) {
  try {
    localStorage.setItem(key, String(on))
  } catch {
    /* ignore */
  }
}

export const notifyPrefs = $state<NotifyPrefs>({ enabled: saved(ENABLED_KEY), showText: saved(TEXT_KEY) })

export function setNotificationsEnabled(on: boolean): void {
  notifyPrefs.enabled = on
  save(ENABLED_KEY, on)
}

export function setNotificationText(on: boolean): void {
  notifyPrefs.showText = on
  save(TEXT_KEY, on)
}
