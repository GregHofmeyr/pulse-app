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
const realClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
afterEach(() => {
  if (realClipboard) Object.defineProperty(navigator, 'clipboard', realClipboard)
  else delete (navigator as unknown as Record<string, unknown>).clipboard
  if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = ''
})

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

  it('QR modules are a whole number of pixels', async () => {
    answers(OFF, { two_factor_setup: () => ({ secret: 'K', qr: { size: 3, modules: [true, false, false, false, false, false, false, false, false] } }) })
    await show()
    click('Set up'); type('input', 'pw'); await submit()
    const svg = document.querySelector('svg')!
    expect(svg.getAttribute('width')).toBe('198') // 11 modules x 18px
    expect(svg.getAttribute('height')).toBe('198')
  })

  it('Copy says Copied, or says to copy by hand when it fails', async () => {
    const enabled = { two_factor_setup: () => ({ secret: 'K', qr: { size: 1, modules: [true] } }), two_factor_enable: () => ({ recovery_codes: CODES, signed_out: 0 }) }
    answers(OFF, enabled)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: vi.fn(async () => {}) }, configurable: true })
    await show()
    click('Set up'); type('input', 'pw'); await submit()
    type('input', '123456'); await submit()
    click('Copy'); await settle(); flushSync()
    expect(button('Copied')).toBeTruthy()
    unmount(cmp!); document.body.innerHTML = ''
    answers(OFF, enabled)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: vi.fn(async () => { throw new Error('denied') }) }, configurable: true })
    await show()
    click('Set up'); type('input', 'pw'); await submit()
    type('input', '123456'); await submit()
    click('Copy'); await settle(); flushSync()
    expect(text()).toContain("Couldn't copy: select and copy them by hand.")
    expect(button('Copy')).toBeTruthy()
  })

  it('a failed status call keeps the heading, shows the error and Try again', async () => {
    let fail = true
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd !== 'two_factor_status') return undefined
      if (fail) throw 'Network down.'
      return OFF
    })
    await show()
    expect(text()).toContain('Two-factor authentication')
    expect(document.querySelector('h3')).not.toBeNull()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('Network down.')
    fail = false
    click('Try again'); await settle(); flushSync()
    expect(calls('two_factor_status')).toHaveLength(2)
    expect(button('Set up')).toBeTruthy()
    expect(document.querySelector('[role=alert]')).toBeNull()
  })

  it('if the refresh after "I\'ve saved these" fails, no stale status is shown', async () => {
    let n = 0
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'two_factor_status') { if (++n > 1) throw 'Network down.'; return OFF }
      if (cmd === 'two_factor_setup') return { secret: 'K', qr: { size: 1, modules: [true] } }
      if (cmd === 'two_factor_enable') return { recovery_codes: CODES, signed_out: 0 }
    })
    await show()
    click('Set up'); type('input', 'pw'); await submit()
    type('input', '123456'); await submit()
    click("I've saved these"); await settle(); flushSync()
    expect(button('Set up')).toBeUndefined()
    expect(button('Try again')).toBeTruthy()
  })

  it('if the refresh after turning off fails, no stale status is shown', async () => {
    let n = 0
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'two_factor_status') { if (++n > 1) throw 'Network down.'; return ON }
    })
    await show()
    click('Turn off'); type('input', '123456'); await submit()
    expect(button('Turn off')).toBeUndefined()
    expect(button('Try again')).toBeTruthy()
  })

  it('new recovery codes with a wrong code shows the error and shows no codes', async () => {
    answers(ON, { two_factor_regenerate: () => { throw "That code didn't work." } })
    await show()
    click('New recovery codes'); type('input', '111111'); await submit()
    expect(document.querySelector('[role=alert]')?.textContent).toBe("That code didn't work.")
    expect(document.querySelector('ul')).toBeNull()
  })

  it('Cancel on the regenerate and turn-off views forgets the code and returns to status', async () => {
    answers(ON)
    await show()
    for (const [open, again] of [['New recovery codes', 'New recovery codes'], ['Turn off', 'Turn off']]) {
      click(open); type('input', '123456')
      click('Cancel')
      expect(document.querySelector('input')).toBeNull()
      expect(text()).toContain('7 recovery codes left')
      click(again)
      expect(document.querySelector<HTMLInputElement>('input')!.value).toBe('')
      click('Cancel')
    }
    expect(calls('two_factor_regenerate')).toHaveLength(0)
    expect(calls('two_factor_disable')).toHaveLength(0)
  })
})
