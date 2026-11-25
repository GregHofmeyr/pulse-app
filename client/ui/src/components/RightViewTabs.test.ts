// @vitest-environment jsdom
// The right panel's switcher (#638): tabs for the views that could fill the column, arrow keys, and never spilling out.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { compile } from 'svelte/compiler'
import { afterEach, describe, expect, it, vi } from 'vitest'
import RightViewTabs from './RightViewTabs.svelte'

const views = [
  { id: 'search', label: 'Search results', icon: 'search' as const },
  { id: 'build', label: 'Pinned build', icon: 'pin' as const },
  { id: 'members', label: 'Members', icon: 'users' as const },
]
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const show = (width: number, onPick = vi.fn(), current = 'build') => {
  cmp = mount(RightViewTabs, { target: document.body, props: { views, current, width, panelId: 'right-view', onPick } })
  flushSync()
  return onPick
}
const tabs = () => [...document.querySelectorAll<HTMLButtonElement>('[role=tab]')]

describe('RightViewTabs', () => {
  it('a tablist of the views, the shown one selected and the only one in the tab order', () => {
    show(320)
    const list = document.querySelector('[role=tablist]')!
    expect(list.getAttribute('aria-label')).toBe('Right panel')
    expect(tabs().map((t) => t.textContent?.trim())).toEqual(['Search results', 'Pinned build', 'Members'])
    expect(tabs().map((t) => t.getAttribute('aria-selected'))).toEqual(['false', 'true', 'false'])
    expect(tabs().map((t) => t.tabIndex)).toEqual([-1, 0, -1])
    expect(tabs()[1].getAttribute('aria-controls')).toBe('right-view')
    expect(tabs()[1].id).toBe('right-tab-build')
  })

  it('a click picks; arrows, Home and End move and pick, wrapping', () => {
    const onPick = show(320)
    tabs()[2].click()
    expect(onPick).toHaveBeenLastCalledWith('members')
    const key = (k: string) => tabs()[1].dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
    key('ArrowRight')
    expect(onPick).toHaveBeenLastCalledWith('members')
    expect(document.activeElement).toBe(tabs()[2])
    key('ArrowLeft')
    expect(onPick).toHaveBeenLastCalledWith('search')
    key('End')
    expect(onPick).toHaveBeenLastCalledWith('members')
    key('Home')
    expect(onPick).toHaveBeenLastCalledWith('search')
    tabs()[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
    expect(onPick).toHaveBeenLastCalledWith('members')
  })

  // #638 review: a label cut short by its ellipsis can still be read
  it('every tab has its name as a tooltip, wide too; a badge is a dot and part of its name', () => {
    cmp = mount(RightViewTabs, { target: document.body, props: { views: [views[0], { id: 'chat', label: 'Chat', icon: 'message', badge: { label: ', unread messages' } }], current: 'search', width: 320, panelId: 'right-view', onPick: () => {} } })
    flushSync()
    expect(tabs().map((t) => t.dataset.tip)).toEqual(['Search results', 'Chat'])
    expect(tabs()[1].querySelector('.tab-dot')).not.toBeNull()
    expect(tabs()[1].getAttribute('aria-label')).toBe('Chat, unread messages')
    expect(tabs()[0].querySelector('.tab-dot')).toBeNull()
  })

  it('narrow (the column at its least): icons only, each named by aria-label and a tooltip', () => {
    show(180)
    expect(document.querySelector('.right-tabs')!.classList.contains('compact')).toBe(true)
    for (const [i, t] of tabs().entries()) {
      expect(t.querySelector('.label')).toBeNull()
      expect(t.getAttribute('aria-label')).toBe(views[i].label)
      expect(t.getAttribute('data-tip')).toBe(views[i].label)
      expect(t.querySelector('svg')).not.toBeNull()
    }
  })

  it('never wraps or spills: one row, each tab may shrink, its label ends in an ellipsis', () => {
    const src = readFileSync(resolve(__dirname, 'RightViewTabs.svelte'), 'utf8')
    const css = compile(src, { filename: 'RightViewTabs.svelte', css: 'external', cssHash: () => 'x' }).css!.code
    const rule = (sel: string) => css.match(new RegExp(`${sel.replace(/\./g, '\\.')}\\.x\\s*\\{([^}]*)\\}`))?.[1] ?? ''
    expect(rule('.right-tabs')).toMatch(/flex-wrap:\s*nowrap/)
    expect(rule('.right-tabs')).toMatch(/overflow:\s*hidden/)
    expect(rule('.tab')).toMatch(/min-width:\s*0/)
    expect(rule('.label')).toMatch(/text-overflow:\s*ellipsis/)
    expect(rule('.label')).toMatch(/white-space:\s*nowrap/)
  })
})
