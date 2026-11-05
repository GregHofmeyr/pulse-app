// A reorder of a server's channel list (#54) shows at once: the new order is held as a draft, drawn over the
// server's layout (`applyOrder`), until the save answers. Saved: the server's layout takes over, then the draft goes
// (same order, so nothing flashes). Refused (someone moved things first): the draft goes and the list snaps to the
// server's order.
import { applyLayout } from './state'
import { toRequest, type Order } from './layout'
import { api, errorText } from './tauri'
import { app, ui } from './store.svelte'

/** serverId -> the order being saved. */
export const drafts = $state({ byServer: {} as Record<string, Order> })

/** serverId -> the latest save's number: only its answer drops the draft. */
const latest: Record<string, number> = {}
let saves = 0

function dropDraft(serverId: string, save: number): void {
  // A later save replaced the draft: that one's answer decides.
  if (latest[serverId] !== save) return
  const { [serverId]: _gone, ...rest } = drafts.byServer
  drafts.byServer = rest
}

/** Save `order` as `serverId`'s list, based on the layout revision held. A second save while one is in flight
 *  replaces the draft and is based on the same revision (refused if the first landed). */
export async function saveOrder(serverId: string, order: Order): Promise<void> {
  const revision = app.state.layouts[serverId]?.revision ?? 0
  const save = (latest[serverId] = ++saves)
  drafts.byServer = { ...drafts.byServer, [serverId]: order }
  try {
    const saved = await api.saveLayout(serverId, toRequest(order, revision))
    app.state = applyLayout(app.state, saved)
    dropDraft(serverId, save)
  } catch (e) {
    dropDraft(serverId, save)
    ui.notice = errorText(e)
  }
}
