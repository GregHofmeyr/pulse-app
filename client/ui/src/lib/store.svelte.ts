// Reactive wrapper around the pure reducer in state.ts, fed by Tauri events from the Rust core.
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'
import type { ReportKind } from './protocol/ReportKind'
import { applyEvent, applyReady, emptyState, restoreUnsent, setPendingStatus, type AppState, type ConnState } from './state'
import { api } from './tauri'
import { loadUnsent, saveUnsent } from './unsent'
import { isMuted, messageSound, onScreen, SOUND_GAP_MS } from './conversations'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { noteReactionEvent } from './reactions'
import { loadSoundPrefs, playSound } from './voiceui'
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

/** A request to show a message (lib/jump.ts): the open channel scrolls to it, flashes it, then clears
 * this. `seq` makes a repeat of the same target a new request; `missing`: it couldn't be found. */
export type JumpRequest = { channelId: string; messageId: string; seq: number; missing?: boolean; error?: string }

/** What the user is looking at: drives the message sound and read marking. */
export const ui = $state({
  openChannelId: null as string | null,
  /** The quote whose replies panel is open (its replies are on screen). */
  openThreadId: null as string | null,
  /** A request to switch to this channel (the shell does it and clears it). */
  goTo: null as string | null,
  jump: null as JumpRequest | null,
  autoReported: false,
  /** Last session's crash report, held because automatic sending is off: ask Send / Don't send. */
  heldReport: null as ReportKind | null,
})

/** One shared clock for time-dependent UI (mute expiry, "5m ago"). Ticks every 30 s. */
export const clock = $state({ nowIso: new Date().toISOString() })
setInterval(() => (clock.nowIso = new Date().toISOString()), 30_000)
let lastPlayedAt = 0

/**
 * For a message that could ding (someone else's, in a DM/group or mentioning you), one line saying
 * why it did or didn't: only flags, never who, where or what. Skipped because you're reading that
 * conversation in the focused window (every message while you chat): only if the OS says the
 * window isn't focused after all, which is the case worth knowing about.
 */
function logSoundDecision(v: Parameters<typeof messageSound>[0], play: boolean) {
  const m = v.message
  const isPrivate = v.channel.server_id === null
  const mentioned = m.mentions.includes(v.me)
  if (m.kind !== 'normal' || m.author_id === v.me || !(isPrivate || mentioned)) return
  const gapOk = v.now - v.lastPlayedAt >= SOUND_GAP_MS
  const p = loadSoundPrefs()
  // switched off in settings looks nothing like a sound that got lost
  const prefs = p.volume_pct === 0 ? 'vol0' : p.off.includes('message') ? 'off' : 'on'
  const line = `sound: message private=${isPrivate} mentioned=${mentioned} open=${v.open} focused=${v.focused} muted=${v.muted} gap_ok=${gapOk} sound=${prefs} play=${play}`
  if (play || !onScreen(v) || !v.focused) return void api.uiLog(line, 'info')
  getCurrentWindow().isFocused().then(
    (f) => {
      if (!f) void api.uiLog(`${line}; the OS says focused=false`, 'info')
    },
    () => {},
  )
}

function maybeSound(e: Event) {
  if (e.t !== 'MessageCreated') return
  const m = e.d.message
  const ch = app.state.channels[m.channel_id]
  const me = app.state.me
  if (!ch || !me) return
  const now = Date.now()
  const v = {
    message: m,
    me: me.id,
    channel: ch,
    open: ui.openChannelId === m.channel_id,
    openThread: ui.openThreadId,
    focused: windowFocus.focused,
    muted: isMuted(app.state, ch, new Date(now).toISOString()),
    lastPlayedAt,
    now,
  }
  const play = messageSound(v)
  try {
    logSoundDecision(v, play)
  } catch {
    /* a diagnostic never costs the message */
  }
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

/** Called with each live event after it is applied. Modules that react to events register here
 *  (this store can't import them: they import it). */
const eventHooks = new Set<(e: Event) => void>()
/** Register `hook`; returns its unregister function. */
export function onAppEvent(hook: (e: Event) => void): () => void {
  eventHooks.add(hook)
  return () => void eventHooks.delete(hook)
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
      for (const hook of eventHooks) hook(e.payload)
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
