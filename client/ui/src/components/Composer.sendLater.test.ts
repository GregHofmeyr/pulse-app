// @vitest-environment jsdom
// Send later (#299): the composer's clock button, its quick picks scheduling what's in the box, and why it can't when
// the box is empty or holds files.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import Composer from './Composer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { serialize } from '../lib/composerDom'
import { plusItem } from './plusMenuHarness'
import { flushMicrotasks } from '../lib/testSettle'

const NOW = new Date(2026, 10, 16, 10, 0).getTime()
let cmp: ReturnType<typeof mount> | undefined
const onSchedule = vi.fn(async (_text: string, _at: string) => {})
const onShowScheduled = vi.fn()

function show(extra: Record<string, unknown> = {}) {
  app.state = { ...emptyState() }
  cmp = mount(Composer, {
    target: document.body,
    props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend: () => {}, onTyping: () => {}, onSchedule, onShowScheduled, ...extra },
  })
  flushSync()
  return document.querySelector<HTMLElement>('[role="textbox"]')!
}
function type(box: HTMLElement, text: string) {
  box.replaceChildren(document.createTextNode(text))
  box.dispatchEvent(new InputEvent('input', { bubbles: true }))
  flushSync()
}
// Send later… lives in the + menu (#461)
const clock = () => plusItem('later')
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Send later"]')
const entries = () => [...(menu()?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])].map((b) => b.textContent?.trim())

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  onSchedule.mockClear()
  onShowScheduled.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('Send later (#299)', () => {
  it('only where messages can be scheduled', () => {
    show({ onSchedule: undefined })
    expect(clock()).toBeNull()
  })

  it('is the + menu\'s last row, after a divider (#461)', () => {
    show({ files: true })
    const item = clock()!
    expect(item.textContent!.trim()).toBe('Send later…')
    expect(item.previousElementSibling!.getAttribute('role')).toBe('separator')
    expect(item.nextElementSibling).toBeNull()
  })

  it('schedules the text for a quick pick, and empties the box', async () => {
    const box = show()
    type(box, 'see you at 8')
    clock()!.click(); flushSync()
    expect(entries()).toEqual(['In 1 hour', 'Tonight at 20:00', 'Tomorrow at 09:00', 'Pick a date and time…'])
    ;[...menu()!.querySelectorAll<HTMLButtonElement>('[role=menuitem]')][1].click()
    await flushMicrotasks(6)
    expect(onSchedule).toHaveBeenCalledWith('see you at 8', new Date(2026, 10, 16, 20, 0).toISOString())
    flushSync()
    expect(serialize(box)).toBe('')
    expect(menu()).toBeNull()
  })

  it('says why when there is nothing to schedule, and offers this chat\'s list', () => {
    show({ scheduledCount: 2 })
    clock()!.click(); flushSync()
    expect(menu()!.textContent).toContain('Write a message first')
    expect(entries()).toEqual(['Scheduled in this chat (2)'])
    ;[...menu()!.querySelectorAll<HTMLButtonElement>('[role=menuitem]')][0].click()
    flushSync()
    expect(onShowScheduled).toHaveBeenCalled()
  })

  it('a refusal stays in the menu, the text in the box', async () => {
    onSchedule.mockRejectedValueOnce('you can have up to 50 scheduled messages')
    const box = show()
    type(box, 'hi')
    clock()!.click(); flushSync()
    ;[...menu()!.querySelectorAll<HTMLButtonElement>('[role=menuitem]')][0].click()
    await flushMicrotasks(6)
    flushSync()
    expect(menu()!.textContent).toContain('up to 50 scheduled messages')
    expect(serialize(box)).toBe('hi')
  })
})
