// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import TwoFactorSettings from './TwoFactorSettings.svelte'

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const show = async () => {
  cmp = mount(TwoFactorSettings, { target: document.body })
  flushSync()
  await settle(); flushSync()
}
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const button = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === t)
const click = (t: string) => { button(t)!.click(); flushSync() }
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
const text = () => document.body.textContent ?? ''

type Status = { enabled: boolean; available: boolean; recovery_codes_left: number }
const answers = (status: Status, more: Record<string, () => unknown> = {}) =>
  invoke.mockImplementation(async (cmd: string) => (cmd === 'two_factor_status' ? status : more[cmd]?.()))
const OFF = { enabled: false, available: true, recovery_codes_left: 0 }
const ON = { enabled: true, available: true, recovery_codes_left: 7 }
const CODES = ['aaaa-bbbb-cccc-dddd', 'eeee-ffff-gggg-hhhh']

beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(undefined) })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('TwoFactorSettings', () => {
  it('unavailable: the sentence and no button', async () => {
    answers({ enabled: false, available: false, recovery_codes_left: 0 })
    await show()
    expect(text()).toContain("Two-factor authentication isn't set up on this server yet.")
    expect(document.querySelectorAll('button')).toHaveLength(0)
  })

  it('setup: password, QR, code, recovery codes once, then gone for good', async () => {
    // 3x3 matrix with 4 dark modules
    const modules = [true, false, true, false, true, false, true, false, false]
    answers(OFF, {
      two_factor_setup: () => ({ secret: 'ABCD EFGH', qr: { size: 3, modules } }),
      two_factor_enable: () => ({ recovery_codes: CODES, signed_out: 2 }),
    })
    await show()
    click('Set up')
    type('input[type=password]', 'hunter22hunter')
    await submit()
    expect(calls('two_factor_setup')[0][1]).toEqual({ password: 'hunter22hunter' })
    expect(text()).toContain('ABCD EFGH')
    const svg = document.querySelector('svg')!
    expect(svg.getAttribute('shape-rendering')).toBe('crispEdges')
    expect(svg.getAttribute('viewBox')).toBe('0 0 11 11')
    const rects = [...svg.querySelectorAll('rect')]
    expect(rects.filter((r) => r.getAttribute('fill') === '#000')).toHaveLength(4)
    expect(rects.filter((r) => r.getAttribute('fill') === '#fff')).toHaveLength(1)

    type('input', ' 123456 ')
    await submit()
    expect(calls('two_factor_enable')[0][1]).toEqual({ code: '123456' })
    for (const c of CODES) expect(text()).toContain(c)
    expect(text()).toContain('Signed out 2 other devices.')
    expect(document.querySelector('input')).toBeNull()

    click("I've saved these")
    await settle(); flushSync()
    for (const c of CODES) expect(text()).not.toContain(c)

    // a fresh mount never shows them again
    unmount(cmp!); document.body.innerHTML = ''
    answers(ON)
    await show()
    for (const c of CODES) expect(text()).not.toContain(c)
    expect(text()).toContain('7 recovery codes left')
  })

  it('says when nobody else was signed in, and Copy writes the codes', async () => {
    const writeText = vi.fn(async () => {})
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    answers(OFF, {
      two_factor_setup: () => ({ secret: 'K', qr: { size: 1, modules: [true] } }),
      two_factor_enable: () => ({ recovery_codes: CODES, signed_out: 0 }),
    })
    await show()
    click('Set up'); type('input', 'pw'); await submit()
    type('input', '123456'); await submit()
    expect(text()).toContain('No other devices were signed in.')
    click('Copy'); await settle()
    expect(writeText).toHaveBeenCalledWith(CODES.join('\n'))
  })

  it('a wrong setup code shows the error and stays on the code step', async () => {
    answers(OFF, {
      two_factor_setup: () => ({ secret: 'K', qr: { size: 1, modules: [true] } }),
      two_factor_enable: () => { throw "That code didn't work." },
    })
    await show()
    click('Set up'); type('input', 'pw'); await submit()
    type('input', '000000'); await submit()
    expect(document.querySelector('[role=alert]')?.textContent).toBe("That code didn't work.")
    expect(document.querySelector('svg')).not.toBeNull()
  })

  it('a wrong password shows the error', async () => {
    answers(OFF, { two_factor_setup: () => { throw 'Wrong password.' } })
    await show()
    click('Set up'); type('input', 'nope'); await submit()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('Wrong password.')
    expect(document.querySelector('svg')).toBeNull()
  })

  it('cancel forgets typed values and the pending secret', async () => {
    answers(OFF, { two_factor_setup: () => ({ secret: 'SECRETKEY', qr: { size: 1, modules: [true] } }) })
    await show()
    click('Set up'); type('input', 'pw'); await submit()
    expect(text()).toContain('SECRETKEY')
    click('Cancel')
    expect(text()).not.toContain('SECRETKEY')
    expect(button('Set up')).toBeTruthy()
    click('Set up')
    expect(document.querySelector<HTMLInputElement>('input')!.value).toBe('')
  })

  it('new recovery codes: asks for a code, shows the new set once', async () => {
    answers(ON, { two_factor_regenerate: () => ({ recovery_codes: CODES }) })
    await show()
    click('New recovery codes')
    type('input', '654321'); await submit()
    expect(calls('two_factor_regenerate')[0][1]).toEqual({ code: '654321' })
    for (const c of CODES) expect(text()).toContain(c)
    click("I've saved these"); await settle(); flushSync()
    for (const c of CODES) expect(text()).not.toContain(c)
  })

  it('turn off: sends the code (or a recovery code) and returns to the off state', async () => {
    let on = true
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'two_factor_status' ? (on ? ON : OFF) : cmd === 'two_factor_disable' ? void (on = false) : undefined)
    await show()
    click('Turn off')
    type('input', 'aaaa-bbbb-cccc-dddd'); await submit()
    expect(calls('two_factor_disable')[0][1]).toEqual({ code: 'aaaa-bbbb-cccc-dddd' })
    expect(button('Set up')).toBeTruthy()
  })

  it('turn off with a wrong code shows the error and stays on', async () => {
    answers(ON, { two_factor_disable: () => { throw "That code didn't work." } })
    await show()
    click('Turn off'); type('input', '111111'); await submit()
    expect(document.querySelector('[role=alert]')?.textContent).toBe("That code didn't work.")
    expect(document.querySelector('input')).not.toBeNull()
  })
})
