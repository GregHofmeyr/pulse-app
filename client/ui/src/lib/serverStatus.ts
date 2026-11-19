// Your status in one server (#154), the pure part: the choices on a server's right-click menu, and what it means for
// your own alerts there. Members see it in that server; everyone sees your main status everywhere else.
import type { AppState } from './state'
import type { Status } from './protocol/Status'
import { dndActive } from './status'

/** My status here: Same as everywhere (the default, no status of its own), then the three statuses. */
export const SERVER_STATUS_CHOICES: readonly { label: string; status: Status | null }[] = [
  { label: 'Same as everywhere', status: null },
  { label: 'Online', status: 'online' },
  { label: 'Do Not Disturb', status: 'dnd' },
  { label: 'Invisible', status: 'invisible' },
]

/** How a status reads on the rail's dot. */
export const STATUS_LABEL: Record<Status, string> = { online: 'Online', dnd: 'Do Not Disturb', invisible: 'Invisible' }

/** Your own status in `serverId`, or null when it follows your main one. */
export function myStatusIn(s: Pick<AppState, 'myServerStatuses'>, serverId: string | null | undefined): Status | null {
  if (!serverId) return null
  return s.myServerStatuses.find((x) => x.server_id === serverId)?.status ?? null
}

/** Whether Do Not Disturb holds back alerts from `serverId` (null: a DM or group): your status there when you've set
 *  one (DND there holds them; Online or Invisible there lets them through), else your main DND. */
export function dndIn(s: AppState, serverId: string | null | undefined, nowIso: string): boolean {
  const here = myStatusIn(s, serverId)
  return here ? here === 'dnd' : dndActive(s, nowIso)
}
