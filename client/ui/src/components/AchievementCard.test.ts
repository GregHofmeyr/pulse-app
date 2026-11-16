// @vitest-environment jsdom
// A Steam achievement card (#379): the unlocks with their icons, descriptions and rarity, "and N more", progress; and
// the bigger 100 % card with the game's art. Drawn from a system message by MessageItem.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import AchievementCard from './AchievementCard.svelte'
import type { AchievementCard as Card } from '../lib/protocol/AchievementCard'

const HASH = 'ab'.repeat(32)
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const card = (over: Partial<Card> = {}): Card => ({
  user_id: 'U1',
  app_id: 4000001,
  game: 'Space Game',
  achievements: [
    { name: 'Second Wind', description: 'Run a bit.', icon: { hash: HASH, width: 64, height: 64 }, global_percent: 3.2 },
    { name: 'Hidden One', description: null, icon: null, global_percent: null },
  ],
  more: 2,
  unlocked: 2,
  total: 3,
  complete: false,
  ...over,
})

function show(c: Card) {
  cmp = mount(AchievementCard, { target: document.body, props: { card: c, name: 'alex', time: '12:00', messageId: 'M1' } })
  flushSync()
  return document.querySelector<HTMLElement>('.ach')!
}

describe('AchievementCard', () => {
  it('lists the unlocks with icon, description and rarity, then "and N more" and the progress', () => {
    const el = show(card())
    expect(el.textContent).toContain('alex unlocked in Space Game')
    const rows = el.querySelectorAll('li')
    expect(rows).toHaveLength(2)
    expect(rows[0].querySelector('img.icon')!.getAttribute('src')).toContain(HASH)
    expect(rows[0].textContent).toContain('Run a bit.')
    expect(rows[0].querySelector('.rarity')!.textContent).toBe('3.2 % of players')
    expect(rows[0].querySelector<HTMLElement>('.rarity')!.dataset.rarity).toBe('legendary')
    // no icon or rarity from Steam: a placeholder and no pill
    expect(rows[1].querySelector('.icon.none')).toBeTruthy()
    expect(rows[1].querySelector('.rarity')).toBeNull()
    expect(el.querySelector('.more')!.textContent).toBe('and 2 more')
    expect(el.querySelector('.count')!.textContent).toBe('2 / 3 · 66 %')
    expect(el.classList.contains('complete')).toBe(false)
  })

  it('the 100 % card is bigger: the game art and the headline', () => {
    const el = show(card({ complete: true, achievements: [], more: 0, unlocked: 3, art: { hash: HASH, width: 460, height: 215 } }))
    expect(el.classList.contains('complete')).toBe(true)
    expect(el.querySelector('.headline')!.textContent).toBe('100 % complete')
    expect(el.textContent).toContain('alex got every achievement in Space Game')
    expect(el.querySelector('img.art')!.getAttribute('src')).toContain(HASH)
    expect(el.querySelector('li')).toBeNull()
  })
})
