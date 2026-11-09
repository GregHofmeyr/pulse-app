// The picker wheel's client state (#158). Saved wheels are fetched when a popover or a Settings page opens (they
// aren't in Ready) and kept current by WheelSaved / WheelDeleted while loaded. Spins remember when they reached
// this client, so a line that mounts right then spins and one that mounts later (opening the chat late) is settled.
import { getContext, setContext } from 'svelte'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Wheel } from './protocol/Wheel'
import type { SpinWheelRequest } from './protocol/SpinWheelRequest'
import type { QuietSpin } from './protocol/QuietSpin'
import { applyEvent } from './state'
import { app, onAppEvent } from './store.svelte'
import { errorText, wheelCmd } from './tauri'
import { LIVE_WINDOW_MS, QUIET_CARD_MS, QUIET_LINGER_MS } from './wheel'

/** Loaded lists: a server's wheels by server id, and yours (null until fetched). */
export const wheels = $state<{ server: Record<string, Wheel[]>; mine: Wheel[] | null }>({ server: {}, mine: null })

const byName = (a: Wheel, b: Wheel) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }) || (a.id < b.id ? -1 : 1)
const sorted = (list: Wheel[]) => [...list].sort(byName)

export async function loadServerWheels(serverId: string): Promise<Wheel[]> {
  const list = sorted(await wheelCmd.listServer(serverId))
  wheels.server[serverId] = list
  return list
}

export async function loadMyWheels(): Promise<Wheel[]> {
  const list = sorted(await wheelCmd.listMine())
  wheels.mine = list
  return list
}

/** A wheel made or changed (here or anywhere): into whichever loaded list holds it. */
export function upsertWheel(w: Wheel): void {
  const put = (list: Wheel[]) => sorted([...list.filter((x) => x.id !== w.id), w])
  if (w.server_id) {
    const list = wheels.server[w.server_id]
    if (list) wheels.server[w.server_id] = put(list)
  } else if (wheels.mine) {
    wheels.mine = put(wheels.mine)
  }
}

export function dropWheel(id: string, serverId: string | null): void {
  if (serverId) {
    const list = wheels.server[serverId]
    if (list) wheels.server[serverId] = list.filter((w) => w.id !== id)
  } else if (wheels.mine) {
    wheels.mine = wheels.mine.filter((w) => w.id !== id)
  }
}

// a saved wheel's changes go out one at a time, in order: the last sent is the one that sticks
const chains = new Map<string, Promise<unknown>>()
const latest = new Map<string, number>()
let sent = 0

/** Change a saved wheel: shown at once, then saved behind its earlier changes. Only the answer to the newest change
 *  is shown (an older one would briefly undo it). A refusal fetches its list fresh. Resolves with the refusal's
 *  text, or '' when it saved. */
export function saveWheel(w: Wheel, change: { name?: string; options?: string[] }): Promise<string> {
  const n = ++sent
  latest.set(w.id, n)
  upsertWheel({ ...w, ...change })
  const run = (chains.get(w.id) ?? Promise.resolve()).then(async () => {
    try {
      const saved = await wheelCmd.update(w.id, change)
      if (latest.get(w.id) === n) upsertWheel(saved)
      return ''
    } catch (e) {
      await (w.server_id ? loadServerWheels(w.server_id) : loadMyWheels()).catch(() => {})
      return errorText(e)
    }
  })
  chains.set(w.id, run)
  return run
}

/** Every change still being saved has been (a spin of a saved wheel waits, so it spins what you see). */
export const savesSettled = (): Promise<unknown> => Promise.all(chains.values())

// message id → when its spin reached this client
const arrived = new Map<string, number>()

/** A spin's line reached this client now (its MessageCreated, or the spinner's own answer). */
export function noteSpin(m: Message, now = Date.now()): void {
  if (!m.wheel || arrived.has(m.id)) return
  arrived.set(m.id, now)
  // the map only needs the last few seconds
  for (const [id, at] of arrived) if (now - at > 60_000) arrived.delete(id)
}

/** Whether the line should spin as it mounts: its spin reached this client just now. */
export function spunLive(id: string, now = Date.now()): boolean {
  const at = arrived.get(id)
  return at !== undefined && now - at < LIVE_WINDOW_MS
}

// --- whether a chat is on screen (#271): a voice channel's chat panel can be open but hidden. Its spin cards are
// only seen, and only chime, while it shows. TextChannel provides it; elsewhere a chat counts as shown. ---
const CHAT_SHOWN = Symbol('wheelChatShown')
/** For TextChannel, as it starts: whether this chat is on screen now. */
export const provideChatShown = (shown: () => boolean): void => void setContext(CHAT_SHOWN, shown)
/** For a spin's line or card, as it starts: whether its chat is on screen (call the result when needed). */
export const chatShown = (): (() => boolean) => getContext<(() => boolean) | undefined>(CHAT_SHOWN) ?? (() => true)

// --- your own spins (#270): the chat you spun in jumps to the latest, as sending a message does ---
const ownSpinListeners = new Set<(channelId: string) => void>()

/** Hear about each spin you start (public or quiet), once it's on screen. Returns the unsubscribe. */
export function onOwnSpin(fn: (channelId: string) => void): () => void {
  ownSpinListeners.add(fn)
  return () => ownSpinListeners.delete(fn)
}

const ownSpin = (channelId: string) => {
  for (const fn of ownSpinListeners) fn(channelId)
}

/** Show your own spin's line as soon as the server answers (its MessageCreated, when it comes, dedupes by id). */
function show(line: Message): Message {
  noteSpin(line)
  app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: line, nonce: null } }, Date.now())
  ownSpin(line.channel_id)
  return line
}

/** Spin a saved wheel or a quick one in a chat. Rejects with the server's reason (too soon, too few options…). */
export const spinWheel = async (channelId: string, request: SpinWheelRequest): Promise<Message> =>
  show(await wheelCmd.spin(channelId, request))

/** Spin again from a spin's line, or without its winner. */
export const spinAgain = async (lineId: string, removeWinner: boolean): Promise<Message> =>
  show(await wheelCmd.spinAgain(lineId, removeWinner))

// --- quiet spins (#270): the chat watches one land, then its card goes; nothing is saved anywhere ---

export interface QuietCard {
  spin: QuietSpin
  /** When it reached this client (ms): a card that mounts later than LIVE_WINDOW_MS after shows it landed. */
  at: number
}

/** The quiet spins on show, by chat. */
export const quietSpins = $state<Record<string, QuietCard[]>>({})
const quietTimers = new Map<string, ReturnType<typeof setTimeout>>()

function expireQuiet(id: string, after: number) {
  clearTimeout(quietTimers.get(id))
  quietTimers.set(id, setTimeout(() => dropQuietSpin(id), after))
}

/** A quiet spin reached this client (its event, or our own answer): on show until QUIET_CARD_MS from now. */
export function addQuietSpin(spin: QuietSpin, now = Date.now()): void {
  const list = quietSpins[spin.channel_id] ?? []
  if (list.some((c) => c.spin.id === spin.id)) return
  quietSpins[spin.channel_id] = [...list, { spin, at: now }]
  expireQuiet(spin.id, QUIET_CARD_MS)
}

export function dropQuietSpin(id: string): void {
  clearTimeout(quietTimers.get(id))
  quietTimers.delete(id)
  for (const [channel, list] of Object.entries(quietSpins)) {
    if (!list.some((c) => c.spin.id === id)) continue
    const left = list.filter((c) => c.spin.id !== id)
    if (left.length) quietSpins[channel] = left
    else delete quietSpins[channel]
  }
}

/** While a card is pointed at (or has focus) it stays; let go, it lingers QUIET_LINGER_MS more, then goes. */
export function holdQuietSpin(id: string, held: boolean): void {
  if (!quietTimers.has(id)) return
  if (held) clearTimeout(quietTimers.get(id))
  else expireQuiet(id, QUIET_LINGER_MS)
}

/** Spin quietly in a chat: everyone there watches it land, nothing is saved. Rejects with the server's reason. */
export async function spinQuietly(channelId: string, request: SpinWheelRequest): Promise<QuietSpin> {
  const q = await wheelCmd.spinQuietly(channelId, request)
  addQuietSpin(q)
  ownSpin(q.channel_id)
  return q
}

// "Spin quietly", per device (#270): the popover's checkbox, which `/wheel a, b` follows too.
const QUIET_KEY = 'pulse.wheelSpinQuietly'
function savedQuiet(): boolean {
  try {
    return localStorage.getItem(QUIET_KEY) === 'true'
  } catch {
    return false
  }
}
export const wheelPrefs = $state({ quiet: savedQuiet() })
export function setSpinQuietly(on: boolean): void {
  wheelPrefs.quiet = on
  try {
    localStorage.setItem(QUIET_KEY, String(on))
  } catch {
    /* ignore */
  }
}

/** A quick wheel from `/wheel a, b, c`: quiet or not, as the popover's checkbox was last left. */
export const spinQuick = (channelId: string, options: string[]): Promise<unknown> =>
  wheelPrefs.quiet ? spinQuietly(channelId, { options }) : spinWheel(channelId, { options })

export function handleWheelEvent(e: Event): void {
  if (e.t === 'MessageCreated' && e.d.message.wheel) noteSpin(e.d.message)
  else if (e.t === 'WheelSaved') upsertWheel(e.d.wheel)
  else if (e.t === 'WheelDeleted') dropWheel(e.d.wheel_id, e.d.server_id)
  else if (e.t === 'WheelSpunQuietly') addQuietSpin(e.d.spin)
}

onAppEvent(handleWheelEvent)

/** For tests: forget everything. */
export function resetWheels(): void {
  wheels.server = {}
  wheels.mine = null
  arrived.clear()
  chains.clear()
  latest.clear()
  for (const t of quietTimers.values()) clearTimeout(t)
  quietTimers.clear()
  for (const k of Object.keys(quietSpins)) delete quietSpins[k]
  ownSpinListeners.clear()
}
