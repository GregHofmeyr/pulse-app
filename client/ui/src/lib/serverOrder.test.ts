// Server rail order (#185): your own order of the servers, and moving one.
import { describe, expect, it } from 'vitest'
import { moveServer, orderedServers } from './serverOrder'
import { applyEvent, applyReady, emptyState } from './state'
import { testReady } from './testReady'

const srv = (id: string) => ({ id, name: id, icon_hash: null, created_by: null }) as never
const ids = (xs: { id: string }[]) => xs.map((x) => x.id)

describe('server rail order', () => {
  it('placed servers first, in your order; the rest after, as they come', () => {
    const servers = [srv('A'), srv('B'), srv('C'), srv('D')]
    expect(ids(orderedServers(servers, []))).toEqual(['A', 'B', 'C', 'D'])
    expect(ids(orderedServers(servers, ['C', 'A']))).toEqual(['C', 'A', 'B', 'D'])
    // a server that's gone is skipped
    expect(ids(orderedServers(servers, ['X', 'D']))).toEqual(['D', 'A', 'B', 'C'])
  })

  it('moving one gives every server, in the new order', () => {
    expect(moveServer(['A', 'B', 'C'], 'A', 2)).toEqual(['B', 'C', 'A'])
    expect(moveServer(['A', 'B', 'C'], 'C', 0)).toEqual(['C', 'A', 'B'])
    expect(moveServer(['A', 'B', 'C'], 'B', 1)).toEqual(['A', 'B', 'C'])
    // past either end stays at that end; an unknown id changes nothing
    expect(moveServer(['A', 'B', 'C'], 'B', 9)).toEqual(['A', 'C', 'B'])
    expect(moveServer(['A', 'B', 'C'], 'B', -3)).toEqual(['B', 'A', 'C'])
    expect(moveServer(['A', 'B', 'C'], 'X', 0)).toEqual(['A', 'B', 'C'])
  })

  it('the state keeps it from Ready (absent from older servers) and ServerOrderChanged', () => {
    let s = applyReady(emptyState(), testReady([]))
    expect(s.serverOrder).toEqual([])
    s = applyReady(emptyState(), testReady([], { server_order: ['S2', 'S1'] }))
    expect(s.serverOrder).toEqual(['S2', 'S1'])
    s = applyEvent(s, { t: 'ServerOrderChanged', d: { user_id: 'U1', server_ids: ['S1'] } }, 0)
    expect(s.serverOrder).toEqual(['S1'])
  })
})
