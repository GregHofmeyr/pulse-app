// @vitest-environment jsdom
// F1 race weekends (#386) in the chat: the weekend card (the race, sessions in your own time, the countdown, podium
// predictions with the dice, live picks from others, closed at lights out), the lights-out line, the results card,
// the scores card, and the cards folded to one line with "F1 race weekends" off.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { F1Card as Card } from '../lib/protocol/F1Card'
import type { F1Prediction } from '../lib/protocol/F1Prediction'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { f1Prefs } from '../lib/f1Prefs.svelte'

const NOW = Date.parse('2030-03-14T07:00:00.000Z')
const RACE = '2030-03-15T07:00:00.000Z'
const driver = (id: string, code: string, name: string) => ({ id, code, name, team: 'Rocket' })
const FIELD = [driver('racer_a', 'RAA', 'Ace Racer'), driver('rider_b', 'RIB', 'Bo Rider'), driver('speed_c', 'SPC', 'Cy Speed'), driver('dash_d', 'DAD', 'Di Dash')]
const weekendCard = (predictions: F1Prediction[] = [], race_at = RACE): Card => ({
  kind: 'weekend',
  weekend: {
    season: 2030, round: 2, race_name: 'Harbour Grand Prix', circuit: 'Harbour Street Circuit', locality: 'Port Town', country: 'UK', race_at,
    sessions: [{ kind: 'practice1', starts_at: '2030-03-13T03:30:00.000Z' }, { kind: 'qualifying', starts_at: '2030-03-14T08:00:00.000Z' }, { kind: 'race', starts_at: race_at }],
  },
  drivers: FIELD,
  predictions,
})
const msg = (card: Card, id = 'M1'): Message => ({
  id, channel_id: 'C1', author_id: null, kind: 'system', content: '🏁 Race weekend', reply_to_id: null, created_at: '2030-03-13T00:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], f1: card,
})
const names: Record<string, string> = { U1: 'alex', U2: 'sam', U3: 'kim', U4: 'jo' }

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message) {
  if (cmp) unmount(cmp)
  document.body.innerHTML = ''
  app.state = {
    ...emptyState(),
    me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } },
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'f1', position: 0 } } as never,
    messages: { C1: [m] },
  }
  mountShown(m)
}
function mountShown(m: Message) {
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: 'S1', name: '', nameOf: (id: string | null) => (id ? names[id] : 'Someone'), replyTo: null,
    grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
function reshow() {
  if (cmp) unmount(cmp)
  document.body.innerHTML = ''
  mountShown(app.state.messages.C1[0])
}
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick(); flushSync() }
const selects = () => [...document.querySelectorAll<HTMLSelectElement>('.predict select')]
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === text || b.getAttribute('aria-label') === text)
function choose(i: number, value: string) {
  const s = selects()[i]
  s.value = value
  s.dispatchEvent(new Event('change', { bubbles: true }))
  flushSync()
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  f1Prefs.on = true
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('the race weekend card', () => {
  it('shows the race, its flag, the countdown and every session', () => {
    show(msg(weekendCard()))
    const card = document.querySelector('.f1.weekend')!
    expect(card.querySelector('.title')!.textContent).toBe('Harbour Grand Prix')
    expect(card.querySelector('.flag')!.textContent).toBe('🇬🇧')
    expect(card.querySelector('.where')!.textContent).toBe('Harbour Street Circuit · Port Town, UK')
    expect(card.querySelector('.count')!.textContent).toBe('Lights out in 1d 0h')
    const sessions = [...card.querySelectorAll('.sessions li span')].map((s) => s.textContent)
    expect(sessions).toEqual(['Practice 1', 'Qualifying', 'Race'])
    expect(card.querySelectorAll('.sessions li.past')).toHaveLength(1)
  })

  it('picks three drivers and saves them; the dice pick for you', async () => {
    const saved: F1Prediction[] = [{ user_id: 'U1', picks: ['racer_a', 'rider_b', 'speed_c'], points: null }]
    invoke.mockImplementation(async (cmd: string) => (cmd === 'f1_predict' ? saved : undefined))
    show(msg(weekendCard()))
    expect(button('Save picks')!.disabled).toBe(true)
    choose(0, 'racer_a')
    choose(1, 'rider_b')
    // a driver picked already can't be picked again
    expect([...selects()[2].options].find((o) => o.value === 'racer_a')!.disabled).toBe(true)
    choose(2, 'speed_c')
    expect(button('Save picks')!.disabled).toBe(false)
    button('Save picks')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('f1_predict', { messageId: 'M1', picks: ['racer_a', 'rider_b', 'speed_c'] })
    reshow()
    expect(document.querySelector('.who li.me .name')!.textContent).toBe('alex')
    expect([...document.querySelectorAll('.who li.me .code')].map((c) => c.textContent)).toEqual(['RAA', 'RIB', 'SPC'])
    // nothing changed yet
    expect(button('Update picks')!.disabled).toBe(true)
    // the dice: three different drivers from the field
    button('Pick three at random')!.click()
    flushSync()
    const rolled = selects().map((s) => s.value)
    expect(new Set(rolled).size).toBe(3)
    expect(rolled.every((id) => FIELD.some((d) => d.id === id))).toBe(true)
    // and Clear mine takes it back
    invoke.mockResolvedValue([])
    button('Clear mine')!.click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('f1_predict', { messageId: 'M1', picks: null })
  })

  it("others' picks land live", () => {
    show(msg(weekendCard()))
    app.state = applyEvent(app.state, { t: 'F1PredictionsChanged', d: { channel_id: 'C1', message_id: 'M1', predictions: [{ user_id: 'U2', picks: ['dash_d', 'racer_a', 'rider_b'], points: null }] } }, NOW)
    reshow()
    expect(document.querySelector('.who .name')!.textContent).toBe('sam')
    expect([...document.querySelectorAll('.who .code')].map((c) => c.textContent)).toEqual(['DAD', 'RAA', 'RIB'])
  })

  it('closes at lights out and shows the points once scored', () => {
    show(msg(weekendCard([{ user_id: 'U2', picks: ['racer_a', 'rider_b', 'speed_c'], points: 5 }], '2030-03-14T06:00:00.000Z')))
    expect(selects()).toHaveLength(0)
    expect(document.querySelector('.count')!.textContent).toBe('Lights out')
    expect(document.querySelector('.predict .hint')!.textContent).toBe('Closed at lights out.')
    expect(document.querySelector('.who .pts')!.textContent).toBe('5 pts')
  })
})

describe('the other F1 cards', () => {
  it('the lights-out line jumps to your picks', () => {
    show(msg({ kind: 'lights_out', season: 2030, round: 2, race_name: 'Harbour Grand Prix', race_at: RACE, weekend_message_id: 'M0' }))
    const line = document.querySelector('.system')!
    expect(line.textContent).toContain('🚦 Lights out in 1 hour: Harbour Grand Prix')
    expect(button('Your picks')).toBeTruthy()
  })

  it('the results card: the podium, the rest, the fastest lap and the standings', () => {
    const row = (position: number, d: (typeof FIELD)[number], time: string, points: number) => ({ position, driver: d, time, points })
    show(msg({
      kind: 'results', season: 2030, round: 2, race_name: 'Harbour Grand Prix', country: 'UK',
      results: [row(1, FIELD[0], '1:30:00.000', 25), row(2, FIELD[2], '+1.000', 18), row(3, FIELD[1], '+2.000', 15), row(4, FIELD[3], '+1 Lap', 12)],
      fastest_lap: { driver: FIELD[2], lap: 40, time: '1:20.001' },
      standings: [{ position: 1, driver: FIELD[0], points: 100, wins: 3 }],
      constructor_standings: [{ position: 1, team: 'Rocket', points: 180, wins: 3 }, { position: 2, team: 'Comet', points: 90.5, wins: 0 }],
    }))
    const steps = [...document.querySelectorAll('.podium .step')]
    // P2, P1, P3
    expect(steps.map((s) => s.querySelector('.code')!.textContent)).toEqual(['SPC', 'RAA', 'RIB'])
    expect(steps[1].classList.contains('p1')).toBe(true)
    expect(document.querySelector('.rows .who')!.textContent).toContain('Di Dash')
    expect(document.querySelector('.fast')!.textContent).toBe('⏱ Fastest lap: Cy Speed 1:20.001 (lap 40)')
    expect(document.querySelector('.standings summary')!.textContent).toBe("Drivers' standings after round 2")
    // the constructors' table too (#403)
    const teams = document.querySelector('[data-constructors]')!
    expect(teams.querySelector('summary')!.textContent).toBe("Constructors' standings after round 2")
    expect([...teams.querySelectorAll('tr')].map((r) => r.textContent)).toEqual(['1Rocket3 wins180', '2Comet90.5'])
  })

  it('a results card from before constructors were added shows none', () => {
    show(msg({
      kind: 'results', season: 2030, round: 2, race_name: 'Harbour Grand Prix', country: 'UK',
      results: [{ position: 1, driver: FIELD[0], time: '1:30:00.000', points: 25 }], fastest_lap: null, standings: [],
    } as unknown as Card))
    expect(document.querySelector('[data-constructors]')).toBeNull()
  })

  it("the sprint card (#403): the sprint's top 10, points where scored", () => {
    show(msg({
      kind: 'sprint', season: 2030, round: 2, race_name: 'Harbour Grand Prix', country: 'UK',
      results: [{ position: 1, driver: FIELD[1], time: '26:51.033', points: 8 }, { position: 2, driver: FIELD[0], time: '+4.951', points: 7 }, { position: 9, driver: FIELD[3], time: '+20.1', points: 0 }],
    }))
    const card = document.querySelector('[data-f1-sprint]')!
    expect(card.querySelector('.title')!.textContent).toBe('Harbour Grand Prix Sprint')
    expect([...card.querySelectorAll('tr')].map((r) => r.textContent)).toEqual(['1Bo RiderRocket26:51.0338', '2Ace RacerRocket+4.9517', '9Di DashRocket+20.1'])
    f1Prefs.on = false
    reshow()
    expect(document.querySelector('.system.folded')!.textContent).toContain('🏎️ Sprint, Harbour Grand Prix: RIB, RAA, DAD')
  })

  it('picks close when qualifying starts when the server says so (#403)', () => {
    // qualifying at 08:00, an hour away: still open, and the hint says when it closes
    show(msg({ ...weekendCard(), picks_close_at: '2030-03-14T08:00:00.000Z' } as Card))
    expect(selects()).toHaveLength(3)
    expect(document.querySelector('.predict .hint')!.textContent).toContain('Closes when qualifying starts.')
    // qualifying has started: closed, though lights out is a day away
    vi.setSystemTime(Date.parse('2030-03-14T08:01:00.000Z'))
    reshow()
    expect(selects()).toHaveLength(0)
    expect(document.querySelector('.count')!.textContent).toContain('Lights out in')
    expect(document.querySelector('.predict .hint')!.textContent).toBe('Closed when qualifying started.')
  })

  it('the scores card: each pick marked, the points and the season', () => {
    show(msg({
      kind: 'scores', season: 2030, round: 2, race_name: 'Harbour Grand Prix', podium: [FIELD[0], FIELD[2], FIELD[1]], drivers: FIELD,
      scores: [{ user_id: 'U2', picks: ['racer_a', 'rider_b', 'speed_c'], points: 5 }, { user_id: 'U3', picks: ['dash_d', 'racer_a', 'rider_b'], points: 2 }],
      season_table: [{ user_id: 'U2', points: 12, races: 2 }, { user_id: 'U3', points: 2, races: 1 }], weekend_message_id: 'M0',
    }))
    const first = document.querySelector('.scores li')!
    expect(first.querySelector('.name')!.textContent).toBe('sam')
    expect([...first.querySelectorAll('.code')].map((c) => [c.textContent, c.classList.contains('hit'), c.classList.contains('near')])).toEqual([
      ['RAA', true, false], ['RIB', false, true], ['SPC', false, true],
    ])
    expect(first.querySelector('.pts')!.textContent).toBe('5 pts')
    // a pick off the podium is named from the field
    expect([...document.querySelectorAll('.scores li')][1].querySelector('.code')!.textContent).toBe('DAD')
    expect([...document.querySelectorAll('.season li')].map((l) => l.textContent)).toEqual(['1sam2 races12 pts', '2kim1 race2 pts'])
    expect(button("See everyone's picks")).toBeTruthy()
  })

  it('fold to one line with "F1 race weekends" off, and open on a click', () => {
    f1Prefs.on = false
    show(msg(weekendCard()))
    expect(document.querySelector('.f1')).toBeNull()
    expect(document.querySelector('.system.folded')!.textContent).toContain('🏁 Race weekend: Harbour Grand Prix')
    button('Show')!.click()
    flushSync()
    expect(document.querySelector('.f1.weekend')).not.toBeNull()
  })
})
