// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ChangePassword from './ChangePassword.svelte'

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const show = async () => {
  cmp = mount(ChangePassword, { target: document.body })
  flushSync()
  await settle(); flushSync()
}
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const button = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === t)
const click = (t: string) => { button(t)!.click(); flushSync() }
const type = (label: string, v: string) => {
  const el = [...document.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith(label))!
    .querySelector('input')!
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const submit = async () => {
  document.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  await settle(); flushSync()
}
const text = () => document.body.textContent ?? ''
const inputs = () => [...document.querySelectorAll<HTMLInputElement>('input')]

type Status = { enabled: boolean; available: boolean; recovery_codes_left: number }
const answers = (status: Status | Error, more: Record<string, () => unknown> = {}) =>
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'two_factor_status') {
      if (status instanceof Error) throw status.message
      return status
    }
    return more[cmd]?.()
  })
const OFF = { enabled: false, available: true, recovery_codes_left: 0 }
const ON = { enabled: true, available: true, recovery_codes_left: 7 }

beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(undefined) })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

const fill = (current: string, next: string, confirm: string) => {
  type('Current password', current)
  type('New password', next)
  type('Confirm new password', confirm)
}

describe('ChangePassword', () => {
  it('opens a form and submits the passwords to the core, then forgets them', async () => {
    answers(OFF)
    await show()
    expect(inputs()).toHaveLength(0)
    click('Change password')
    expect(inputs().every((i) => i.type === 'password')).toBe(true)
    expect(text()).not.toContain('Code')
    fill('old password', 'new password 1', 'new password 1')
    await submit()
    expect(calls('change_password')).toHaveLength(1)
    expect(calls('change_password')[0][1]).toEqual({
      currentPassword: 'old password', newPassword: 'new password 1', code: null,
    })
    expect(text()).toContain('Password changed. Other devices were signed out.')
    expect(inputs()).toHaveLength(0)
    // Opening again starts empty.
    click('Change password')
    expect(inputs().every((i) => i.value === '')).toBe(true)
  })

  it('a confirmation that does not match blocks the submit', async () => {
    answers(OFF)
    await show()
    click('Change password')
    fill('old password', 'new password 1', 'new password 2')
    await submit()
    expect(calls('change_password')).toHaveLength(0)
    expect(document.querySelector('[role=alert]')?.textContent).toBe("The new passwords don't match.")
  })

  it('with two-factor on, asks for a code and sends it trimmed', async () => {
    answers(ON)
    await show()
    click('Change password')
    fill('old password', 'new password 1', 'new password 1')
    type('Code', ' 123456 ')
    await submit()
    expect(calls('change_password')[0][1]).toEqual({
      currentPassword: 'old password', newPassword: 'new password 1', code: '123456',
    })
  })

  it('if the two-factor state is unknown, offers the code field anyway', async () => {
    answers(new Error('offline'))
    await show()
    click('Change password')
    expect(text()).toContain('Code')
    fill('old password', 'new password 1', 'new password 1')
    await submit()
    expect(calls('change_password')[0][1]).toEqual({
      currentPassword: 'old password', newPassword: 'new password 1', code: null,
    })
  })

  it('shows the core error and keeps the form', async () => {
    answers(OFF, { change_password: () => { throw 'wrong password' } })
    await show()
    click('Change password')
    fill('nope nope', 'new password 1', 'new password 1')
    await submit()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('wrong password')
    expect(inputs().length).toBeGreaterThan(0)
    click('Cancel')
    expect(inputs()).toHaveLength(0)
    expect(document.querySelector('[role=alert]')).toBeNull()
  })
})
