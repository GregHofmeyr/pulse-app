import { describe, expect, it } from 'vitest'
import type { RosterEntry } from './selectors'
import type { StreamInfo } from './protocol/StreamInfo'
import { gridTiles, nobodyStreaming } from './callgrid'

const entry = (userId: string): RosterEntry => ({ userId, name: userId, muted: false, deafened: false, pending: false })
const alex = entry('alex')
const sam = entry('sam')
const info = (user: string) => ({ user_id: user }) as StreamInfo
const label = (t: ReturnType<typeof gridTiles>) => t.map((x) => (x.kind === 'user' ? `user ${x.entry.userId}` : `stream ${x.streamer}`))

describe('gridTiles', () => {
  it('stream_tile_follows_its_streamer', () => {
    const t = gridTiles({ roster: [alex, sam], streams: { sam: info('sam') }, meId: 'alex', showEveryone: true })
    expect(label(t)).toEqual(['user alex', 'user sam', 'stream sam'])
  })

  it('marks_my_own_stream', () => {
    const t = gridTiles({ roster: [alex, sam], streams: { alex: info('alex') }, meId: 'alex', showEveryone: true })
    expect(t[1]).toEqual({ kind: 'stream', streamer: 'alex', mine: true })
  })

  it('orphan_stream_goes_last', () => {
    const t = gridTiles({ roster: [alex, sam], streams: { ghost: info('ghost'), alex: info('alex') }, meId: 'sam', showEveryone: true })
    expect(label(t)).toEqual(['user alex', 'stream alex', 'user sam', 'stream ghost'])
  })

  it('toggle_off_shows_only_streams', () => {
    const t = gridTiles({ roster: [alex, sam], streams: { sam: info('sam') }, meId: 'alex', showEveryone: false })
    expect(label(t)).toEqual(['stream sam'])
  })
})

describe('nobodyStreaming', () => {
  it('nobody_streaming_with_toggle_off_is_empty_state', () => {
    const off = gridTiles({ roster: [alex, sam], streams: {}, meId: 'alex', showEveryone: false })
    expect(nobodyStreaming(off, false)).toBe(true)
    const on = gridTiles({ roster: [alex, sam], streams: {}, meId: 'alex', showEveryone: true })
    expect(nobodyStreaming(on, true)).toBe(false)
    const live = gridTiles({ roster: [alex], streams: { alex: info('alex') }, meId: 'alex', showEveryone: false })
    expect(nobodyStreaming(live, false)).toBe(false)
  })
})
