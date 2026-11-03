// Page errors nothing caught go to the app's log file (the blank window after a sign-in on 2026-11-03 left no trace
// there; only the devtools console had it). One line each: the error and the top of its stack.
import { api } from './tauri'

const FRAMES = 5

function describe(e: unknown): string {
  if (e instanceof Error) {
    const frames = (e.stack ?? '').split('\n').slice(1, FRAMES + 1).map((f) => f.trim().replace(/^at /, '')).filter(Boolean)
    return [`${e.name}: ${e.message}`, ...frames].join(' | ')
  }
  return String(e)
}

/** Starts logging uncaught errors and unhandled rejections; returns the stop. */
export function logUncaughtErrors(): () => void {
  const onError = (ev: ErrorEvent) => void api.uiLog(`uncaught: ${describe(ev.error ?? ev.message)}`, 'warn')
  const onRejection = (ev: Event) => void api.uiLog(`unhandled rejection: ${describe((ev as PromiseRejectionEvent).reason)}`, 'warn')
  window.addEventListener('error', onError)
  window.addEventListener('unhandledrejection', onRejection)
  return () => {
    window.removeEventListener('error', onError)
    window.removeEventListener('unhandledrejection', onRejection)
  }
}
