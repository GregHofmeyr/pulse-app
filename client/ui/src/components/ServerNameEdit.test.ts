// @vitest-environment jsdom
// Renaming a server in place (#55): Enter saves (trimmed; the same name or a blank one just ends it), Escape puts it
// back, leaving the field saves, and a refusal keeps the field with why.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import ServerNameEdit from './ServerNameEdit.svelte'

let cmp: ReturnType<typeof mount> | undefined
let onDone = vi.fn()
const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve()
  flushSync()
}
function render() {
  onDone = vi.fn()
  cmp = mount(ServerNameEdit, { target: document.body, props: { server: { id: 'S1', name: 'Alpha', icon_hash: null }, onDone } })
  flushSync()
  return document.querySelector<HTMLInputElement>('input[aria-label="Rename Alpha"]')!
}
const type = (el: HTMLInputElement, v: string) => {
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const key = (el: Element, k: string) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue({ id: 'S1', name: 'Lounge', icon_hash: null })
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ServerNameEdit', () => {
  it('starts focused on the name; Enter saves it trimmed', async () => {
    const field = render()
    expect(document.activeElement).toBe(field)
    expect(field.value).toBe('Alpha')
    type(field, '  Lounge ')
    key(field, 'Enter')
    await settle()
    expect(invoke).toHaveBeenCalledWith('rename_server', { serverId: 'S1', name: 'Lounge' })
    expect(onDone).toHaveBeenCalledWith(true)
  })

  it('Escape puts it back without saving, and goes no further', () => {
    const field = render()
    const outer = vi.fn()
    window.addEventListener('keydown', outer)
    type(field, 'Lounge')
    key(field, 'Escape')
    expect(invoke).not.toHaveBeenCalled()
    expect(onDone).toHaveBeenCalledWith(true)
    expect(outer).not.toHaveBeenCalled()
    window.removeEventListener('keydown', outer)
  })

  it('the same name or a blank one just ends it', async () => {
    const field = render()
    type(field, '   ')
    key(field, 'Enter')
    await settle()
    expect(invoke).not.toHaveBeenCalled()
    expect(onDone).toHaveBeenCalledTimes(1)
  })

  it('leaving the field saves', async () => {
    const field = render()
    type(field, 'Lounge')
    field.dispatchEvent(new FocusEvent('blur'))
    await settle()
    expect(invoke).toHaveBeenCalledWith('rename_server', { serverId: 'S1', name: 'Lounge' })
    expect(onDone).toHaveBeenCalledWith(false)
  })

  it('a refusal keeps the field and says why', async () => {
    invoke.mockRejectedValue({ message: "Only the server's owner can rename it." })
    const field = render()
    type(field, 'Lounge')
    key(field, 'Enter')
    await settle()
    expect(onDone).not.toHaveBeenCalled()
    expect(document.querySelector('[role=alert]')!.textContent).toContain('owner can rename')
  })
})
