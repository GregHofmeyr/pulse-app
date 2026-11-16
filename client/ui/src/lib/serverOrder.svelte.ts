// Server rail order (#185): moving a server in your rail, by drag, Alt+Left / Alt+Right or its menu. Shown at once,
// saved to your account (so your other devices follow), and put back if the save is refused.
import { app } from './store.svelte'
import { api, errorText } from './tauri'
import { moveServer, orderedServers } from './serverOrder'

/** Your servers' ids as your rail shows them. */
export const railIds = (): string[] => orderedServers(app.state.servers, app.state.serverOrder).map((s) => s.id)

/** Move `id` to `index` in your rail. Resolves with an error message if the save was refused (the old order is back),
 *  else null. */
export async function moveServerTo(id: string, index: number): Promise<string | null> {
  const before = app.state.serverOrder
  const ids = railIds()
  const next = moveServer(ids, id, index)
  if (next.every((x, i) => x === ids[i])) return null
  app.state = { ...app.state, serverOrder: next }
  try {
    await api.setServerOrder(next)
    return null
  } catch (e) {
    // only if nothing newer (another move, or the echo from another device) has landed since
    if (app.state.serverOrder === next) app.state = { ...app.state, serverOrder: before }
    return errorText(e)
  }
}

/** One place left (-1) or right (1). */
export function stepServer(id: string, dir: -1 | 1): Promise<string | null> {
  const at = railIds().indexOf(id)
  return at < 0 ? Promise.resolve(null) : moveServerTo(id, at + dir)
}
