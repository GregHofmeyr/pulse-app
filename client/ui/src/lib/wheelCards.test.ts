// @vitest-environment jsdom
// A public spin's card stays until seen (#271): then CARD_SEEN_MS more, ✕ folds it at once, CARD_MAX_MS after the spin
// it folds anyway. Per device, remembered across a reload.
import { beforeEach, describe, expect, it } from 'vitest'
import { CARD_MAX_MS, CARD_SEEN_MS, cardState, dismissCard, markCardSeen, resetWheelCards, WHEEL_CARDS_KEY } from './wheelCards'

const T = 1_800_000_000_000

beforeEach(() => {
  localStorage.clear()
  resetWheelCards()
})

describe('a spin card', () => {
  it('shows until seen, folding at the hour at the latest', () => {
    expect(cardState('L1', T, T)).toEqual({ show: true, seen: null, foldAt: T + CARD_MAX_MS })
    expect(cardState('L1', T, T + CARD_MAX_MS - 1).show).toBe(true)
    expect(cardState('L1', T, T + CARD_MAX_MS).show).toBe(false)
  })

  it('once seen, stays CARD_SEEN_MS more (seen once: seeing it again later changes nothing)', () => {
    expect(CARD_SEEN_MS).toBe(20_000)
    markCardSeen('L1', T, T + 5000)
    markCardSeen('L1', T, T + 9000)
    expect(cardState('L1', T, T + 5000 + CARD_SEEN_MS - 1)).toEqual({ show: true, seen: T + 5000, foldAt: T + 5000 + CARD_SEEN_MS })
    expect(cardState('L1', T, T + 5000 + CARD_SEEN_MS).show).toBe(false)
  })

  it('seen just before the hour still folds at the hour', () => {
    markCardSeen('L1', T, T + CARD_MAX_MS - 1000)
    expect(cardState('L1', T, T + CARD_MAX_MS - 1000).foldAt).toBe(T + CARD_MAX_MS)
  })

  it('✕ folds it for good', () => {
    dismissCard('L1', T)
    expect(cardState('L1', T, T + 1).show).toBe(false)
  })

  it('is remembered across a reload, per message', () => {
    markCardSeen('L1', T, T + 1000)
    dismissCard('L2', T)
    resetWheelCards() // the app restarting: only localStorage is left
    expect(cardState('L1', T, T + 2000).seen).toBe(T + 1000)
    expect(cardState('L2', T, T + 2000).show).toBe(false)
    expect(cardState('L3', T, T + 2000).show).toBe(true)
  })

  it('forgets spins older than the hour (the store stays small)', () => {
    markCardSeen('old', T, T + 1000)
    markCardSeen('new', T + CARD_MAX_MS + 10, T + CARD_MAX_MS + 20)
    const stored = JSON.parse(localStorage.getItem(WHEEL_CARDS_KEY)!)
    expect(Object.keys(stored)).toEqual(['new'])
  })

  it('unreadable storage: as if nothing were remembered', () => {
    localStorage.setItem(WHEEL_CARDS_KEY, '{nope')
    resetWheelCards()
    expect(cardState('L1', T, T).show).toBe(true)
    localStorage.setItem(WHEEL_CARDS_KEY, JSON.stringify({ L1: { at: 'x', seen: 'y' }, L2: 5 }))
    resetWheelCards()
    expect(cardState('L1', T, T)).toEqual({ show: true, seen: null, foldAt: T + CARD_MAX_MS })
    expect(cardState('L2', T, T).show).toBe(true)
  })
})
