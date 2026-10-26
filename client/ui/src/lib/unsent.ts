import type { AppState, Pending } from './state'

// Unsent messages, per signed-in user: guarded localStorage, same pattern as lib/voiceui.ts.
const key = (userId: string) => `pulse.unsent.${userId}`

const isPending = (p: unknown): p is Pending => {
  const x = p as Partial<Pending> | null
  return !!x && typeof x.nonce === 'string' && typeof x.content === 'string' && (x.reply_to_id === null || typeof x.reply_to_id === 'string')
}

/** What an earlier run left unsent. Storage is untrusted: anything malformed gives `{}`. */
export function loadUnsent(userId: string): Record<string, Pending[]> {
  try {
    const raw = localStorage.getItem(key(userId))
    const parsed: unknown = raw ? JSON.parse(raw) : null
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: Record<string, Pending[]> = {}
    for (const [channel, list] of Object.entries(parsed)) {
      if (!Array.isArray(list)) continue
      const ok = list.filter(isPending).map((p): Pending => ({
        nonce: p.nonce, content: p.content, reply_to_id: p.reply_to_id, status: 'failed', created_at: typeof p.created_at === 'string' ? p.created_at : '',
      }))
      if (ok.length) out[channel] = ok
    }
    return out
  } catch {
    return {}
  }
}

export function saveUnsent(userId: string, pending: AppState['pending']) {
  try {
    const kept = Object.fromEntries(Object.entries(pending).filter(([, list]) => list.length > 0))
    if (Object.keys(kept).length) localStorage.setItem(key(userId), JSON.stringify(kept))
    else localStorage.removeItem(key(userId))
  } catch {
    /* ignore */
  }
}
