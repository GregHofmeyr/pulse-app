// F1 race weekends' client side (#386). A weekend card's predictions are the same for everyone, so their
// `F1PredictionsChanged` event carries them whole and the reducer puts them in place (lib/state.ts); a pick answers
// with them too. Here: picking, and each server's F1 status (its channel, the next weekend for the countdown banner,
// the season's table), asked of the server once and again every few minutes while a banner shows it.
import type { F1Prediction } from './protocol/F1Prediction'
import type { F1Status } from './protocol/F1Status'
import type { Message } from './protocol/Message'
import { setF1Predictions } from './state'
import { app } from './store.svelte'
import { f1Cmd } from './tauri'

export { f1Prefs, setF1RaceWeekends } from './f1Prefs.svelte'

/** How old a server's status may get before a banner asks again. */
export const STATUS_TTL_MS = 10 * 60_000

/** Your podium (three driver ids, P1 first), or none to take it back. Rejects with the server's reason. */
export async function predictPodium(m: Message, picks: string[] | null): Promise<F1Prediction[]> {
  const predictions = await f1Cmd.predict(m.id, picks)
  app.state = setF1Predictions(app.state, m.channel_id, m.id, predictions)
  return predictions
}

/** Each server's F1 status as last fetched. */
export const f1Status = $state({ byServer: {} as Record<string, F1Status> })
const fetchedAt = new Map<string, number>()
const inFlight = new Map<string, Promise<void>>()

/** Ask for a server's status unless a fresh one is held (`force`: ask anyway, say after picking a channel). */
export function loadF1Status(serverId: string, force = false, now = Date.now()): Promise<void> {
  const at = fetchedAt.get(serverId)
  if (!force && at !== undefined && now - at < STATUS_TTL_MS) return Promise.resolve()
  const running = inFlight.get(serverId)
  if (running && !force) return running
  const go = f1Cmd.status(serverId).then(
    (st) => {
      f1Status.byServer[serverId] = st
      fetchedAt.set(serverId, Date.now())
    },
    () => {},
  ).finally(() => {
    if (inFlight.get(serverId) === go) inFlight.delete(serverId)
  })
  inFlight.set(serverId, go)
  return go
}

/** Hold a status the server just answered with (Server settings' pick). */
export function putF1Status(serverId: string, st: F1Status): void {
  f1Status.byServer[serverId] = st
  fetchedAt.set(serverId, Date.now())
}

/** For tests, and on sign-out: forget every status. */
export function resetF1(): void {
  fetchedAt.clear()
  inFlight.clear()
  f1Status.byServer = {}
}
