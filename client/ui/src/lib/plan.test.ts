// Plan something's pure parts (#387): the form's fields to and from an instant, the gate on Post plan, how a time
// reads, answers, the forecast's words, the reducer's PlanChanged and a reminder's alert.
import { describe, expect, it } from 'vitest'
import {
  answered, defaultStart, fieldsToIso, forecastLine, inForecastWindow, isoToFields, myRsvp, nextRsvp, parsePlanArgs,
  planReady, planWhen, weatherOf,
} from './plan'
import { shouldAlert, type AlertInput } from './conversations'
import { shownText } from './poll'
import { applyEvent, emptyState } from './state'
import type { Message } from './protocol/Message'
import type { Plan } from './protocol/Plan'

const plan = (over: Partial<Plan> = {}): Plan => ({
  title: 'Braai', starts_at: '2026-11-20T16:30:00.000Z', place: 'Cape Town', note: null,
  going: ['U2'], maybe: ['U3'], cant: [], cancelled_at: null, edited_at: null, ...over,
})
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '📅 Plan: Braai', reply_to_id: null,
  created_at: '2026-11-16T09:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [],
  thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})

describe('the form', () => {
  it('turns the day and time (your own time) into an instant and back', () => {
    const iso = fieldsToIso('2026-11-20', '18:30')!
    expect(iso).toBe(new Date(2026, 10, 20, 18, 30).toISOString())
    expect(isoToFields(iso)).toEqual({ date: '2026-11-20', time: '18:30' })
    expect(fieldsToIso('2026-11-31', '18:30')).toBeNull()
    expect(fieldsToIso('', '18:30')).toBeNull()
    expect(fieldsToIso('2026-11-20', '')).toBeNull()
  })

  it('starts on the next whole hour after an hour from now', () => {
    const now = new Date(2026, 10, 16, 10, 20).getTime()
    expect(defaultStart(now)).toEqual({ date: '2026-11-16', time: '12:00' })
  })

  it('posts with a title and a time that is not over', () => {
    const now = Date.parse('2026-11-16T10:00:00.000Z')
    expect(planReady('Braai', '2026-11-20T16:30:00.000Z', '', '', now)).toBe(true)
    expect(planReady('  ', '2026-11-20T16:30:00.000Z', '', '', now)).toBe(false)
    expect(planReady('t'.repeat(101), '2026-11-20T16:30:00.000Z', '', '', now)).toBe(false)
    expect(planReady('Braai', null, '', '', now)).toBe(false)
    expect(planReady('Braai', '2026-11-16T09:00:00.000Z', '', '', now)).toBe(false)
    expect(planReady('Braai', '2026-11-20T16:30:00.000Z', 'p'.repeat(101), '', now)).toBe(false)
    expect(planReady('Braai', '2026-11-20T16:30:00.000Z', '', 'n'.repeat(501), now)).toBe(false)
  })

  it('/plan fills the title', () => {
    expect(parsePlanArgs("  Braai at   kim's ")).toBe("Braai at kim's")
    expect([...parsePlanArgs('é'.repeat(150))].length).toBe(100)
  })
})

describe('how it reads', () => {
  it('in your own time: today, tomorrow, or the day', () => {
    const now = new Date(2026, 10, 16, 10, 0).getTime()
    const time = (h: number, m: number) => new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(new Date(2026, 0, 1, h, m))
    expect(planWhen(new Date(2026, 10, 16, 19, 0).toISOString(), now)).toBe(`Today, ${time(19, 0)}`)
    expect(planWhen(new Date(2026, 10, 17, 9, 30).toISOString(), now)).toBe(`Tomorrow, ${time(9, 30)}`)
    const later = planWhen(new Date(2026, 10, 20, 18, 30).toISOString(), now)
    expect(later.endsWith(`, ${time(18, 30)}`)).toBe(true)
    expect(later).not.toMatch(/2026/)
    expect(planWhen(new Date(2027, 0, 5, 18, 30).toISOString(), now)).toMatch(/2027/)
  })

  it('a forecast only for a place within the forecast reach, not cancelled', () => {
    const now = Date.parse('2026-11-16T10:00:00.000Z')
    expect(inForecastWindow(plan(), now)).toBe(true)
    expect(inForecastWindow(plan({ place: null }), now)).toBe(false)
    expect(inForecastWindow(plan({ cancelled_at: '2026-11-16T09:00:00.000Z' }), now)).toBe(false)
    expect(inForecastWindow(plan({ starts_at: '2026-12-10T10:00:00.000Z' }), now)).toBe(false)
  })

  it('the forecast in words', () => {
    expect(weatherOf(0).text).toBe('Clear')
    expect(weatherOf(63).text).toBe('Rain')
    expect(weatherOf(96).text).toBe('Thunderstorms')
    expect(forecastLine({ place: 'Cape Town', date: '2026-11-20', code: 61, temp_max: 24.6, temp_min: 14.2, rain_chance: 70 }))
      .toBe('Rain · 14°–25° · 70% chance of rain')
    expect(forecastLine({ place: 'Cape Town', date: '2026-11-20', code: 0, temp_max: 30, temp_min: 18, rain_chance: 0 })).toBe('Clear · 18°–30°')
  })

  it("previews show the plan's title", () => {
    expect(shownText(msg({ plan: plan() }))).toBe('📅 Braai')
  })
})

describe('answers', () => {
  it('yours from the lists; clicking yours takes it back', () => {
    const p = plan()
    expect(myRsvp(p, 'U2')).toBe('going')
    expect(myRsvp(p, 'U3')).toBe('maybe')
    expect(myRsvp(p, 'U9')).toBeNull()
    expect(nextRsvp('going', 'going')).toBeNull()
    expect(nextRsvp('going', 'cant')).toBe('cant')
    expect(answered(p, 'maybe')).toEqual(['U3'])
  })

  it('PlanChanged puts the card in place for everyone', () => {
    const s = { ...emptyState(), messages: { C1: [msg({ plan: plan() })] } }
    const changed = plan({ going: ['U2', 'U4'], cancelled_at: '2026-11-17T08:00:00.000Z' })
    const next = applyEvent(s, { t: 'PlanChanged', d: { channel_id: 'C1', message_id: 'M1', plan: changed } }, 0)
    expect(next.messages.C1[0].plan).toEqual(changed)
    // one this client doesn't hold changes nothing
    expect(applyEvent(s, { t: 'PlanChanged', d: { channel_id: 'C1', message_id: 'NOPE', plan: changed } }, 0)).toBe(s)
  })
})

describe("a plan's reminder", () => {
  const line = (mentions: string[]) =>
    msg({ id: 'R1', kind: 'system', author_id: null, content: '⏰ Braai starts in an hour', mentions,
      plan_reminder: { plan_message_id: 'M1', title: 'Braai', starts_at: '2026-11-20T16:30:00.000Z' } })
  const input = (m: Message, over: Partial<AlertInput> = {}): AlertInput => ({
    message: m, me: 'U2', channel: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } as AlertInput['channel'],
    open: false, openThread: null, focused: false, muted: false, level: 'mentions', ...over,
  })
  it('alerts those it pings, as a mention, unless Plan something is off', () => {
    expect(shouldAlert(input(line(['U2', 'U3'])))).toBe(true)
    expect(shouldAlert(input(line(['U3'])))).toBe(false)
    expect(shouldAlert(input(line(['U2']), { planReminders: false }))).toBe(false)
    expect(shouldAlert(input(line(['U2']), { muted: true }))).toBe(true)
    // other system lines still never alert
    expect(shouldAlert(input(msg({ kind: 'system', author_id: null, mentions: ['U2'] })))).toBe(false)
  })
})
