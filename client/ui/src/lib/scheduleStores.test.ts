// @vitest-environment jsdom
// The scheduled (#299) and reminder (#300) stores: kept from Ready and their events, a failed scheduled message told
// once, a reminder come due alerting once (unless Reminder alerts is off), and the inbox's Mark all read reading them.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { MessageReminder } from './protocol/MessageReminder'
import type { ScheduledMessage } from './protocol/ScheduledMessage'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
const playSound = vi.fn()
vi.mock('./voiceui', async (orig) => ({ ...(await orig<typeof import('./voiceui')>()), playSound: (...a: unknown[]) => playSound(...a) }))

import { noteScheduledEvent, scheduled, scheduledIn } from './scheduled.svelte'
import { noteReminderEvent, openReminder, reminders, seeAllReminders } from './reminders.svelte'
import { setReminderAlerts } from './reminderPrefs.svelte'
import { app, ui } from './store.svelte'
import { emptyState } from './state'
import { windowFocus } from './windowFocus.svelte'
import { notifyPrefs } from './notifyPrefs.svelte'

const sched = (over: Partial<ScheduledMessage> = {}): ScheduledMessage => ({
  id: 'S1', channel_id: 'C1', content: 'see you at 8', reply_to_id: null, send_at: '2026-11-16T18:00:00.000Z',
  created_at: '2026-11-16T10:00:00.000Z', edited_at: null, failed: null, ...over,
})
const rem = (over: Partial<MessageReminder> = {}): MessageReminder => ({
  id: 'R1', message_id: 'M1', channel_id: 'C1', thread_root_id: null, remind_at: '2026-11-16T10:20:00.000Z',
  created_at: '2026-11-16T10:00:00.000Z', due: false, seen: false,
  message: { message_id: 'M1', author_id: 'U2', content: 'the plan for friday', deleted: false }, ...over,
})

beforeEach(() => {
  invoke.mockReset(); invoke.mockResolvedValue(undefined)
  playSound.mockReset()
  scheduled.list = []
  reminders.list = []
  ui.notice = null
  setReminderAlerts(true)
  notifyPrefs.enabled = true
  notifyPrefs.showText = true
  windowFocus.focused = true
  app.state = {
    ...emptyState(),
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } },
  }
})

describe('scheduled messages (#299)', () => {
  it('follow their events, per chat', () => {
    noteScheduledEvent({ t: 'ScheduledSaved', d: { user_id: 'U1', scheduled: sched() } })
    noteScheduledEvent({ t: 'ScheduledSaved', d: { user_id: 'U1', scheduled: sched({ id: 'S2', channel_id: 'C2' }) } })
    expect(scheduledIn('C1').map((s) => s.id)).toEqual(['S1'])
    noteScheduledEvent({ t: 'ScheduledRemoved', d: { user_id: 'U1', id: 'S1', sent: 'M9' } })
    expect(scheduled.list.map((s) => s.id)).toEqual(['S2'])
    expect(ui.notice).toBeNull()
  })

  it('one that could not be sent is told once, and stays', () => {
    noteScheduledEvent({ t: 'ScheduledSaved', d: { user_id: 'U1', scheduled: sched() } })
    noteScheduledEvent({ t: 'ScheduledSaved', d: { user_id: 'U1', scheduled: sched({ failed: "you can't post in that chat any more" }) } })
    expect(ui.notice).toBe("A scheduled message to #general couldn't be sent: you can't post in that chat any more")
    ui.notice = null
    noteScheduledEvent({ t: 'ScheduledSaved', d: { user_id: 'U1', scheduled: sched({ failed: 'still' }) } })
    expect(ui.notice).toBeNull()
    expect(scheduled.list).toHaveLength(1)
  })

  it('away from the window, it is a desktop notification too', () => {
    windowFocus.focused = false
    noteScheduledEvent({ t: 'ScheduledSaved', d: { user_id: 'U1', scheduled: sched({ failed: 'that chat is gone' }) } })
    expect(invoke).toHaveBeenCalledWith('notify_message', { notice: expect.objectContaining({ body: 'that chat is gone', channelId: 'C1', tag: 'scheduled-S1' }) })
  })
})

describe('reminders (#300)', () => {
  it('one come due alerts once: the sound, and a line while Pulse is in focus', () => {
    noteReminderEvent({ t: 'ReminderSaved', d: { user_id: 'U1', reminder: rem() } })
    expect(playSound).not.toHaveBeenCalled()
    noteReminderEvent({ t: 'ReminderSaved', d: { user_id: 'U1', reminder: rem({ due: true }) } })
    expect(playSound).toHaveBeenCalledTimes(1)
    expect(ui.notice).toContain('Reminder: ')
    expect(ui.notice).toContain('the plan for friday')
    noteReminderEvent({ t: 'ReminderSaved', d: { user_id: 'U1', reminder: rem({ due: true }) } })
    expect(playSound).toHaveBeenCalledTimes(1)
  })

  it('away from the window: a notification that opens the message', () => {
    windowFocus.focused = false
    noteReminderEvent({ t: 'ReminderSaved', d: { user_id: 'U1', reminder: rem({ due: true }) } })
    expect(invoke).toHaveBeenCalledWith('notify_message', { notice: expect.objectContaining({ title: 'Reminder', channelId: 'C1', messageId: 'M1', tag: 'reminder-R1' }) })
  })

  it('Reminder alerts off: it only lands in the inbox', () => {
    setReminderAlerts(false)
    windowFocus.focused = false
    noteReminderEvent({ t: 'ReminderSaved', d: { user_id: 'U1', reminder: rem({ due: true }) } })
    expect(playSound).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
    expect(reminders.list[0].due).toBe(true)
  })

  it('opening a due one reads it on every device; Mark all read reads the rest', () => {
    reminders.list = [rem({ due: true }), rem({ id: 'R2', due: true }), rem({ id: 'R3' })]
    openReminder(reminders.list[0])
    expect(invoke).toHaveBeenCalledWith('reminder_seen', { id: 'R1' })
    expect(reminders.list.find((r) => r.id === 'R1')!.seen).toBe(true)
    seeAllReminders()
    expect(reminders.list.map((r) => [r.id, r.seen])).toEqual([['R1', true], ['R2', true], ['R3', false]])
    noteReminderEvent({ t: 'ReminderRemoved', d: { user_id: 'U1', id: 'R2' } })
    expect(reminders.list.map((r) => r.id)).toEqual(['R1', 'R3'])
  })
})
