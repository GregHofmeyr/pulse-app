// What the build pop-out (#620) is handed of a build: title, media and notes, never its comments. The main window
// makes it from the build it holds; the core keeps it for the pop-out (client/src-tauri/src/build_window.rs).
import { buildGameName, buildShots, isArchived, isQuickDrop } from './builds'
import { nameOfIn } from './selectors'
import type { AppState } from './state'
import type { BuildLink } from './protocol/BuildLink'
import type { Message } from './protocol/Message'

export type SnapshotShot = { id: string; width: number | null; height: number | null }

/** build_window.rs's `BuildSnapshot`, field for field. Times are ISO; the pop-out shows them relative. */
export type BuildSnapshot = {
  message_id: string
  title: string
  game_name: string | null
  game_guessed: boolean
  author_name: string
  saved_at: string
  updated_at: string | null
  notes: string | null
  link: BuildLink | null
  screenshots: SnapshotShot[]
  archived: boolean
  quick_drop: boolean
}

export function snapshotOf(m: Message, s: AppState): BuildSnapshot {
  const b = m.build!
  const serverId = s.channels[m.channel_id]?.server_id ?? ''
  return {
    message_id: m.id,
    title: b.title,
    game_name: buildGameName(s, serverId, m),
    game_guessed: b.game_guessed,
    author_name: nameOfIn(() => s, serverId)(m.author_id),
    saved_at: m.created_at,
    updated_at: b.updated_at,
    notes: b.notes,
    link: b.link,
    screenshots: buildShots(m).map((a) => ({ id: a.id, width: a.width, height: a.height })),
    archived: isArchived(m),
    quick_drop: isQuickDrop(m),
  }
}
