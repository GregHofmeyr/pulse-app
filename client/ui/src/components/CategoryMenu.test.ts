// @vitest-environment jsdom
// A category heading's right-click menu (#54): Create channel, Rename category…, Delete category, then Move up /
// Move down (disabled at the ends). Keyboard and focus as in the channel and server menus.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import CategoryMenu from './CategoryMenu.svelte'
import type { Category } from '../lib/protocol/Category'

const gaming: Category = { id: 'C2', server_id: 'S1', name: 'Gaming', position: 1 }

let cmp: ReturnType<typeof mount> | undefined
let opener: HTMLButtonElement
const cb = {
  onClose: vi.fn(), onCreateChannel: vi.fn(), onRename: vi.fn(), onMoveUp: vi.fn(), onMoveDown: vi.fn(), onError: vi.fn(),
}

function render(ends: { canMoveUp?: boolean; canMoveDown?: boolean } = {}) {
  opener = document.createElement('button')
  document.body.append(opener)
  for (const f of Object.values(cb)) f.mockReset()
  cmp = mount(CategoryMenu, {
    target: document.body,
    props: { category: gaming, x: 20, y: 50, opener, canMoveUp: ends.canMoveUp ?? true, canMoveDown: ends.canMoveDown ?? true, ...cb },
  })
  flushSync()
}
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Actions for Gaming"]')
const items = () => [...(menu()?.querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]') ?? [])]
const labels = () => items().map((b) => b.textContent?.trim())
const item = (label: string) => items().find((b) => b.textContent?.trim() === label)!
const key = (el: Element, k: string) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('CategoryMenu', () => {
  it('lists create, rename, delete, then the moves after a separator; focus on the first', () => {
    render()
    expect(labels()).toEqual(['Create channel', 'Rename category…', 'Delete category', 'Move up', 'Move down'])
    expect(item('Delete category').classList.contains('danger')).toBe(true)
    expect(item('Move up').previousElementSibling?.classList.contains('menu-sep')).toBe(true)
    expect(menu()!.classList.contains('menu-surface')).toBe(true)
    expect(document.activeElement).toBe(item('Create channel'))
  })

  it('the moves are disabled at the ends, and the arrow keys skip them', () => {
    render({ canMoveUp: true, canMoveDown: false })
    expect(item('Move down').disabled).toBe(true)
    expect(item('Move up').disabled).toBe(false)
    key(item('Create channel'), 'End')
    expect(document.activeElement).toBe(item('Move up'))
    key(item('Move up'), 'ArrowDown')
    expect(document.activeElement).toBe(item('Create channel'))
  })

  it('each item acts and closes the menu', () => {
    render()
    item('Create channel').click()
    expect(cb.onCreateChannel).toHaveBeenCalledTimes(1)
    expect(cb.onClose).toHaveBeenCalledTimes(1)
    unmount(cmp!)
    render()
    item('Rename category…').click()
    expect(cb.onRename).toHaveBeenCalledTimes(1)
    unmount(cmp!)
    render()
    item('Move up').click()
    expect(cb.onMoveUp).toHaveBeenCalledTimes(1)
    unmount(cmp!)
    render()
    item('Move down').click()
    expect(cb.onMoveDown).toHaveBeenCalledTimes(1)
    expect(cb.onClose).toHaveBeenCalled()
  })

  it('Delete category deletes it at once (its channels move up, nothing is lost)', async () => {
    render()
    item('Delete category').click()
    await Promise.resolve()
    expect(invoke).toHaveBeenCalledWith('delete_category', { categoryId: 'C2' })
    expect(cb.onClose).toHaveBeenCalled()
    expect(document.querySelector('[role=dialog]')).toBeNull()
  })

  it('a failed delete is handed back once the menu has gone', async () => {
    render()
    invoke.mockRejectedValueOnce('no network')
    item('Delete category').click()
    await vi.waitFor(() => expect(cb.onError).toHaveBeenCalledWith('no network'))
  })

  it('Escape closes it and gives focus back to the heading', () => {
    render()
    key(item('Create channel'), 'ArrowDown')
    expect(document.activeElement).toBe(item('Rename category…'))
    key(document.activeElement!, 'Escape')
    expect(cb.onClose).toHaveBeenCalled()
    expect(document.activeElement).toBe(opener)
  })
})
