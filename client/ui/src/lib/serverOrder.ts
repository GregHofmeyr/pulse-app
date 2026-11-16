// Server rail order (#185), the pure part: the order you dragged your servers into, kept on the server so it follows
// you to every device (Ready's `server_order`, `ServerOrderChanged`). Servers you never placed (none dragged yet, or
// new since) follow the placed ones in their usual order.
import type { Server } from './protocol/Server'

/** `servers` as your rail shows them: the ones in `order` first, in that order, then the rest as they come. */
export function orderedServers(servers: Server[], order: string[]): Server[] {
  if (order.length === 0) return servers
  const byId = new Map(servers.map((s) => [s.id, s]))
  const placed = order.map((id) => byId.get(id)).filter((s): s is Server => !!s)
  const seen = new Set(placed.map((s) => s.id))
  return [...placed, ...servers.filter((s) => !seen.has(s.id))]
}

/** `ids` (every server, in rail order) with `id` moved to `index` (kept within the ends). */
export function moveServer(ids: string[], id: string, index: number): string[] {
  const from = ids.indexOf(id)
  if (from < 0) return ids
  const rest = ids.filter((x) => x !== id)
  const to = Math.max(0, Math.min(index, rest.length))
  return [...rest.slice(0, to), id, ...rest.slice(to)]
}
