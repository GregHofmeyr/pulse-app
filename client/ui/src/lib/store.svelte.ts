// Reactive wrapper around the pure reducer in state.ts, fed by Tauri events from the Rust core.
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'
import { applyEvent, applyReady, emptyState, setPendingStatus, type AppState, type ConnState } from './state'
import { api } from './tauri'
import { isMuted, messageSound } from './conversations'
import { playSound } from './voiceui'

export const app = $state<{ state: AppState }>({ state: emptyState() })

/** What the user is looking at: drives the message sound and read marking. */
export const ui = $state({ openChannelId: null as string | null, focused: true })

/** One shared clock for time-dependent UI (mute expiry, "5m ago"). Ticks every 30 s. */
export const clock = $state({ nowIso: new Date().toISOString() })
setInterval(() => (clock.nowIso = new Date().toISOString()), 30_000)
let lastPlayedAt = 0

function maybeSound(e: Event) {
  if (e.t !== 'MessageCreated') return
  const m = e.d.message
  const ch = app.state.channels[m.channel_id]
  const me = app.state.me
  if (!ch || !me) return
  const now = Date.now()
  const play = messageSound({
    message: m,
    me: me.id,
    channel: ch,
    open: ui.openChannelId === m.channel_id,
    focused: ui.focused,
    muted: isMuted(app.state, ch, new Date(now).toISOString()),
    lastPlayedAt,
    now,
  })
  if (play) {
    lastPlayedAt = now
    playSound('message')
  }
}

/** Open (or reuse) your DM with `userId`; resolves to its channel id. */
export async function openDm(userId: string): Promise<string> {
  const ch = await api.createDm([userId])
  // Show it straight away: the ChannelCreated gateway event may arrive after this response.
  if (!app.state.channels[ch.id]) app.state = applyEvent(app.state, { t: 'ChannelCreated', d: { channel: ch } }, Date.now())
  return ch.id
}

/** Subscribe to the live connection. Returns an unsubscribe function. */
export async function startListening(): Promise<UnlistenFn> {
  const offs: UnlistenFn[] = await Promise.all([
    listen<Ready>('pulse://ready', (e) => (app.state = applyReady(app.state, e.payload))),
    listen<Event>('pulse://event', (e) => {
      maybeSound(e.payload)
      app.state = applyEvent(app.state, e.payload, Date.now())
    }),
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
