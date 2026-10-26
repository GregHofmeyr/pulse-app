// What the call grid shows, in order. Pure, so it's tested without the UI.
import type { StreamInfo } from './protocol/StreamInfo'
import type { RosterEntry } from './selectors'

export type Tile = { kind: 'user'; entry: RosterEntry } | { kind: 'stream'; streamer: string; mine: boolean }

/** `myLive`: you are live here by your own state, which shows before the server's list does. Each stream tile follows its streamer's user tile; a stream whose streamer isn't listed goes last. `showEveryone: false` keeps the stream tiles only. */
export function gridTiles(a: { roster: RosterEntry[]; streams: Record<string, StreamInfo>; meId: string; showEveryone: boolean; myLive?: boolean }): Tile[] {
  const stream = (streamer: string): Tile => ({ kind: 'stream', streamer, mine: streamer === a.meId })
  const live = (id: string) => !!a.streams[id] || (!!a.myLive && id === a.meId)
  const tiles: Tile[] = []
  const placed = new Set<string>()
  for (const entry of a.roster) {
    if (a.showEveryone) tiles.push({ kind: 'user', entry })
    if (live(entry.userId)) {
      tiles.push(stream(entry.userId))
      placed.add(entry.userId)
    }
  }
  const rest = a.myLive && !a.streams[a.meId] ? [...Object.keys(a.streams), a.meId] : Object.keys(a.streams)
  for (const streamer of rest) if (!placed.has(streamer)) tiles.push(stream(streamer))
  return tiles
}

/** The grid has no stream tile while the toggle hides everyone else: show the empty-state line. */
export function nobodyStreaming(tiles: Tile[], showEveryone: boolean): boolean {
  return !showEveryone && !tiles.some((t) => t.kind === 'stream')
}
