// @vitest-environment jsdom
// A plan in the chat (#387): the card (title, your-time when, place, note), answering (yours in the accent, again
// takes it back), who picked what, a cancelled plan, its creator's Edit plan and Cancel plan in the message's menu,
// Add to calendar, the forecast (only within reach, only while Plan something is on) and the reminder line.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Plan } from '../lib/protocol/Plan'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { resetPlans } from '../lib/plans.svelte'
import { planPrefs } from '../lib/planPrefs.svelte'

const NOW = Date.parse('2026-11-16T10:00:00.000Z')
const plan = (over: Partial<Plan> = {}): Plan => ({
  title: 'Braai at kim\'s', starts_at: '2026-11-20T16:30:00.000Z', place: 'Cape Town', note: 'bring ice',
  going: [], maybe: [], cant: [], cancelled_at: null, edited_at: null, ...over,
})
const msg = (p: Plan | null, over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '📅 Plan: Braai',
  reply_to_id: null, created_at: '2026-11-16T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...(p ? { plan: p } : {}), ...over,
})
const names: Record<string, string> = { U1: 'alex', U2: 'sam', U3: 'kim', U4: 'jo' }

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message, mine = false) {
  app.state = {
    ...emptyState(),
    me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } },
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } },
    messages: { C1: [m] },
  }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: names[m.author_id ?? ''] ?? '', nameOf: (id: string | null) => (id ? names[id] : 'Someone'), replyTo: null,
    grouped: false, mine, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
/** Show again with the message as the store now holds it (the reducer or an answer changed it). */
function reshow(mine = false) {
  const m = app.state.messages.C1[0]
  if (cmp) unmount(cmp)
  document.body.innerHTML = ''
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: names[m.author_id ?? ''] ?? '', nameOf: (id: string | null) => (id ? names[id] : 'Someone'), replyTo: null,
    grouped: false, mine, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const card = () => document.querySelector<HTMLElement>('.plan')!
const rsvp = (label: string) => [...card().querySelectorAll<HTMLButtonElement>('button.rsvp')].find((b) => b.textContent!.includes(label))!
const menuItems = () => [...document.querySelectorAll<HTMLButtonElement>('[role=menu][aria-label="Message actions"] > [role=menuitem]')].map((b) => b.textContent!.trim())
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  resetPlans()
  planPrefs.on = true
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  ui.goTo = null
  ui.jump = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('the plan card', () => {
  it('shows the title, when in your own time, the place and the note, standing in for the text', () => {
    show(msg(plan()))
    expect(card().querySelector('.title')!.textContent).toBe("Braai at kim's")
    expect(card().querySelector('time')!.getAttribute('datetime')).toBe('2026-11-20T16:30:00.000Z')
    expect(card().querySelector('.place')!.textContent!.trim()).toBe('Cape Town')
    expect(card().querySelector('.note')!.textContent).toBe('bring ice')
    expect([...card().querySelectorAll('button.rsvp')].map((b) => b.textContent!.replace(/\s+/g, ''))).toEqual(['Going0', 'Maybe0', "Can't0"])
    expect(document.querySelector('.content')).toBeNull()
  })

  it('a click answers; the answer lands with the counts and who picked what; again takes it back', async () => {
    show(msg(plan({ going: ['U3'] })))
    invoke.mockResolvedValueOnce(plan({ going: ['U3', 'U1'], maybe: ['U4'] }))
    rsvp('Going').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('rsvp_plan', { messageId: 'M1', rsvp: 'going' })
    reshow()
    expect(rsvp('Going').getAttribute('aria-pressed')).toBe('true')
    expect(rsvp('Going').classList.contains('on')).toBe(true)
    expect(rsvp('Going').textContent!.replace(/\s+/g, '')).toBe('Going2')
    expect([...card().querySelectorAll('.who li')].map((li) => li.querySelector('.who-label')!.textContent)).toEqual(['Going', 'Maybe'])
    expect(card().querySelector('.who [role=img]')!.getAttribute('aria-label')).toBe('kim and alex')

    invoke.mockResolvedValueOnce(plan({ going: ['U3'] }))
    rsvp('Going').click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('rsvp_plan', { messageId: 'M1', rsvp: null })
  })

  it('a refused answer says why', async () => {
    show(msg(plan()))
    invoke.mockRejectedValueOnce('this plan was cancelled')
    rsvp('Maybe').click()
    await settle()
    expect(card().querySelector('[role=alert]')!.textContent).toBe('this plan was cancelled')
  })

  it('with an end (#399), shows the range, and says Ended once it has', () => {
    show(msg(plan({ ends_at: '2026-11-20T19:00:00.000Z' })))
    const end = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(Date.parse('2026-11-20T19:00:00.000Z'))
    expect(card().querySelector('time')!.textContent).toMatch(new RegExp(`–${end}$`))
    unmount(cmp!)
    vi.setSystemTime(Date.parse('2026-11-20T19:30:00.000Z'))
    show(msg(plan({ starts_at: '2026-11-20T16:30:00.000Z', ends_at: '2026-11-20T19:00:00.000Z' })))
    expect(card().querySelector('.when')!.textContent).toContain('Ended')
  })

  it('a cancelled plan says so and takes no answers', () => {
    show(msg(plan({ cancelled_at: '2026-11-17T08:00:00.000Z' })), true)
    expect(card().classList.contains('cancelled')).toBe(true)
    expect(card().querySelector('.when')!.textContent).toContain('Cancelled')
    expect(rsvp('Going').disabled).toBe(true)
    expect(card().querySelector('.add')).toBeNull()
    // and its creator can't change it any more
    document.querySelector<HTMLButtonElement>('button[aria-label="More"]')!.click()
    flushSync()
    expect(menuItems()).not.toContain('Edit plan')
    expect(menuItems()).not.toContain('Cancel plan')
  })

  it('Add to calendar saves the .ics through the core', async () => {
    show(msg(plan()))
    invoke.mockResolvedValueOnce('/home/alex/Braai.ics')
    card().querySelector<HTMLButtonElement>('.add')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('save_plan_ics', { messageId: 'M1', title: "Braai at kim's" })
  })
})

describe('the forecast', () => {
  it('within reach: asked of the server once and shown', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'plan_forecast' ? { place: 'Cape Town, South Africa', date: '2026-11-20', code: 61, temp_max: 24.6, temp_min: 14.2, rain_chance: 70 } : undefined))
    show(msg(plan()))
    await settle()
    flushSync()
    expect(invoke.mock.calls.filter(([c]) => c === 'plan_forecast')).toEqual([['plan_forecast', { messageId: 'M1' }]])
    expect(card().querySelector('.forecast')!.textContent).toContain('Rain · 14°–25° · 70% chance of rain')
  })

  it('a failed fetch says so with Retry (#399), which asks again', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'plan_forecast') throw 'the server is unavailable'
      return undefined
    })
    show(msg(plan()))
    await settle()
    flushSync()
    const retry = card().querySelector<HTMLButtonElement>('.forecast button.retry')!
    expect(card().querySelector('.forecast')!.textContent).toContain("Couldn't get the forecast.")
    invoke.mockImplementation(async (cmd: string) => (cmd === 'plan_forecast' ? { place: 'Cape Town, South Africa', date: '2026-11-20', code: 0, temp_max: 25, temp_min: 15, rain_chance: 0 } : undefined))
    retry.click()
    await settle()
    flushSync()
    expect(invoke.mock.calls.filter(([c]) => c === 'plan_forecast').length).toBe(2)
    expect(card().querySelector('.retry')).toBeNull()
    expect(card().querySelector('.forecast')!.textContent).toContain('Clear · 15°–25°')
  })

  it('not for a plan beyond reach, one without a place, or with Plan something off', async () => {
    show(msg(plan({ starts_at: '2026-12-30T10:00:00.000Z' })))
    await settle()
    unmount(cmp!)
    show(msg(plan({ place: null }), { id: 'M2' }))
    await settle()
    unmount(cmp!)
    planPrefs.on = false
    show(msg(plan(), { id: 'M3' }))
    await settle()
    cmp = undefined
    expect(invoke.mock.calls.filter(([c]) => c === 'plan_forecast')).toEqual([])
  })
})

describe("its creator's menu", () => {
  it('Edit plan opens the form filled in; Save changes sends the edit', async () => {
    show(msg(plan(), { author_id: 'U1' }), true)
    document.querySelector<HTMLButtonElement>('button[aria-label="More"]')!.click()
    flushSync()
    expect(menuItems()).toEqual(expect.arrayContaining(['Edit plan', 'Cancel plan']))
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent!.trim() === 'Edit plan')!.click()
    flushSync()
    const form = document.querySelector<HTMLElement>('[role=dialog][aria-label="Edit plan"]')!
    const title = form.querySelector<HTMLInputElement>('input[aria-label="Title"]')!
    expect(title.value).toBe("Braai at kim's")
    expect(form.querySelector<HTMLInputElement>('input[aria-label="Place"]')!.value).toBe('Cape Town')
    // no end yet: blank, and none is sent
    expect(form.querySelector<HTMLInputElement>('input[aria-label="Ends (optional)"]')!.value).toBe('')
    title.value = 'Braai (moved)'
    title.dispatchEvent(new Event('input'))
    flushSync()
    invoke.mockResolvedValueOnce(plan({ title: 'Braai (moved)' }))
    const save = [...form.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Save changes')!
    save.click()
    await settle()
    const call = invoke.mock.calls.find(([c]) => c === 'edit_plan')!
    expect(call[1]).toMatchObject({ messageId: 'M1', request: { title: 'Braai (moved)', starts_at: '2026-11-20T16:30:00.000Z', place: 'Cape Town', note: 'bring ice' } })
    expect((call[1] as { request: object }).request).not.toHaveProperty('ends_at')
  })

  it('Cancel plan cancels it; nobody else gets either item', async () => {
    show(msg(plan(), { author_id: 'U1' }), true)
    document.querySelector<HTMLButtonElement>('button[aria-label="More"]')!.click()
    flushSync()
    invoke.mockResolvedValueOnce(plan({ cancelled_at: '2026-11-16T10:00:00.000Z' }))
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent!.trim() === 'Cancel plan')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('cancel_plan', { messageId: 'M1' })
    expect(app.state.messages.C1[0].plan!.cancelled_at).toBe('2026-11-16T10:00:00.000Z')
    unmount(cmp!)
    document.body.innerHTML = ''
    show(msg(plan()))
    document.querySelector<HTMLButtonElement>('button[aria-label="More"]')!.click()
    flushSync()
    expect(menuItems()).not.toContain('Edit plan')
    expect(menuItems()).not.toContain('Cancel plan')
  })
})

describe('the reminder line', () => {
  it('names the plan and its time, with See plan', () => {
    show(msg(null, {
      id: 'R1', kind: 'system', author_id: null, content: '⏰ Braai starts in an hour', mentions: ['U1'],
      plan_reminder: { plan_message_id: 'M1', title: 'Braai', starts_at: '2026-11-16T11:00:00.000Z' },
    }))
    const line = document.querySelector<HTMLElement>('[data-message-id="R1"]')!
    expect(line.textContent).toContain('Braai starts in an hour (Today,')
    expect([...line.querySelectorAll('button')].map((b) => b.textContent)).toContain('See plan')
  })
})
