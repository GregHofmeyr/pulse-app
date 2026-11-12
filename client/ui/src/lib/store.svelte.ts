// Reactive wrapper around the pure reducer in state.ts, fed by Tauri events from the Rust core.
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Gif } from './protocol/Gif'
import type { CardProfile } from './protocol/CardProfile'
import type { Profile } from './protocol/Profile'
import type { User } from './protocol/User'
import type { Ready } from './protocol/Ready'
import type { ReportKind } from './protocol/ReportKind'
import { addPending, applyEvent, applyReady, emptyState, restoreUnsent, setPendingStatus, setProfile, type AppState, type ConnState, type Pending, type PendingFile } from './state'
import { attachmentSends } from './attachmentSends.svelte'
import { api, type AttachmentProgress, type StagedFile } from './tauri'
import { loadUnsent, saveUnsent } from './unsent'
import { isMuted, messageSound, notifyLevel, onScreen, SOUND_GAP_MS } from './conversations'
import { dndActive } from './status'
import { notifyMessage, NotifyThrottle } from './notify'
import { notifyPrefs } from './notifyPrefs.svelte'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { noteReactionEvent } from './reactions'
import { playSound } from './voiceui'
import { loadMixer, soundGain } from './mixer'
import { streamApi } from './stream/stream.svelte'
import { windowFocus } from './windowFocus.svelte'
import { forgetEmbedOrigin } from './linkPreviews.svelte'
import { forgetClipLinks } from './clipLinks'
import { forgetAttachmentLinks } from './attachmentLinks'

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
  /** A request to open this quote's replies panel (its channel's view does it and clears it). */
  openReplies: null as { channelId: string; rootId: string } | null,
  /** A request to open this clip's panel (its clips channel's view does it and clears it): a clips
   *  channel has no history to jump in. */
  openClip: null as { channelId: string; clip: Message } | null,
  jump: null as JumpRequest | null,
  /** A request to open this chat's pins popover (a pin line's See pins, #178; its channel's view does it and clears it). */
  openPins: null as string | null,
  /** The quick switcher's global keybind fired (#319; the core already brought the window up): the shell opens the
   *  switcher, or closes it if Pulse was in front with it open, and clears this. */
  quickSwitch: null as { wasFocused: boolean } | null,
  /** A short line for the shell to show for a moment (it shows it and clears it). */
  notice: null as string | null,
  autoReported: false,
  /** Last session's crash report, held because automatic sending is off: ask Send / Don't send. */
  heldReport: null as ReportKind | null,
})

/** One shared clock for time-dependent UI (mute expiry, "5m ago"). Ticks every 30 s. */
export const clock = $state({ nowIso: new Date().toISOString() })
setInterval(() => (clock.nowIso = new Date().toISOString()), 30_000)
let lastPlayedAt = 0

/**
 * For a message that could ding (someone else's, in a DM/group, mentioning you, or in a server channel on All
 * messages), one line saying
 * why it did or didn't: only flags, never who, where or what. Skipped because you're reading that
 * conversation in the focused window (every message while you chat): only if the OS says the
 * window isn't focused after all, which is the case worth knowing about.
 */
function logSoundDecision(v: Parameters<typeof messageSound>[0], play: boolean) {
  const m = v.message
  const isPrivate = v.channel.server_id === null
  const mentioned = m.mentions.includes(v.me)
  if (m.kind !== 'normal' || m.author_id === v.me || !(isPrivate || mentioned || v.level === 'all')) return
  const gapOk = v.now - v.lastPlayedAt >= SOUND_GAP_MS
  // turned to 0 % in the volume mixer looks nothing like a sound that got lost
  const prefs = soundGain(loadMixer(), 'message') > 0 ? 'on' : 'off'
  const line = `sound: message private=${isPrivate} mentioned=${mentioned} open=${v.open} thread=${m.thread_root_id ?? '-'} open_thread=${v.openThread ?? '-'} focused=${v.focused} muted=${v.muted} level=${isPrivate ? '-' : v.level} gap_ok=${gapOk} sound=${prefs} play=${play}`
  if (play || !onScreen(v) || !v.focused) return void api.uiLog(line, 'info')
  getCurrentWindow().isFocused().then(
    (f) => {
      if (!f) void api.uiLog(`${line}; the OS says focused=false`, 'info')
    },
    () => {},
  )
}

/** One desktop notification per conversation per few seconds (lib/notify.ts). */
const notifications = new NotifyThrottle((notice) => void api.notifyMessage(notice))

/** A new message: the message sound, and a desktop notification while the window isn't focused. */
export function maybeAlert(e: Event) {
  if (e.t === 'MessageUpdated' && e.d.message.deleted) return notifications.forget(e.d.message.channel_id, e.d.message.id)
  if (e.t !== 'MessageCreated') return
  const m = e.d.message
  const ch = app.state.channels[m.channel_id]
  const me = app.state.me
  if (!ch || !me) return
  const now = Date.now()
  const nowIso = new Date(now).toISOString()
  const v = {
    message: m,
    me: me.id,
    channel: ch,
    open: ui.openChannelId === m.channel_id,
    openThread: ui.openThreadId,
    focused: windowFocus.focused,
    muted: isMuted(app.state, ch, nowIso),
    level: notifyLevel(app.state, ch),
    // the real time, not the clock: a DND timer ends exactly when it ends
    dnd: dndActive(app.state, nowIso),
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
  const s = app.state
  const notice = notifyMessage(s, v, notifyPrefs)
  if (notice) notifications.offer(m.channel_id, m.id, (more) => (more ? (notifyMessage(s, v, notifyPrefs, more) ?? notice) : notice))
}

/** A clicked notification (the core has brought the window up): open its conversation, and a
 *  reply's replies panel. Gone, or signed out since: the window coming up is all. */
export function handleNotificationClick(c: { channelId: string; threadId: string | null }) {
  const ch = app.state.channels[c.channelId]
  if (!ch || !app.state.me) return
  ui.goTo = c.channelId
  if (c.threadId && ch.kind === 'quotes') ui.openReplies = { channelId: c.channelId, rootId: c.threadId }
}

/** Open (or reuse) your DM with `userId`; resolves to its channel id. */
export async function openDm(userId: string): Promise<string> {
  const ch = await api.createDm([userId])
  // Show it straight away: the ChannelCreated gateway event may arrive after this response.
  if (!app.state.channels[ch.id]) app.state = applyEvent(app.state, { t: 'ChannelCreated', d: { channel: ch } }, Date.now())
  // and who's in it (a DM: you and them; your notes, #308: just you), which GroupMembersChanged may also bring late
  const me = app.state.me?.id
  if (me && ch.kind === 'dm' && !app.state.dmMembers[ch.id]) {
    app.state = applyEvent(app.state, { t: 'GroupMembersChanged', d: { channel_id: ch.id, user_ids: [...new Set([me, userId])].sort() } }, Date.now())
  }
  return ch.id
}

/** Queue `text` for `channelId`: shown as pending straight away and sent through the core's outbox (retries, failed state).
 *  With `files` (#57) the core uploads them first, then sends (its outcome arrives on the same `pulse://outbox`). */
export function queueMessage(channelId: string, text: string, replyToId: string | null, gif: Gif | null = null, files: readonly StagedFile[] = [], tempDays: number | null = null): void {
  const nonce = crypto.randomUUID()
  const pendingFiles: PendingFile[] = files.map((f) => ({ id: f.id, name: f.name, size: f.size, kind: f.kind, preview: f.preview }))
  app.state = addPending(app.state, channelId, {
    nonce, content: text, reply_to_id: replyToId, thread_root_id: null, status: 'pending', created_at: new Date().toISOString(),
    ...(gif ? { gif } : {}),
    ...(pendingFiles.length ? { files: pendingFiles, temp_days: tempDays } : {}),
  })
  if (pendingFiles.length) sendFiles(channelId, nonce, text, replyToId, pendingFiles, tempDays)
  else void api.sendMessage(channelId, text, replyToId, nonce, gif)
}

function sendFiles(channelId: string, nonce: string, text: string, replyToId: string | null, files: PendingFile[], tempDays: number | null) {
  attachmentSends.start(nonce)
  api
    .sendWithAttachments({ channelId, content: text, replyToId, threadRootId: null, nonce, staged: files.map((f) => f.id), tempDays })
    .catch(() => (app.state = setPendingStatus(app.state, channelId, nonce, 'failed')))
}

/** Try a failed message with files again under its own nonce: what was already uploaded isn't uploaded again. */
export function retryFiles(channelId: string, p: Pending): void {
  if (!p.files?.length) return
  app.state = setPendingStatus(app.state, channelId, p.nonce, 'pending')
  sendFiles(channelId, p.nonce, p.content, p.reply_to_id, p.files, p.temp_days ?? null)
}

/** Give up on a pending message with files: the upload stops and the queued files are let go. */
export function discardFiles(p: Pending): void {
  if (!p.files?.length) return
  attachmentSends.forget(p.nonce)
  void api.cancelAttachmentSend(p.nonce)
  for (const f of p.files) void api.unstageAttachment(f.id)
}

// Unsent messages: saved whenever the pending map changes, and restored (failed, never resent) on
// the first Ready for each signed-in user. Saving waits for `me`, so a not-yet-restored empty map
// never overwrites what an earlier run left.
let restoredFor: string | null = null
$effect.root(() => {
  // Back at the window, or notifications switched off: no "and N more" still to come.
  $effect(() => {
    if (windowFocus.focused || !notifyPrefs.enabled) notifications.clear()
  })
  // A timed DND ends on time (#45): the clock moves to its end, so your dot flips back to Online then, without
  // waiting for the 30 s tick or the server (which sweeps it within ~15 s). A new status replaces the timer.
  $effect(() => {
    const { status, dnd_until } = app.state.myStatus
    if (status !== 'dnd' || !dnd_until) return
    const end = Date.parse(dnd_until)
    // setTimeout holds at most ~24.8 days (the server caps a DND at 7)
    const t = setTimeout(() => (clock.nowIso = new Date(Math.max(Date.now(), end)).toISOString()), Math.min(Math.max(0, end - Date.now()), 2 ** 31 - 1))
    return () => clearTimeout(t)
  })
  $effect(() => {
    const me = app.state.me
    const pending = app.state.pending
    if (me && restoredFor === me.id) saveUnsent(me.id, pending)
  })
})

const profileLoads = new Map<string, Promise<Profile>>()
/** The newest ProfileUpdated seen per user while that user's load is in flight. */
const updatedDuringLoad = new Map<string, { user: User; card: CardProfile }>()
/** When each cached profile was last fetched (ms), for background revalidation. */
const profileFetchedAt = new Map<string, number>()
/** A cached profile older than this is refetched in the background when it is opened. */
export const PROFILE_STALE_MS = 5 * 60_000
/** Bumped by resetState and on every Ready: a load started before it must not write into the new state. */
let loadEpoch = 0

/** Forget every profile load in flight (their answers are dropped) and every fetch time. */
function forgetProfileLoads() {
  loadEpoch++
  profileLoads.clear()
  updatedDuringLoad.clear()
  profileFetchedAt.clear()
}

/** Remember a ProfileUpdated that lands while the user's profile is loading, so the older response can't undo it. */
export function noteProfileUpdate(e: Event) {
  if (e.t === 'ProfileUpdated' && profileLoads.has(e.d.user.id)) updatedDuringLoad.set(e.d.user.id, e.d)
}

/** Fetch `userId`'s profile into the cache; calls made while one is in flight share its request. */
export function loadProfile(userId: string): Promise<Profile> {
  const inflight = profileLoads.get(userId)
  if (inflight) return inflight
  const epoch = loadEpoch
  const p = api.getProfile(userId).then((fetched) => {
    if (epoch !== loadEpoch) return fetched
    const newer = updatedDuringLoad.get(userId)
    const profile = newer ? { ...fetched, user: newer.user, card: newer.card } : fetched
    app.state = setProfile(app.state, profile)
    profileFetchedAt.set(userId, Date.now())
    return profile
  })
  profileLoads.set(userId, p)
  // settled either way: the next call fetches again (so a failure isn't stuck)
  const done = () => {
    if (epoch !== loadEpoch) return
    profileLoads.delete(userId)
    updatedDuringLoad.delete(userId)
  }
  p.then(done, done)
  return p
}

/** On opening a cached profile: refetch it in the background if it was fetched over
 * [PROFILE_STALE_MS] ago, so stats don't wait for a reconnect. The cached copy stays on screen
 * meanwhile; a failure keeps it. Shares any load in flight. A copy cached some other way (the
 * editor's save answer) is as fresh as now. */
export function revalidateProfile(userId: string) {
  if (!app.state.profiles[userId]) return
  const at = profileFetchedAt.get(userId)
  if (at === undefined) {
    profileFetchedAt.set(userId, Date.now())
    return
  }
  if (Date.now() - at < PROFILE_STALE_MS) return
  loadProfile(userId).catch(() => {})
}

/** The core's (re)connected snapshot. */
export function handleReady(r: Ready) {
  // Ready clears the profile cache: a load from before it must not refill it with older data.
  forgetProfileLoads()
  const before = app.state.streams
  // A different user than the one on screen: nothing of theirs carries over (no sign-out was seen).
  const prev = app.state.me
  if (prev && prev.id !== r.me.id) {
    forgetClipLinks()
    forgetAttachmentLinks()
  }
  let next = applyReady(prev && prev.id !== r.me.id ? { ...app.state, pending: {} } : app.state, r)
  if (restoredFor !== r.me.id) {
    restoredFor = r.me.id
    next = restoreUnsent(next, loadUnsent(r.me.id))
  }
  app.state = next
  void streamApi.onReady(r, before)
  for (const hook of readyHooks) hook(r)
}

/** Called with each Ready after it is applied (the call store's rings, #50), like [onAppEvent]. */
const readyHooks = new Set<(r: Ready) => void>()
export function onAppReady(hook: (r: Ready) => void): () => void {
  readyHooks.add(hook)
  return () => void readyHooks.delete(hook)
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
      maybeAlert(e.payload)
      noteProfileUpdate(e.payload)
      noteReactionEvent(e.payload, app.state.me?.id)
      app.state = applyEvent(app.state, e.payload, Date.now())
      streamApi.onEvent(e.payload)
      for (const hook of eventHooks) hook(e.payload)
    }),
    // LiveKit closed stream audio's connection under us
    listen<{ channel_id: string }>('stream://audio_closed', (e) => void streamApi.onAudioClosed(e.payload.channel_id)),
    listen<{ channelId: string; threadId: string | null }>('pulse://notification-click', (e) => handleNotificationClick(e.payload)),
    listen<{ wasFocused: boolean }>('pulse://quick-switch', (e) => (ui.quickSwitch = { wasFocused: e.payload.wasFocused })),
    listen<ConnState>('pulse://conn', (e) => {
      app.connAt = Date.now()
      app.state = { ...app.state, conn: e.payload }
    }),
    // Last session crashed or quit unexpectedly and its report just went out: tell the user once.
    listen('pulse://auto-report-sent', () => (ui.autoReported = true)),
    // ...or it was kept back (automatic sending is off): ask the user.
    listen<{ kind: ReportKind }>('pulse://crash-report-held', (e) => (ui.heldReport = e.payload.kind)),
    listen<AttachmentProgress>('attachments://progress', (e) => attachmentSends.progress(e.payload)),
    listen<{ nonce: string; error: string }>('attachments://failed', (e) => attachmentSends.failed(e.payload.nonce, e.payload.error)),
    listen<{ nonce: string; status: 'sent' | 'failed'; message: Message | null }>('pulse://outbox', (e) => {
      const { nonce, status, message } = e.payload
      if (status === 'sent') attachmentSends.forget(nonce)
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
  notifications.clear() // signed out: nothing more about the last account's messages
  ui.openReplies = null
  ui.openClip = null
  forgetProfileLoads()
  forgetEmbedOrigin() // the next account may be on another server
  forgetClipLinks() // the next account asks for its own play links
  forgetAttachmentLinks()
  app.state = emptyState()
  // a held crash report belongs to the session (and server) it was offered in: the next sign-in
  // asks again if the core still has it for that server
  ui.heldReport = null
}
