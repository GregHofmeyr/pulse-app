// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ServerSettings from './ServerSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ServerSettings dialog', () => {
  it('focuses into the dialog, closes on Esc, and gives focus back', () => {
    app.state = emptyState()
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    const dialog = document.querySelector('[role=dialog]')!
    expect(dialog.contains(document.activeElement)).toBe(true)
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(onClose).toHaveBeenCalledTimes(1)
    unmount(cmp)
    cmp = undefined
    expect(document.activeElement).toBe(opener)
  })

  // Svelte delegates onkeydown to the mount root, so these run the real inner handlers first.
  const esc = (el: Element) => el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
  const settle = () => new Promise((r) => setTimeout(r, 0))
  const emoji = { id: 'X1', server_id: 'S1', name: 'pog', hash: 'h1', still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00.000Z' }

  it('Esc in the add form closes the form, not the dialog', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue({ hash: 'nh', content_type: 'image/webp', width: 128, height: 128, frames: 1, animated: false, still_hash: null })
    app.state = emptyState()
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    const file = document.querySelector<HTMLInputElement>('input[type=file]')!
    Object.defineProperty(file, 'files', { value: [new File([new Uint8Array([1])], 'a.png')], configurable: true })
    file.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    flushSync()
    esc(document.querySelector('input[aria-label="Emoji name"]')!)
    flushSync()
    expect(document.querySelector('input[aria-label="Emoji name"]')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
  })

  it('Esc in the rename input cancels the rename, not the dialog', async () => {
    app.state = { ...emptyState(), customEmoji: { X1: emoji } }
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label="Rename :pog:"]')!.click()
    flushSync()
    esc(document.querySelector('input[aria-label="Rename emoji"]')!)
    flushSync()
    expect(document.querySelector('input[aria-label="Rename emoji"]')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
    esc(document.querySelector('[role=dialog]')!) // nothing inside handled it: now it closes
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('Esc with focus outside the dialog still closes it, and Tab pulls focus back in', () => {
    app.state = emptyState()
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    ;(document.activeElement as HTMLElement).blur()
    const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true })
    document.body.dispatchEvent(tab)
    expect(tab.defaultPrevented).toBe(true)
    expect(document.querySelector('[role=dialog]')!.contains(document.activeElement)).toBe(true)
    esc(document.body)
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
