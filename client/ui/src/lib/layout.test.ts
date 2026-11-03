import { describe, expect, it } from 'vitest'
import { NARROW_BODY, searchMakesRoom } from './layout'

// #97: in a narrow window only one side panel shows. A replies panel (a quote's or a clip's) opened from search
// hides the results for as long as it's open; closing it brings them back as they were.
describe('searchMakesRoom', () => {
  it('hides the results only when narrow, searching and a replies panel is open', () => {
    expect(searchMakesRoom({ narrow: true, searchOpen: true, threadOpen: true })).toBe(true)
    expect(searchMakesRoom({ narrow: false, searchOpen: true, threadOpen: true })).toBe(false)
    expect(searchMakesRoom({ narrow: true, searchOpen: false, threadOpen: true })).toBe(false)
    expect(searchMakesRoom({ narrow: true, searchOpen: true, threadOpen: false })).toBe(false)
  })
  it('narrow means the window body (sidebar + chat + panels) is under 1100 px', () => {
    expect(NARROW_BODY).toBe(1100)
  })
})
