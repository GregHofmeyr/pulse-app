// Reactive wrapper around the pure reducer in state.ts, fed by Tauri events from the Rust core.
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'
import { applyEvent, applyReady, emptyState, setPendingStatus, type AppState, type ConnState } from './state'
import { api } from './tauri'

export const app = $state<{ state: AppState }>({ state: emptyState() })

/** Subscribe to the live connection. Returns an unsubscribe function. */
export async function startListening(): Promise<UnlistenFn> {
  const offs: UnlistenFn[] = await Promise.all([
    listen<Ready>('pulse://ready', (e) => (app.state = applyReady(app.state, e.payload))),
    listen<Event>('pulse://event', (e) => (app.state = applyEvent(app.state, e.payload, Date.now()))),
    listen<ConnState>('pulse://conn', (e) => (app.state = { ...app.state, conn: e.payload })),
    listen<{ nonce: string; status: 'sent' | 'failed'; message: Message | null }>('pulse://outbox', (e) => {
      const { nonce, status, message } = e.payload
      if (status === 'sent' && message) {
        // Confirmed over REST: show it even if the gateway is down (the event, if it comes, dedupes by id).
        app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message, nonce } }, Date.now())
        return
      }
      if (status === 'failed') {
        for (const channel of Object.keys(app.state.pending)) {
          app.state = setPendingStatus(app.state, channel, nonce, 'failed')
        }
      }
    }),
  ])
  // Skip the backoff wait when the network comes back or the user looks at the app.
  const kick = () => {
    if (app.state.conn === 'reconnecting') void api.reconnectNow()
  }
  window.addEventListener('online', kick)
  window.addEventListener('focus', kick)
  return () => {
    offs.forEach((off) => off())
    window.removeEventListener('online', kick)
    window.removeEventListener('focus', kick)
  }
}

export function resetState() {
  app.state = emptyState()
}
