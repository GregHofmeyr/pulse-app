// Notification levels (#195): one list for the server tab's menu and the channel menu.
import type { NotifyLevel } from './protocol/NotifyLevel'

/** The levels, in the order the menus list them (a server's default first). */
export const NOTIFY_LEVELS: readonly { level: NotifyLevel; label: string }[] = [
  { level: 'all', label: 'All messages' },
  { level: 'mentions', label: 'Only @mentions' },
  { level: 'nothing', label: 'Nothing' },
]

/** A channel's choices: following its server (null), then the levels. */
export const CHANNEL_LEVELS: readonly { level: NotifyLevel | null; label: string }[] = [{ level: null, label: 'Follow server' }, ...NOTIFY_LEVELS]
