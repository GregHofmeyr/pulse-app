// Your own order of a server's channel list (#304), the client side. While you have one for a server, its list is
// drawn in your order (lib/layout.ts `applyOrder` over the server's layout) and your drags and moves change only
// yours: they never save the server's layout, so they can't be refused over (or get in the way of) anyone else's
// layout save. A move shows at once and is put back if the server refuses; the server's answer (and the
// ChannelOrderChanged it sends your other devices) replaces ours.
import { toPersonalRequest, type Order } from './layout'
import type { ChannelOrder } from './protocol/ChannelOrder'
import { app } from './store.svelte'
import { api } from './tauri'

const put = (serverId: string, order: ChannelOrder | null) => {
  const { [serverId]: _was, ...rest } = app.state.channelOrders
  app.state = { ...app.state, channelOrders: order ? { ...rest, [serverId]: order } : rest }
}

/** Save `order` as your own order of `serverId`'s list (starting one, or changing it): at once, put back when refused. */
export async function savePersonalOrder(serverId: string, order: Order): Promise<void> {
  const was = app.state.channelOrders[serverId] ?? null
  const req = toPersonalRequest(order)
  put(serverId, { server_id: serverId, ...req })
  try {
    put(serverId, await api.setChannelOrder(serverId, req))
  } catch (e) {
    put(serverId, was)
    throw e
  }
}

/** Back to the server's order for `serverId`: at once, put back when refused. */
export async function resetPersonalOrder(serverId: string): Promise<void> {
  const was = app.state.channelOrders[serverId] ?? null
  put(serverId, null)
  try {
    await api.setChannelOrder(serverId, null)
  } catch (e) {
    put(serverId, was)
    throw e
  }
}

/** Whether you have your own order for `serverId`. */
export const hasPersonalOrder = (serverId: string): boolean => !!app.state.channelOrders[serverId]
