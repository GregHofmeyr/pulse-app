// Speaking rings (#651 §8.4): a dev-build-only stress harness for the performance check. In the devtools console of
// a dev build, in a call: `const stop = __ringStress(8, 'bars')` adds 8 fake people to the stage, all speaking with
// that ring style, fed a synthetic level every 70 ms (the core's tick); `stop()` takes them away again. main.ts loads
// this only under import.meta.env.DEV, so release builds never ship it.
import { apply } from './ringLevels.svelte'
import type { RingStyle } from './speakingRing'
import { app, ui } from './store.svelte'
import { voice } from './voice.svelte'

const TICK_MS = 70
const FAKE_LIST = 'ring-stress'

/** A cheap speech-like walk: drifts, jumps now and then, rests at a low level sometimes. */
function step(v: number): number {
  const r = Math.random()
  if (r < 0.06) return 0.1
  if (r < 0.14) return Math.random()
  return Math.min(1, Math.max(0, v + (Math.random() - 0.5) * 0.3))
}

const round = (v: number) => Math.round(v * 20) / 20

function stress(n: number, style: RingStyle): () => void {
  const channelId = voice.channelId ?? ui.openChannelId
  if (!channelId) throw new Error('__ringStress: join a call first')
  const ids = Array.from({ length: n }, (_, i) => `ring-stress-${i + 1}`)
  const s = app.state
  // the fake people live in a members list of their own, so userOf finds them and nothing else is touched
  const fakes = ids.map((id, i) => ({ user: { id, username: `stress${i + 1}`, display_name: `Stress ${i + 1}`, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] }, ring_style: style }, nickname: null }))
  const inCall = [...(s.voice[channelId] ?? []), ...ids.map((user_id) => ({ user_id, flags: { muted: false, deafened: false } }))]
  app.state = { ...s, members: { ...s.members, [FAKE_LIST]: fakes }, voice: { ...s.voice, [channelId]: inCall } }
  voice.speaking = new Set([...voice.speaking, ...ids])

  const lvl = new Map(ids.map((id) => [id, Math.random()]))
  const timer = setInterval(() => {
    const frame: Record<string, number> = {}
    for (const id of ids) {
      const v = step(lvl.get(id)!)
      lvl.set(id, v)
      frame[id] = round(v)
    }
    apply(frame)
  }, TICK_MS)

  let stopped = false
  return () => {
    if (stopped) return
    stopped = true
    clearInterval(timer)
    apply({})
    const fake = new Set(ids)
    const now = app.state
    const members = { ...now.members }
    delete members[FAKE_LIST]
    const voiceNow = { ...now.voice }
    if (voiceNow[channelId]) voiceNow[channelId] = voiceNow[channelId].filter((m) => !fake.has(m.user_id))
    app.state = { ...now, members, voice: voiceNow }
    voice.speaking = new Set([...voice.speaking].filter((id) => !fake.has(id)))
  }
}

/** Puts `__ringStress(n, style)` on window (dev builds only). */
export function install(): void {
  ;(window as unknown as { __ringStress: typeof stress }).__ringStress = stress
}
