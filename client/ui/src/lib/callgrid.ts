// What the call grid shows, in order. Pure, so it's tested without the UI.
import type { StreamInfo } from './protocol/StreamInfo'
import type { RosterEntry } from './selectors'

export type Tile = { kind: 'user'; entry: RosterEntry } | { kind: 'stream'; streamer: string; mine: boolean }

/** Each stream tile follows its streamer's user tile; a stream whose streamer isn't listed goes last. `showEveryone: false` keeps the stream tiles only. */
export function gridTiles(a: { roster: RosterEntry[]; streams: Record<string, StreamInfo>; meId: string; showEveryone: boolean }): Tile[] {
  const stream = (streamer: string): Tile => ({ kind: 'stream', streamer, mine: streamer === a.meId })
  const tiles: Tile[] = []
  const placed = new Set<string>()
  for (const entry of a.roster) {
    if (a.showEveryone) tiles.push({ kind: 'user', entry })
    if (a.streams[entry.userId]) {
      tiles.push(stream(entry.userId))
      placed.add(entry.userId)
    }
  }
  for (const streamer of Object.keys(a.streams)) if (!placed.has(streamer)) tiles.push(stream(streamer))
  return tiles
}

/** The grid has no stream tile while the toggle hides everyone else: show the empty-state line. */
export function nobodyStreaming(tiles: Tile[], showEveryone: boolean): boolean {
  return !showEveryone && !tiles.some((t) => t.kind === 'stream')
}
