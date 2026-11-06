// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ReportBug from './ReportBug.svelte'

let cmp: ReturnType<typeof mount> | undefined
function show(props: Record<string, unknown> = {}) {
  cmp = mount(ReportBug, { target: document.body, props: { onClose: () => {}, ...props } })
  flushSync()
}
beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd) => (cmd === 'submit_report' || cmd === 'submit_idea' ? 'TVWXYZ' : undefined))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const dialog = () => document.querySelector<HTMLElement>('[role=dialog]')!
const radio = (name: string) => [...dialog().querySelectorAll<HTMLButtonElement>('[role=radio]')].find((b) => b.textContent?.trim() === name)!
const area = () => dialog().querySelector('textarea')!
const button = (name: string) => [...dialog().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === name)
const settle = async () => { await new Promise((r) => setTimeout(r, 0)); flushSync() }
function type(text: string) {
  area().value = text
  area().dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const send = async () => { button('Send')!.click(); flushSync(); await settle() }

describe('Report a bug / Suggest an idea (#218)', () => {
  it('opens on Bug, with the Bug / Idea switch at the top', () => {
    show()
    const group = dialog().querySelector('[role=radiogroup]')!
    expect(group.getAttribute('aria-label')).toBe('Report type')
    expect([...group.querySelectorAll('[role=radio]')].map((b) => b.textContent?.trim())).toEqual(['Bug', 'Idea'])
    expect(radio('Bug').getAttribute('aria-checked')).toBe('true')
    expect(dialog().getAttribute('aria-label')).toBe('Report a bug')
    expect(dialog().textContent).toContain('recent app log')
    // the switch sits above the text box
    expect(group.compareDocumentPosition(area()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('a bug is sent with its diagnostics, as before', async () => {
    show()
    type('voice cut out')
    await send()
    expect(invoke).toHaveBeenCalledWith('submit_report', { text: 'voice cut out' })
    expect(dialog().textContent).toContain('Thanks! Report #TVWXYZ sent.')
  })

  it('Idea: a bigger box, says nothing else is attached, and sends as an idea', async () => {
    show()
    const bugRows = Number(area().getAttribute('rows'))
    radio('Idea').click()
    flushSync()
    expect(invoke).not.toHaveBeenCalled() // the switch doesn't submit the form
    expect(radio('Idea').getAttribute('aria-checked')).toBe('true')
    expect(dialog().getAttribute('aria-label')).toBe('Suggest an idea')
    expect(dialog().querySelector('h1')?.textContent).toBe('Suggest an idea')
    expect(Number(area().getAttribute('rows'))).toBeGreaterThan(bugRows)
    expect(area().classList.contains('idea')).toBe(true)
    expect(dialog().textContent).not.toContain('app log')
    expect(dialog().textContent).toContain('No logs or system info')
    type('Let me pin a clip')
    await send()
    expect(invoke).toHaveBeenCalledWith('submit_idea', { text: 'Let me pin a clip' })
    expect(invoke).not.toHaveBeenCalledWith('submit_report', expect.anything())
    expect(dialog().textContent).toContain('Thanks! Idea #TVWXYZ sent.')
  })

  it('switching keeps what was typed', () => {
    show()
    type('half a thought')
    radio('Idea').click()
    flushSync()
    expect(area().value).toBe('half a thought')
  })

  it("an idea that couldn't be sent offers Send again, not saving a report file", async () => {
    invoke.mockImplementation(async () => { throw "can't reach the server: dns error" })
    show()
    radio('Idea').click()
    flushSync()
    type('x')
    await send()
    expect(dialog().textContent).toContain("Couldn't reach the server.")
    expect(button('Save report')).toBeUndefined()
    expect(button('Send')?.disabled).toBe(false)
  })

  it('signed out (save only), there is no Idea switch', () => {
    show({ saveOnly: true })
    expect(dialog().querySelector('[role=radiogroup]')).toBeNull()
    expect(dialog().getAttribute('aria-label')).toBe('Report a bug')
  })
})
