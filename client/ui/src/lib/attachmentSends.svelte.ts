// Messages with files on their way (#57): how far each upload is and why one failed, by nonce. Kept out of the app
// state so a progress tick redraws only the pending row's bar.
import type { AttachmentProgress } from './tauri'

type Send = { sent: number; total: number; error: string | null }

class AttachmentSends {
  #by = $state<Record<string, Send>>({})

  start(nonce: string) {
    this.#by[nonce] = { sent: 0, total: 0, error: null }
  }

  progress(p: AttachmentProgress) {
    this.#by[p.nonce] = { sent: p.sent, total: p.total, error: null }
  }

  failed(nonce: string, error: string) {
    const s = this.#by[nonce] ?? { sent: 0, total: 0, error: null }
    this.#by[nonce] = { ...s, error }
  }

  forget(nonce: string) {
    delete this.#by[nonce]
  }

  /** 0..1, or null before the first report. */
  fraction(nonce: string): number | null {
    const s = this.#by[nonce]
    if (!s || !s.total) return null
    return Math.min(1, s.sent / s.total)
  }

  error(nonce: string): string | null {
    return this.#by[nonce]?.error ?? null
  }
}

export const attachmentSends = new AttachmentSends()
