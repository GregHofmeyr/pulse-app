// "Show in-app toasts" and one switch per kind (Settings → Notifications, #166): per device, all on unless switched
// off. Its own key, apart from the desktop-notification prefs; each toast asks `toastShown` once, where it shows.

export type ToastKind = 'live' | 'badge' | 'stream'

/** The kinds, in the order the settings list them. */
export const TOAST_KINDS: readonly { kind: ToastKind; label: string }[] = [
  { kind: 'live', label: 'Someone goes live' },
  { kind: 'badge', label: 'Badge unlocked' },
  { kind: 'stream', label: 'Stream updates' },
]

export type ToastPrefs = { enabled: boolean } & Record<ToastKind, boolean>

const KEY = 'pulse.toasts'
const ALL_ON: ToastPrefs = { enabled: true, live: true, badge: true, stream: true }

function saved(): ToastPrefs {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    if (!v || typeof v !== 'object' || Array.isArray(v)) return { ...ALL_ON }
    const o = v as Record<string, unknown>
    // anything but a stored false is on (a kind added later starts on)
    return Object.fromEntries(Object.keys(ALL_ON).map((k) => [k, o[k] !== false])) as ToastPrefs
  } catch {
    return { ...ALL_ON }
  }
}

export const toastPrefs = $state<ToastPrefs>(saved())

export function setToastPref(key: keyof ToastPrefs, on: boolean): void {
  toastPrefs[key] = on
  try {
    localStorage.setItem(KEY, JSON.stringify(toastPrefs))
  } catch {
    /* ignore */
  }
}

/** Whether a toast of this kind may show: the master switch and its own. */
export function toastShown(kind: ToastKind): boolean {
  return toastPrefs.enabled && toastPrefs[kind]
}

/** Read back from storage (tests: as after a restart). */
export function reloadToastPrefs(): void {
  Object.assign(toastPrefs, saved())
}
