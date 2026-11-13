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

  it("the question is moved out to the page, so the action bar's layer can't tuck it under the composer", () => {
    const host = document.createElement('div')
    host.className = 'bar'
    document.body.append(host)
    onConfirm.mockReset()
    cmp = mount(DeleteButton, { target: host, props: { what: 'quote', onConfirm } })
    flushSync()
    click(host.querySelector('button[aria-label="Delete"]')!)
    expect(ask()?.parentElement).toBe(document.body)
    expect(host.contains(ask())).toBe(false)
    click(ask()!.querySelector('.go')!)
    expect(onConfirm).toHaveBeenCalledOnce()
    expect(ask()).toBeNull()
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

  it('floats over the chat (fixed, by the bin), so opening it never changes the list\'s size', () => {
    const trash = show()
    trash.getBoundingClientRect = () => ({ left: 600, right: 630, top: 100, bottom: 128, width: 30, height: 28, x: 600, y: 100, toJSON() {} }) as DOMRect
    click(trash)
    const box = ask()!
    expect(box.style.position).toBe('fixed')
    expect(box.style.top).toBe('134px') // below the bin
    expect(box.style.left).toBe('390px') // right edges line up (240 wide)
  })

  it('near the bottom of the window it opens above the bin', () => {
    const trash = show()
    const h = window.innerHeight
    trash.getBoundingClientRect = () => ({ left: 600, right: 630, top: h - 40, bottom: h - 12, width: 30, height: 28, x: 600, y: h - 40, toJSON() {} }) as DOMRect
    click(trash)
    expect(ask()!.style.top).toBe('')
    expect(ask()!.style.bottom).toBe('46px') // 6 px above the bin's top
  })

  it('scrolling closes it (it would drift away from its message)', () => {
    vi.useFakeTimers()
    click(show())
    vi.advanceTimersByTime(1)
    document.dispatchEvent(new Event('scroll'))
    flushSync()
    expect(ask()).toBeNull()
  })

  it('Shift+click deletes straight away, as on Discord', () => {
    click(show(), { shiftKey: true })
    expect(onConfirm).toHaveBeenCalledTimes(1)
    expect(ask()).toBeNull()
  })
})
