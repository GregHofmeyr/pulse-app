// Files being saved to disk (#205): a clip, a quoted image or a message's file. The Rust core opens the Save dialog,
// streams the file and reports how far it is (`download://progress`, at most ten a second); here each download's
// key maps to what its icon or button shows. A second click while it downloads cancels it (the core deletes the
// partial file and answers false: no error). Kept out of the app state so a progress tick redraws only that button.
import { api, errorText, type DownloadProgress } from './tauri'

export type DownloadView =
  /** `fraction` 0..1; null when the server didn't say how big it is (an indeterminate ring). */
  | { phase: 'downloading'; fraction: number | null }
  | { phase: 'saved' }

/** How long a finished download says Saved. */
export const SAVED_MS = 2_500

export const clipKey = (messageId: string) => `clip:${messageId}`
export const imageKey = (hash: string) => `image:${hash}`
export const fileKey = (attachmentId: string) => `file:${attachmentId}`

type Running = { label: string; cancelled: boolean }

class Downloads {
  #by = $state<Record<string, DownloadView>>({})
  // From the click until the core answers: the Save dialog, then the download. Not drawn, so not reactive.
  #running = new Map<string, Running>()
  #timers = new Map<string, ReturnType<typeof setTimeout>>()
  /** The polite live region's text: a download starting, finishing or stopping (never each percent). */
  announcement = $state('')

  view(key: string): DownloadView | null {
    return this.#by[key] ?? null
  }

  /** The core's report; ignored once that download has ended (a late event). */
  progress(p: DownloadProgress) {
    const run = this.#running.get(p.key)
    if (!run) return
    const started = this.#by[p.key]?.phase !== 'downloading'
    const fraction = p.total ? Math.min(1, p.received / p.total) : null
    this.#by[p.key] = { phase: 'downloading', fraction }
    if (started) {
      this.#clearTimer(p.key)
      this.announcement = `Downloading ${run.label}`
    }
  }

  /**
   * The download button: starts `save` (the core's command: true when saved, false when its dialog or the download
   * was cancelled), or cancels the one downloading under `key`. A click while its Save dialog is open does nothing.
   * `label` names it for screen readers ("clip", a file's name).
   */
  async toggle(key: string, label: string, save: () => Promise<boolean>, onError: (text: string) => void): Promise<void> {
    if (this.#running.has(key)) {
      if (this.#by[key]?.phase === 'downloading') this.cancel(key)
      return
    }
    const run: Running = { label, cancelled: false }
    this.#running.set(key, run)
    let saved = false
    try {
      saved = await save()
    } catch (e) {
      this.#running.delete(key)
      delete this.#by[key]
      // the owner's error line (role="alert") says it; a stale "Downloading…" must not linger
      if (this.announcement === `Downloading ${label}`) this.announcement = ''
      onError(errorText(e))
      return
    }
    this.#running.delete(key)
    const wasDownloading = this.#by[key]?.phase === 'downloading'
    if (saved) {
      this.#by[key] = { phase: 'saved' }
      this.announcement = `Saved ${label}`
      this.#clearTimer(key)
      this.#timers.set(key, setTimeout(() => {
        this.#timers.delete(key)
        if (this.#by[key]?.phase === 'saved') delete this.#by[key]
      }, SAVED_MS))
    } else {
      delete this.#by[key]
      if (run.cancelled && wasDownloading) this.announcement = `Stopped downloading ${label}`
    }
  }

  /** Stop the download under `key` (the core deletes its partial file). */
  cancel(key: string) {
    const run = this.#running.get(key)
    if (!run || run.cancelled) return
    run.cancelled = true
    void api.cancelDownload(key).catch(() => {})
  }

  /** Tests: forget everything. */
  reset() {
    for (const t of this.#timers.values()) clearTimeout(t)
    this.#timers.clear()
    this.#running.clear()
    this.#by = {}
    this.announcement = ''
  }

  #clearTimer(key: string) {
    clearTimeout(this.#timers.get(key))
    this.#timers.delete(key)
  }
}

export const downloads = new Downloads()
