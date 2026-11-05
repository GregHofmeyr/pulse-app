// Calls in DMs and groups (#50): the one store for rings. The popover (outside the conversation), the band (inside
// it) and the ringtone all read it; the calls themselves live in app state (`state.ts`, `calls`).
//
// The server decides who rings and when a ring ends (answered or declined on any of your devices, timed out, call
// ended); this store follows it, and also drops a ring at its expiry in case that event was missed. Whether a ring may
// grab your attention (popover, ringtone, OS notification) is `ringAlerts`: never under Do Not Disturb or in a muted
// conversation. The band and the call line show either way.
import { app, clock, onAppEvent, onAppReady, openDm, ui } from './store.svelte'
import { api } from './tauri'
import { voiceApi } from './voice.svelte'
import { ringAlerts, ringPlace } from './calls'
import { conversationName } from './conversations'
import { displayName, userOf } from './selectors'
import { loadMixer, ringtoneGain } from './mixer'
import { loadAudioConfig, loadSoundPrefs } from './voiceui'
import { notifyPrefs } from './notifyPrefs.svelte'
import { windowFocus } from './windowFocus.svelte'
import type { CallRing } from './protocol/CallRing'
import type { Event } from './protocol/Event'
import type { Ready } from './protocol/Ready'

export const calls = $state({ rings: [] as CallRing[] })

const expiry = new Map<string, ReturnType<typeof setTimeout>>()

function drop(pred: (r: CallRing) => boolean) {
  const gone = calls.rings.filter(pred)
  if (!gone.length) return
  for (const r of gone) {
    clearTimeout(expiry.get(r.call_id))
    expiry.delete(r.call_id)
  }
  calls.rings = calls.rings.filter((r) => !pred(r))
}

function add(ring: CallRing) {
  drop((r) => r.channel_id === ring.channel_id || r.call_id === ring.call_id)
  const left = Date.parse(ring.expires_at) - Date.now()
  if (!(left > 0)) return
  calls.rings = [...calls.rings, ring]
  // the server ends it too; this only covers a missed CallRingEnded (a dropped socket)
  expiry.set(ring.call_id, setTimeout(() => drop((r) => r.call_id === ring.call_id), left))
}

/** The ring for `channelId`, if you're being rung there (the band shows it, alerting or not). */
export function ringIn(channelId: string): CallRing | null {
  return calls.rings.find((r) => r.channel_id === channelId) ?? null
}

/** Rings shown as a popover: outside their conversation, and allowed to alert. */
export function popoverRings(): CallRing[] {
  const now = clock.nowIso
  return calls.rings.filter((r) => ringPlace(r, ui.openChannelId) === 'popover' && ringAlerts(app.state, r, now))
}

/** "sam is calling you": the desktop notification for a ring, while the window isn't focused. */
function notifyRing(ring: CallRing) {
  if (!notifyPrefs.enabled || windowFocus.focused) return
  const s = app.state
  const caller = displayName(s, null, ring.caller_id)
  const group = s.channels[ring.channel_id]?.kind === 'group'
  const u = userOf(s, ring.caller_id)
  const avatar = u?.avatar_still ?? u?.avatar_hash
  void api.notifyMessage({
    title: `${caller} is calling you`,
    body: group ? `Call in ${conversationName(s, ring.channel_id)}` : 'Incoming call',
    channelId: ring.channel_id,
    ...(avatar ? { avatar } : {}),
  })
}

export function handleCallEvent(e: Event) {
  const me = app.state.me?.id
  switch (e.t) {
    case 'CallRinging': {
      if (e.d.ring.caller_id === me) return
      add(e.d.ring)
      if (ringAlerts(app.state, e.d.ring, new Date().toISOString())) notifyRing(e.d.ring)
      return
    }
    case 'CallRingEnded':
      if (e.d.user_id === me) drop((r) => r.call_id === e.d.call_id)
      return
    case 'CallEnded':
      drop((r) => r.call_id === e.d.call_id)
      return
    case 'ChannelRemoved':
      if (e.d.user_id === me) drop((r) => r.channel_id === e.d.channel_id)
      return
  }
}

export function handleCallsReady(r: Ready) {
  drop(() => true)
  for (const ring of r.ringing ?? []) add(ring)
}

/** Answer: stop ringing here at once, go to the conversation and join its call (leaving any other voice). */
export function joinCall(channelId: string) {
  drop((r) => r.channel_id === channelId)
  ui.goTo = channelId
  return voiceApi.join(channelId)
}

/** Decline: stop ringing on every device of yours. The caller is never told. */
export async function declineCall(channelId: string) {
  drop((r) => r.channel_id === channelId)
  await api.declineCall(channelId)
}

/** The phone button (and the people board's Call): start the conversation's call, or join the one going on, then
 *  connect. Joining moves you out of any other voice channel or call. */
export async function startCall(channelId: string) {
  drop((r) => r.channel_id === channelId)
  await api.startCall(channelId)
  await voiceApi.join(channelId)
}

/** Call someone from the people board: open (or reuse) your DM with them, show it, and start its call. */
export async function callUser(userId: string) {
  const channelId = await openDm(userId)
  ui.goTo = channelId
  await startCall(channelId)
}

// The ringtone plays while any ring may alert; DND, a mute or the last ring ending stops it.
let ringtoneOn = false
$effect.root(() => {
  // signed out: nothing of that account rings on
  $effect(() => {
    if (!app.state.me && calls.rings.length) drop(() => true)
  })
  $effect(() => {
    void clock.nowIso // a DND or mute that runs out lets a ring sound again
    const on = calls.rings.some((r) => ringAlerts(app.state, r, new Date().toISOString()))
    if (on === ringtoneOn) return
    ringtoneOn = on
    if (on) void api.startRingtone(ringtoneGain(loadMixer()), loadAudioConfig().output ?? null, loadSoundPrefs().ringtone)
    else void api.stopRingtone()
  })
})

onAppEvent(handleCallEvent)
onAppReady(handleCallsReady)

/** Signed out: no rings, no ringtone. */
export function resetCalls() {
  drop(() => true)
}
