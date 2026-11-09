// The picker wheel's client state (#158). Saved wheels are fetched when a popover or a Settings page opens (they
// aren't in Ready) and kept current by WheelSaved / WheelDeleted while loaded. Spins remember when they reached
// this client, so a line that mounts right then spins and one that mounts later (opening the chat late) is settled.
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Wheel } from './protocol/Wheel'
import type { SpinWheelRequest } from './protocol/SpinWheelRequest'
import { applyEvent } from './state'
import { app, onAppEvent } from './store.svelte'
import { wheelCmd } from './tauri'
import { LIVE_WINDOW_MS } from './wheel'

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

/** Show your own spin's line as soon as the server answers (its MessageCreated, when it comes, dedupes by id). */
function show(line: Message): Message {
  noteSpin(line)
  app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: line, nonce: null } }, Date.now())
  return line
}

/** Spin a saved wheel or a quick one in a chat. Rejects with the server's reason (too soon, too few options…). */
export const spinWheel = async (channelId: string, request: SpinWheelRequest): Promise<Message> =>
  show(await wheelCmd.spin(channelId, request))

/** Spin again from a spin's line, or without its winner. */
export const spinAgain = async (lineId: string, removeWinner: boolean): Promise<Message> =>
  show(await wheelCmd.spinAgain(lineId, removeWinner))

export function handleWheelEvent(e: Event): void {
  if (e.t === 'MessageCreated' && e.d.message.wheel) noteSpin(e.d.message)
  else if (e.t === 'WheelSaved') upsertWheel(e.d.wheel)
  else if (e.t === 'WheelDeleted') dropWheel(e.d.wheel_id, e.d.server_id)
}

onAppEvent(handleWheelEvent)

/** For tests: forget everything. */
export function resetWheels(): void {
  wheels.server = {}
  wheels.mine = null
  arrived.clear()
}
