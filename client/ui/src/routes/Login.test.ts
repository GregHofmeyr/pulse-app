// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import Login from './Login.svelte'
import { inviteLinks } from '../lib/inviteLinks.svelte'
import { nextTask } from '../lib/testSettle'

const USER = { id: 'u1', username: 'alex' }
let cmp: ReturnType<typeof mount> | undefined
const onAuthed = vi.fn()
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
  await nextTask(); flushSync()
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
  it('the password field has an eye that shows what is typed, signing in and creating an account', () => {
    cmp = mount(Login, { target: document.body, props: { onAuthed } })
    flushSync()
    for (const mode of ['login', 'register']) {
      const eye = document.querySelector<HTMLButtonElement>('button[aria-label="Show password"]')
      expect(eye, mode).not.toBeNull()
      eye!.click(); flushSync()
      expect(document.querySelector('input[autocomplete$=-password]')!.getAttribute('type'), mode).toBe('text')
      document.querySelector<HTMLButtonElement>('button[aria-label="Hide password"]')!.click(); flushSync()
      ;[...document.querySelectorAll<HTMLButtonElement>('button.link')].find((b) => b.textContent?.includes('Create an account'))?.click()
      flushSync()
    }
    expect(document.querySelector('input[autocomplete=new-password]')).not.toBeNull()
  })

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

  it('Back returns to the password step and drops the pending ticket', async () => {
    await toCodeStep()
    expect(calls('cancel_login')).toHaveLength(0)
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Back')!.click()
    flushSync()
    expect(document.querySelector('input[type=password]')).not.toBeNull()
    expect(calls('cancel_login')).toHaveLength(1)
  })

  it('Back still goes back if dropping the ticket fails', async () => {
    await toCodeStep()
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'cancel_login') throw new Error('core gone')
      return undefined
    })
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Back')!.click()
    await nextTask(); flushSync()
    expect(document.querySelector('input[type=password]')).not.toBeNull()
    expect(document.querySelector('[role=alert]')).toBeNull()
  })

  it('Back is disabled while the code is being verified', async () => {
    await toCodeStep()
    invoke.mockImplementation(() => new Promise(() => {}))
    type('input', '123456')
    await submit()
    const back = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Back')!
    expect(back.disabled).toBe(true)
  })

  describe('signing up with an invite (#80, #476)', () => {
    const toRegister = () => {
      cmp = mount(Login, { target: document.body, props: { onAuthed } })
      flushSync()
      ;[...document.querySelectorAll<HTMLButtonElement>('button.link')].find((b) => b.textContent?.includes('Create an account'))!.click()
      flushSync()
    }
    const inviteInput = 'input[autocomplete=off]:not(.hp)'

    it('a pasted invite link fills in its server and its code', async () => {
      invoke.mockResolvedValue(USER)
      toRegister()
      type(inviteInput, '  https://pulse.example/i/abcDEF2345  ')
      expect(document.querySelector<HTMLInputElement>('input[autocomplete=url]')!.value).toBe('https://pulse.example')
      expect(document.querySelector<HTMLInputElement>(inviteInput)!.value).toBe('abcDEF2345')
      type('input[autocomplete=username]', 'kim')
      type('input[type=password]', 'hunter2hunter2')
      await submit()
      expect(calls('register')[0][1]).toEqual({
        serverUrl: 'https://pulse.example', inviteCode: 'abcDEF2345', username: 'kim', password: 'hunter2hunter2', website: '',
      })
    })

    it('the hidden field is out of reach and sent as typed', async () => {
      invoke.mockResolvedValue(USER)
      toRegister()
      const hp = document.querySelector<HTMLInputElement>('input.hp')!
      expect(hp.tabIndex).toBe(-1)
      expect(hp.getAttribute('aria-hidden')).toBe('true')
      type(inviteInput, 'abcDEF2345')
      type('input.hp', 'bot.example')
      type('input[autocomplete=username]', 'kim')
      type('input[type=password]', 'hunter2hunter2')
      await submit()
      expect((calls('register')[0][1] as { website: string }).website).toBe('bot.example')
    })

    it('an invite link that opened Pulse while signed out becomes the sign-up code', () => {
      cmp = mount(Login, { target: document.body, props: { onAuthed } })
      flushSync()
      inviteLinks.code = 'abcDEF2345'
      flushSync()
      expect(document.querySelector<HTMLInputElement>(inviteInput)!.value).toBe('abcDEF2345')
      expect(document.querySelector('input[autocomplete=new-password]')).not.toBeNull()
      expect(inviteLinks.code).toBeNull()
    })
  })

  it('signed out, says an update is waiting when the server publishes a newer version (#460)', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'update_signed_out' ? { version: '9.9.9' } : undefined))
    cmp = mount(Login, { target: document.body, props: { onAuthed } })
    await nextTask(); flushSync()
    expect(calls('update_signed_out')).toHaveLength(1)
    expect((calls('update_signed_out')[0][1] as { server: string }).server).toBeTruthy()
    expect(text()).toContain('An update is available. Sign in to get it.')
  })

  it('says nothing about updates when nothing newer is published, or the ask fails', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'update_signed_out' ? null : undefined))
    cmp = mount(Login, { target: document.body, props: { onAuthed } })
    await nextTask(); flushSync()
    expect(text()).not.toContain('An update is available')
    unmount(cmp)
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'update_signed_out') throw new Error('offline')
      return undefined
    })
    cmp = mount(Login, { target: document.body, props: { onAuthed } })
    await nextTask(); flushSync()
    expect(text()).not.toContain('An update is available')
  })
})
