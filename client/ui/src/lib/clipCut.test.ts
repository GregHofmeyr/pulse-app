import { describe, expect, it } from 'vitest'
import { cutAt, dragEdge, dropSelected, endDrag, keptMs, keptTimeAt, kept, newCut, nextPlayable, pieceCount, select, sendBlock, undo, MIN_PIECE_MS } from './clipCut'

const threeWay = () => {
  let c = newCut(60_000)
  c = cutAt(c, 10_000)
  c = cutAt(c, 40_000) // right piece [40000,60000] selected
  c = select(c, 1) // the middle
  return dropSelected(c)
}

describe('clipCut', () => {
  it('cut then drop the middle keeps two pieces', () => {
    const c = threeWay()
    expect(kept(c)).toEqual([{ start: 0, end: 10_000 }, { start: 40_000, end: 60_000 }])
    expect(keptMs(c)).toBe(30_000)
    expect(pieceCount(c)).toBe(2)
  })

  it('a cut keeps the parent dropped flag and selects the right piece', () => {
    let c = newCut(10_000)
    c = dropSelected(select(c, 0))
    c = cutAt(c, 4_000)
    expect(c.pieces.map((p) => p.dropped)).toEqual([true, true])
    expect(c.selected).toBe(1)
  })

  it('clicking a dropped piece brings it back', () => {
    const c = select(threeWay(), 1)
    expect(c.pieces[1].dropped).toBe(false)
    expect(keptMs(c)).toBe(60_000)
    expect(kept(c)).toEqual([{ start: 0, end: 60_000 }]) // adjacent kept pieces merge
  })

  it('undo steps back through cuts, drops and restores', () => {
    let c = select(threeWay(), 1) // restore
    c = undo(c)
    expect(c.pieces[1].dropped).toBe(true)
    c = undo(c) // the drop
    expect(c.pieces[1].dropped).toBe(false)
    c = undo(c)
    c = undo(c)
    expect(c.pieces).toEqual([{ start: 0, end: 60_000, dropped: false }])
    expect(undo(c)).toEqual(c)
  })

  it('selecting a kept piece is not an undo step', () => {
    const c = newCut(1000)
    expect(select(c, 0).past).toEqual([])
  })

  it('a drag is one undo step', () => {
    let c = cutAt(newCut(60_000), 10_000)
    const before = c.pieces
    c = dragEdge(c, 1, 12_000)
    c = dragEdge(c, 1, 15_000)
    c = dragEdge(c, 1, 20_000)
    expect(c.pieces.map((p) => p.end)).toEqual([20_000, 60_000])
    c = undo(c)
    expect(c.pieces).toEqual(before)
  })

  it('endDrag makes two drags of the same edge two undo steps', () => {
    let c = cutAt(newCut(60_000), 10_000)
    c = dragEdge(c, 1, 12_000)
    c = endDrag(c)
    c = dragEdge(c, 1, 30_000)
    c = undo(c)
    expect(c.pieces[0].end).toBe(12_000)
    c = undo(c)
    expect(c.pieces[0].end).toBe(10_000)
  })

  it('dragging another edge starts a new undo step', () => {
    let c = cutAt(cutAt(newCut(60_000), 10_000), 30_000)
    c = dragEdge(c, 1, 11_000)
    c = dragEdge(c, 2, 31_000)
    c = undo(c)
    expect(c.pieces.map((p) => p.end)).toEqual([11_000, 30_000, 60_000])
  })

  it('cuts_too_close_to_an_edge_are_ignored', () => {
    const c = newCut(60_000)
    expect(cutAt(c, 50)).toBe(c)
    expect(cutAt(c, 60_000 - 50)).toBe(c)
    expect(cutAt(c, 0)).toBe(c)
    expect(cutAt(c, 60_000)).toBe(c)
    const d = cutAt(c, 30_000)
    expect(cutAt(d, 30_000)).toBe(d)
    expect(cutAt(d, 30_000 + 99)).toBe(d)
    expect(cutAt(d, 30_000 + MIN_PIECE_MS).pieces).toHaveLength(3)
  })

  it('edges clamp at MIN_PIECE_MS', () => {
    const c = cutAt(cutAt(newCut(60_000), 10_000), 30_000)
    expect(dragEdge(c, 1, -5_000).pieces[0].end).toBe(MIN_PIECE_MS)
    expect(dragEdge(c, 1, 50_000).pieces[0].end).toBe(30_000 - MIN_PIECE_MS)
    expect(dragEdge(c, 2, 0).pieces[1].end).toBe(10_000 + MIN_PIECE_MS)
    expect(dragEdge(c, 2, 99_000).pieces[1].end).toBe(60_000 - MIN_PIECE_MS)
    expect(dragEdge(c, 1, 1234.6).pieces[0].end).toBe(1235)
    for (const p of dragEdge(c, 1, -5_000).pieces) expect(p.end - p.start).toBeGreaterThanOrEqual(MIN_PIECE_MS)
    expect(dragEdge(c, 0, 5)).toBe(c)
    expect(dragEdge(c, 3, 5)).toBe(c)
  })

  it('playback skips dropped pieces', () => {
    const c = threeWay()
    expect(nextPlayable(c, 5_000)).toBe(5_000)
    expect(nextPlayable(c, 15_000)).toBe(40_000)
    expect(nextPlayable(c, 40_000)).toBe(40_000)
    expect(nextPlayable(c, 60_000)).toBeNull()
    const tail = dropSelected(select(cutAt(newCut(60_000), 50_000), 1))
    expect(nextPlayable(tail, 55_000)).toBeNull()
  })

  it('kept time readout', () => {
    const c = threeWay()
    expect(keptTimeAt(c, 45_000)).toBe(15_000)
    expect(keptTimeAt(c, 5_000)).toBe(5_000)
    expect(keptTimeAt(c, 20_000)).toBe(10_000)
    expect(keptTimeAt(c, 60_000)).toBe(30_000)
  })

  it('over 1:30 blocks send', () => {
    expect(sendBlock(newCut(100_000))).toBe('over')
    expect(sendBlock(newCut(90_000))).toBeNull()
    expect(sendBlock(newCut(90_001))).toBe('over')
  })

  it('dropping_everything_disables_send', () => {
    let c = dropSelected(select(newCut(30_000), 0))
    expect(kept(c)).toEqual([])
    expect(keptMs(c)).toBe(0)
    expect(pieceCount(c)).toBe(0)
    expect(nextPlayable(c, 0)).toBeNull()
    expect(sendBlock(c)).toBe('empty')
    c = undo(c)
    expect(sendBlock(c)).toBeNull()
  })

  it('keeping under a second blocks send', () => {
    expect(sendBlock(newCut(999))).toBe('empty')
    expect(sendBlock(newCut(1000))).toBeNull()
    let c = cutAt(newCut(30_000), 500)
    c = dropSelected(c) // drops the right piece, keeps 500 ms
    expect(sendBlock(c)).toBe('empty')
  })

  it('dropSelected with nothing selected is a no-op', () => {
    const c = newCut(5000)
    expect(dropSelected({ ...c, selected: null })).toEqual({ ...c, selected: null })
  })

  it('ignores non-finite input', () => {
    expect(newCut(NaN).pieces).toEqual([{ start: 0, end: 0, dropped: false }])
    const c = cutAt(newCut(10_000), 5000)
    expect(dragEdge(c, 1, NaN)).toBe(c)
    expect(dragEdge(c, 1, Infinity)).toBe(c)
  })

  it('a drag that changes nothing records no undo step', () => {
    const c = cutAt(newCut(10_000), 5000)
    expect(dragEdge(c, 1, 5000)).toBe(c)
    expect(dragEdge(c, 1, 5000).past).toHaveLength(1)
  })

  it('undo leaves nothing selected, so dropSelected cannot drop the wrong piece', () => {
    let c = cutAt(cutAt(cutAt(newCut(40_000), 10_000), 20_000), 30_000)
    c = select(c, 2)
    c = undo(c) // undo the last cut: index 2 would now be a different piece
    expect(c.selected).toBeNull()
    expect(dropSelected(c)).toBe(c)
    expect(keptMs(c)).toBe(40_000)
  })

  it('never makes a zero-length piece', () => {
    let c = newCut(1000)
    for (const t of [0, 1, 99, 100, 500, 900, 901, 999, 1000]) c = cutAt(c, t)
    for (const p of c.pieces) expect(p.end - p.start).toBeGreaterThanOrEqual(MIN_PIECE_MS)
  })
})
