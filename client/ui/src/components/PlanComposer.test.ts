// @vitest-environment jsdom
// The plan form (#387) picks its day with the shared DatePicker (#643).
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia } from '../lib/testMotion'

vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
const postPlan = vi.fn(async (_c: string, _r: unknown) => ({}))
vi.mock('../lib/plans.svelte', () => ({ postPlan: (c: string, r: unknown) => postPlan(c, r), editPlan: async () => ({}) }))

import PlanComposer, { forgetPlanDrafts } from './PlanComposer.svelte'
import { fieldsToIso } from '../lib/plan'

let cmp: ReturnType<typeof mount> | undefined
const onClose = vi.fn()

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date(2026, 10, 16, 10, 0))
  postPlan.mockClear()
  onClose.mockClear()
  forgetPlanDrafts()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

async function show() {
  const anchor = document.createElement('button')
  document.body.append(anchor)
  cmp = mount(PlanComposer, { target: document.body, props: { anchor, channelId: 'c1', prefill: 'Braai', onClose } })
  flushSync()
  await new Promise((r) => setTimeout(r, 0)) // clickOutside arms
}
const dayBtn = () => document.querySelector<HTMLButtonElement>('.plan-composer button.date-field')!
const picker = () => document.querySelector<HTMLElement>('.date-picker')
const cell = (d: string) => picker()!.querySelector<HTMLButtonElement>(`[data-day="${d}"]`)!
const post = () => document.querySelector<HTMLButtonElement>('.plan-composer .post')!

describe('PlanComposer day (#643)', () => {
  it('uses the shared picker, not a native date input; both time inputs stay', async () => {
    await show()
    expect(document.querySelector('.plan-composer input[type=date]')).toBeNull()
    expect(dayBtn().textContent).toContain('16 Nov 2026')
    expect(document.querySelectorAll('.plan-composer input[type=time]')).toHaveLength(2)
  })

  it('a picked day posts the same instant the native input gave', async () => {
    await show()
    dayBtn().click()
    flushSync()
    const day = cell('2026-11-28')
    day.dispatchEvent(new Event('pointerdown', { bubbles: true })) // a press in the picker isn't outside the form
    day.click()
    flushSync()
    expect(onClose).not.toHaveBeenCalled()
    expect(post().disabled).toBe(false)
    post().click()
    await tick()
    expect(postPlan).toHaveBeenCalledWith('c1', expect.objectContaining({ title: 'Braai', starts_at: fieldsToIso('2026-11-28', '12:00') }))
  })

  it('past days are disabled', async () => {
    await show()
    dayBtn().click()
    flushSync()
    expect(cell('2026-11-15').disabled).toBe(true)
    expect(cell('2026-11-16').disabled).toBe(false)
  })
})
