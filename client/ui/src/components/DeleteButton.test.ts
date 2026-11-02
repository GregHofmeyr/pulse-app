// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import DeleteButton from './DeleteButton.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

const onConfirm = vi.fn()
function show(props: { what?: string; note?: string } = {}) {
  onConfirm.mockReset()
  cmp = mount(DeleteButton, { target: document.body, props: { what: 'message', onConfirm, ...props } })
  flushSync()
  return document.querySelector<HTMLButtonElement>('button[aria-label="Delete"]')!
}
const ask = () => document.querySelector<HTMLElement>('[role="alertdialog"]')
const click = (el: Element, more: MouseEventInit = {}) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, ...more }))
  flushSync()
}

describe('DeleteButton', () => {
  it('a click asks first: nothing is deleted yet, and Cancel has the focus', async () => {
    click(show())
    for (let i = 0; i < 3; i++) await tick()
    expect(onConfirm).not.toHaveBeenCalled()
    expect(ask()?.getAttribute('aria-label')).toBe('Delete message')
    expect(ask()?.textContent).toContain('Delete this message?')
    expect(document.activeElement?.textContent).toBe('Cancel')
  })

  it('says what else goes with it', () => {
    click(show({ what: 'quote', note: 'Its replies go too.' }))
    expect(ask()?.textContent).toContain('Delete this quote? Its replies go too.')
  })

  it('Delete in the question deletes, once, and closes it', () => {
    click(show())
    click(ask()!.querySelector('button.go')!)
    expect(onConfirm).toHaveBeenCalledTimes(1)
    expect(ask()).toBeNull()
  })

  it('Cancel and Escape close it without deleting', async () => {
    const trash = show()
    click(trash)
    click([...ask()!.querySelectorAll('button')].find((b) => b.textContent === 'Cancel')!)
    expect(ask()).toBeNull()
    vi.useFakeTimers()
    click(trash)
    vi.advanceTimersByTime(1) // clickOutside listens from the next tick
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(ask()).toBeNull()
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('Shift+click deletes straight away, as on Discord', () => {
    click(show(), { shiftKey: true })
    expect(onConfirm).toHaveBeenCalledTimes(1)
    expect(ask()).toBeNull()
  })
})
