// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import Login from './Login.svelte'

const USER = { id: 'u1', username: 'alex' }
let cmp: ReturnType<typeof mount> | undefined
const onAuthed = vi.fn()
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const text = () => document.body.textContent ?? ''
const type = (sel: string, v: string) => {
  const el = document.querySelector<HTMLInputElement>(sel)!
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const submit = async () => {
  document.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  await settle(); flushSync()
}
const toCodeStep = async () => {
  invoke.mockImplementation(async (cmd: string) => (cmd === 'login' ? { status: 'mfa_required' } : undefined))
  cmp = mount(Login, { target: document.body, props: { onAuthed } })
  flushSync()
  type('input[autocomplete=username]', 'alex')
  type('input[type=password]', 'pw-for-alex')
  await submit()
}

beforeEach(() => { invoke.mockReset(); onAuthed.mockReset() })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('Login', () => {
  it('ready signs straight in', async () => {
    invoke.mockResolvedValue({ status: 'ready', user: USER })
    cmp = mount(Login, { target: document.body, props: { onAuthed } })
    flushSync()
    type('input[autocomplete=username]', 'alex')
    type('input[type=password]', 'pw-for-alex')
    await submit()
    expect(onAuthed).toHaveBeenCalledWith(USER)
  })

  it('mfa_required shows only the code field, and the password is gone', async () => {
    await toCodeStep()
    const input = document.querySelector<HTMLInputElement>('input')!
    expect(document.querySelectorAll('input')).toHaveLength(1)
    expect(input.getAttribute('autocomplete')).toBe('one-time-code')
    expect(input.placeholder).toBe('6-digit code or recovery code')
    expect(text()).toContain('Code')
    expect([...document.querySelectorAll('button')].map((b) => b.textContent)).toContain('Verify')
    expect([...document.querySelectorAll('button')].map((b) => b.textContent)).toContain('Back')
    expect(onAuthed).not.toHaveBeenCalled()
  })

  it('a right code signs in', async () => {
    await toCodeStep()
    invoke.mockResolvedValue({ status: 'ready', user: USER })
    type('input', ' 123456 ')
    await submit()
    expect(calls('login_mfa')[0][1]).toEqual({ code: '123456' })
    expect(onAuthed).toHaveBeenCalledWith(USER)
  })

  it('a wrong code shows the error and keeps the code step', async () => {
    await toCodeStep()
    invoke.mockRejectedValue("That code didn't work.")
    type('input', '000000')
    await submit()
    expect(document.querySelector('[role=alert]')?.textContent).toBe("That code didn't work.")
    expect(document.querySelectorAll('input')).toHaveLength(1)
  })

  it('expired goes back to the password step with the message and an empty password', async () => {
    await toCodeStep()
    invoke.mockResolvedValue({ status: 'expired' })
    type('input', '123456')
    await submit()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('That sign-in expired. Log in again.')
    expect(document.querySelector<HTMLInputElement>('input[type=password]')!.value).toBe('')
    expect(onAuthed).not.toHaveBeenCalled()
  })

  it('Back returns to the password step', async () => {
    await toCodeStep()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Back')!.click()
    flushSync()
    expect(document.querySelector('input[type=password]')).not.toBeNull()
  })
})
