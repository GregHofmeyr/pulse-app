// @vitest-environment jsdom
// Settings › Chat: Plan reminders (#399) is kept by the server (every device), shows at once, and goes back with the
// reason when the server refuses.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ChatSettings from './ChatSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

let cmp: ReturnType<typeof mount> | undefined
const field = () => [...document.querySelectorAll<HTMLElement>('.settings-field')].find((f) => f.textContent!.includes('PLAN REMINDERS'))!
const pick = (label: string) => [...field().querySelectorAll<HTMLButtonElement>('button[role=radio]')].find((b) => b.textContent === label)!
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  app.state = emptyState()
  cmp = mount(ChatSettings, { target: document.body })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Plan reminders', () => {
  it('on by default; Off saves it on the server for every device', async () => {
    expect(pick('On').getAttribute('aria-checked')).toBe('true')
    expect(field().textContent).toContain('Every device.')
    pick('Off').click()
    flushSync()
    expect(app.state.planReminders).toBe(false)
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_plan_reminders', { on: false })
    expect(pick('Off').getAttribute('aria-checked')).toBe('true')
  })

  it('a refusal puts it back and says why', async () => {
    invoke.mockRejectedValueOnce('try again in a moment')
    pick('Off').click()
    await settle()
    flushSync()
    expect(app.state.planReminders).toBe(true)
    expect(field().querySelector('[role=alert]')!.textContent).toBe('try again in a moment')
  })
})
