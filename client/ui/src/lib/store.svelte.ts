// Reactive wrapper around the pure reducer in state.ts, fed by Tauri events from the Rust core.
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'
import type { ReportKind } from './protocol/ReportKind'
import { applyEvent, applyReady, emptyState, restoreUnsent, setPendingStatus, type AppState, type ConnState } from './state'
import { api } from './tauri'
import { loadUnsent, saveUnsent } from './unsent'
import { isMuted, messageSound } from './conversations'
import { noteReactionEvent } from './reactions'
import { playSound } from './voiceui'
import { streamApi } from './stream/stream.svelte'
import { windowFocus } from './windowFocus.svelte'

// The reducer (state.ts) is pure and never mutates: hold its output as-is (`$state.raw`) instead
// of deep-proxying it. Reassigning `app.state` is the only way to change it, and the only way that
// re-renders; unchanged slices keep their identity, so deriveds over them stop there.
class AppStore {
  state = $state.raw<AppState>(emptyState())
  /** When the last connection state arrived (ms): the start of a "retrying in Ns" countdown. */
  connAt = $state(0)
}
export const app = new AppStore()

/** What the user is looking at: drives the message sound and read marking. */
export const ui = $state({
  openChannelId: null as string | null,
  autoReported: false,
  /** Last session's crash report, held because automatic sending is off: ask Send / Don't send. */
  heldReport: null as ReportKind | null,
})

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
    focused: windowFocus.focused,
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

// Unsent messages: saved whenever the pending map changes, and restored (failed, never resent) on
// the first Ready for each signed-in user. Saving waits for `me`, so a not-yet-restored empty map
// never overwrites what an earlier run left.
let restoredFor: string | null = null
$effect.root(() => {
  $effect(() => {
    const me = app.state.me
    const pending = app.state.pending
    if (me && restoredFor === me.id) saveUnsent(me.id, pending)
  })
})

/** The core's (re)connected snapshot. */
export function handleReady(r: Ready) {
  const before = app.state.streams
  // A different user than the one on screen: nothing of theirs carries over (no sign-out was seen).
  const prev = app.state.me
  let next = applyReady(prev && prev.id !== r.me.id ? { ...app.state, pending: {} } : app.state, r)
  if (restoredFor !== r.me.id) {
    restoredFor = r.me.id
    next = restoreUnsent(next, loadUnsent(r.me.id))
  }
  app.state = next
  void streamApi.onReady(r, before)
}

/** Subscribe to the live connection. Returns an unsubscribe function. */
export async function startListening(): Promise<UnlistenFn> {
  const offs: UnlistenFn[] = await Promise.all([
    listen<Ready>('pulse://ready', (e) => handleReady(e.payload)),
    listen<Event>('pulse://event', (e) => {
      maybeSound(e.payload)
      noteReactionEvent(e.payload, app.state.me?.id)
      app.state = applyEvent(app.state, e.payload, Date.now())
      streamApi.onEvent(e.payload)
    }),
    // LiveKit closed stream audio's connection under us
    listen<{ channel_id: string }>('stream://audio_closed', (e) => void streamApi.onAudioClosed(e.payload.channel_id)),
    listen<ConnState>('pulse://conn', (e) => {
      app.connAt = Date.now()
      app.state = { ...app.state, conn: e.payload }
    }),
    // Last session crashed or quit unexpectedly and its report just went out: tell the user once.
    listen('pulse://auto-report-sent', () => (ui.autoReported = true)),
    // ...or it was kept back (automatic sending is off): ask the user.
    listen<{ kind: ReportKind }>('pulse://crash-report-held', (e) => (ui.heldReport = e.payload.kind)),
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
  // The network is back: check the connection now, whatever state it's in (a half-open socket
  // still reads "connected"). Window focus is probed from Rust.
  const probe = () => void api.probe()
  window.addEventListener('online', probe)
  return () => {
    offs.forEach((off) => off())
    window.removeEventListener('online', probe)
  }
}

export function resetState() {
  restoredFor = null // the next Ready restores again
  app.state = emptyState()
  // a held crash report belongs to the session (and server) it was offered in: the next sign-in
  // asks again if the core still has it for that server
  ui.heldReport = null
}
