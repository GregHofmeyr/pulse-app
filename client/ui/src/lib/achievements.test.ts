// Steam achievement cards (#379): rarity, percentages, progress and the profile's completion line.
import { describe, expect, it } from 'vitest'
import { completionLine, formatRate, percentText, progressShare, progressText, rarityOf } from './achievements'

describe('achievement text', () => {
  it('rarity by the share of players', () => {
    expect([rarityOf(null), rarityOf(2), rarityOf(7), rarityOf(20), rarityOf(40), rarityOf(80)]).toEqual([null, 'legendary', 'epic', 'rare', 'uncommon', 'common'])
  })

  it('percentages and progress', () => {
    expect(percentText(12.345)).toBe('12.3 % of players')
    expect(percentText(0.04)).toBe('<0.1 % of players')
    expect(progressText(2, 3)).toBe('2 / 3 · 66 %')
    expect(progressText(3, 3)).toBe('3 / 3 · 100 %')
    expect(progressText(0, 0)).toBe('')
    expect([progressShare(1, 4), progressShare(5, 4), progressShare(1, 0)]).toEqual([0.25, 1, 0])
  })

  it('the completion line', () => {
    expect(completionLine(null, 2)).toBeNull()
    expect(completionLine(62.5, 0)).toBe('62.5 % completion')
    expect(completionLine(100, 1)).toBe('100 % completion · 1 perfect game')
    expect(completionLine(33.33, 3)).toBe('33.3 % completion · 3 perfect games')
    expect(formatRate(100)).toBe('100 %')
  })
})
