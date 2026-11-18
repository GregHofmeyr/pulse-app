import { describe, expect, it } from 'vitest'
import { dropPlacement, parseDrop } from './dmDrag'

describe('dropping a conversation on a section (#419)', () => {
  it('reads the drop sections', () => {
    expect(parseDrop('pinned')).toEqual({ kind: 'pinned' })
    expect(parseDrop('rest')).toEqual({ kind: 'rest' })
    expect(parseDrop('category:K1')).toEqual({ kind: 'category', id: 'K1' })
    expect(parseDrop('category:')).toBeNull()
    expect(parseDrop(undefined)).toBeNull()
    expect(parseDrop('other')).toBeNull()
  })

  it('into a category: shown there, so unpinned; onto Pinned: pinned, category kept; onto the list: out of both', () => {
    const loose = { pinned: false, category_id: null }
    expect(dropPlacement(loose, { kind: 'category', id: 'K1' })).toEqual({ pinned: false, category_id: 'K1' })
    expect(dropPlacement({ pinned: true, category_id: null }, { kind: 'category', id: 'K1' })).toEqual({ pinned: false, category_id: 'K1' })
    expect(dropPlacement({ pinned: false, category_id: 'K1' }, { kind: 'pinned' })).toEqual({ pinned: true, category_id: 'K1' })
    expect(dropPlacement({ pinned: true, category_id: 'K1' }, { kind: 'rest' })).toEqual(loose)
  })

  it('a drop where it already is changes nothing', () => {
    expect(dropPlacement({ pinned: false, category_id: 'K1' }, { kind: 'category', id: 'K1' })).toBeNull()
    expect(dropPlacement({ pinned: true, category_id: null }, { kind: 'pinned' })).toBeNull()
    expect(dropPlacement({ pinned: false, category_id: null }, { kind: 'rest' })).toBeNull()
  })
})
