// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { clickOutside, modal } from './actions'

describe('clickOutside', () => {
  it('fires for presses outside and Escape, not inside', async () => {
    const inside = document.createElement('div')
    const outside = document.createElement('div')
    document.body.append(inside, outside)
    const cb = vi.fn()
    const a = clickOutside(inside, cb)
    await new Promise((r) => setTimeout(r, 0)) // armed after the opening click
    inside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(cb).not.toHaveBeenCalled()
    outside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(cb).toHaveBeenCalledTimes(1)
    const esc = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
    window.dispatchEvent(esc)
    expect(cb).toHaveBeenCalledTimes(2)
    // consumed: whatever sits under the menu leaves it alone
    expect(esc.defaultPrevented).toBe(true)
    a.destroy()
    outside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(cb).toHaveBeenCalledTimes(2)
  })

  // What WebView2 / Chromium on Windows sends for one press, in order. A right-click's contextmenu comes after the
  // release there (on Linux it comes after mousedown); either way it's what opens a context menu.
  const tick = () => new Promise((r) => setTimeout(r, 0))
  async function press(el: Element, button: 0 | 2 = 0) {
    const init = { bubbles: true, cancelable: true, composed: true, button }
    el.dispatchEvent(new MouseEvent('pointerdown', { ...init, buttons: button === 2 ? 2 : 1 }))
    el.dispatchEvent(new MouseEvent('mousedown', { ...init, buttons: button === 2 ? 2 : 1 }))
    el.dispatchEvent(new MouseEvent('pointerup', init))
    el.dispatchEvent(new MouseEvent('mouseup', init))
    el.dispatchEvent(new MouseEvent(button === 2 ? 'contextmenu' : 'click', init))
    await tick() // let a menu the press opened arm its outside-press listener
  }
  /** A menu as the components build one: opened by `opener`'s click (a toggle: a second click closes it) or by a
   *  right-click on it, closed by clickOutside. */
  const closers: (() => void)[] = []
  afterEach(() => closers.splice(0).forEach((c) => c()))
  function menuOn(opener: HTMLElement, how: 'click' | 'contextmenu') {
    const m = { el: null as HTMLElement | null, action: null as { destroy(): void } | null }
    closers.push(() => close())
    const close = () => {
      m.action?.destroy()
      m.el?.remove()
      m.el = m.action = null
    }
    opener.addEventListener(how, (e) => {
      e.preventDefault()
      if (m.el) return close() // a toggle: its own click closes it
      m.el = document.createElement('div')
      m.el.innerHTML = '<button>item</button>'
      document.body.append(m.el)
      m.action = clickOutside(m.el, close)
    })
    return m
  }
  function scene() {
    document.body.innerHTML = ''
    const bell = document.createElement('button')
    bell.dataset.menuToggle = ''
    const other = document.createElement('button')
    other.dataset.menuToggle = ''
    const row = document.createElement('div')
    const chat = document.createElement('div')
    document.body.append(bell, other, row, chat)
    return { bell, other, row, chat, bellMenu: menuOn(bell, 'click'), otherMenu: menuOn(other, 'click'), rowMenu: menuOn(row, 'contextmenu') }
  }

  it('a menu opened by its toggle: the opening click keeps it, a second click on the toggle closes it once', async () => {
    const s = scene()
    await press(s.bell)
    expect(s.bellMenu.el).not.toBeNull()
    await press(s.bell)
    expect(s.bellMenu.el).toBeNull()
    await press(s.bell)
    expect(s.bellMenu.el).not.toBeNull()
  })

  it('outside presses (left or right) and Escape close it; presses inside do not', async () => {
    const s = scene()
    await press(s.bell)
    await press(s.bellMenu.el!.querySelector('button')!)
    expect(s.bellMenu.el).not.toBeNull()
    await press(s.chat)
    expect(s.bellMenu.el).toBeNull()
    await press(s.bell)
    await press(s.chat, 2)
    expect(s.bellMenu.el).toBeNull()
    await press(s.bell)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    expect(s.bellMenu.el).toBeNull()
  })

  it('a right-click menu (Windows order: contextmenu after the release) stays up, then closes on a press outside', async () => {
    const s = scene()
    await press(s.row, 2)
    expect(s.rowMenu.el).not.toBeNull()
    await press(s.chat)
    expect(s.rowMenu.el).toBeNull()
  })

  // #192: a press on ANY menu toggle was ignored by every open menu, so a person's menu stayed open behind the
  // bell's menu (and after it), closing only from inside it.
  it('a press on another menu\'s toggle closes an open right-click menu: one menu at a time', async () => {
    const s = scene()
    await press(s.row, 2)
    await press(s.bell)
    expect(s.rowMenu.el).toBeNull()
    expect(s.bellMenu.el).not.toBeNull()
    await press(s.bell)
    expect(s.bellMenu.el).toBeNull()
  })

  it('a press on another toggle closes a toggled menu and opens its own', async () => {
    const s = scene()
    await press(s.bell)
    await press(s.other)
    expect(s.bellMenu.el).toBeNull()
    expect(s.otherMenu.el).not.toBeNull()
    await press(s.chat)
    expect(s.otherMenu.el).toBeNull()
  })

  it('a menu opened from the keyboard (Enter clicks the toggle) still treats that toggle as its own', async () => {
    const s = scene()
    s.bell.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    s.bell.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
    await tick()
    expect(s.bellMenu.el).not.toBeNull()
    await press(s.bell)
    expect(s.bellMenu.el).toBeNull()
  })

  it('a menu opened without a click (Shift+F10) after an earlier toggle click doesn\'t inherit that toggle', async () => {
    const s = scene()
    await press(s.bell)
    await press(s.bell) // bell's menu opened and closed again
    s.row.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true }))
    s.row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
    await tick()
    expect(s.rowMenu.el).not.toBeNull()
    await press(s.bell)
    expect(s.rowMenu.el).toBeNull()
  })
})

describe('modal', () => {
  const afterFrame = () => new Promise<void>((r) => requestAnimationFrame(() => setTimeout(() => setTimeout(r))))
  const setup = () => {
    const opener = document.createElement('button')
    const dialog = document.createElement('div')
    dialog.tabIndex = -1
    dialog.innerHTML = '<button id="a">a</button><input id="b" /><button id="c" disabled>c</button><button id="d">d</button>'
    document.body.append(opener, dialog)
    opener.focus()
    return { opener, dialog }
  }
  const key = (el: Element, k: string, shiftKey = false) => {
    const e = new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true })
    el.dispatchEvent(e)
    return e
  }

  it('moves focus in, traps Tab both ways, closes on Escape, restores focus on destroy', () => {
    const { opener, dialog } = setup()
    const onClose = vi.fn()
    const a = modal(dialog, onClose)
    const first = dialog.querySelector<HTMLElement>('#a')!
    const last = dialog.querySelector<HTMLElement>('#d')!
    expect(document.activeElement).toBe(first)
    last.focus()
    expect(key(last, 'Tab').defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(first)
    expect(key(first, 'Tab', true).defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(last)
    dialog.querySelector<HTMLElement>('#b')!.focus()
    expect(key(document.activeElement!, 'Tab').defaultPrevented).toBe(false) // middle: browser moves it
    key(dialog, 'Escape')
    expect(onClose).toHaveBeenCalledTimes(1)
    a.destroy()
    expect(document.activeElement).toBe(opener)
  })

  it('ignores an Escape that something inside already handled', () => {
    const { dialog } = setup()
    const onClose = vi.fn()
    modal(dialog, onClose)
    dialog.querySelector('#b')!.addEventListener('keydown', (e) => e.preventDefault())
    key(dialog.querySelector('#b')!, 'Escape')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('pulls Tab back in when focus sits on the dialog itself', () => {
    const { dialog } = setup()
    modal(dialog, () => {})
    dialog.focus()
    key(dialog, 'Tab')
    expect(document.activeElement).toBe(dialog.querySelector('#a'))
  })

  it('makes everything behind it inert while open, and puts it back on destroy', async () => {
    document.body.innerHTML = ''
    document.body.innerHTML =
      '<div id="app"><nav><button id="opener">o</button></nav><main><p id="side">s</p>' +
      '<div id="host"><div id="bd" data-backdrop></div><div id="dlg" tabindex="-1"><button id="in">in</button><div id="pop"><button id="pick">p</button></div></div></div>' +
      '</main><aside id="already" inert></aside></div><div id="toast"></div><div id="tip" role="tooltip"></div>'
    const $ = (id: string) => document.getElementById(id)!
    const inert = (el: Element) => el.hasAttribute('inert')
    $('opener').focus()
    const a = modal($('dlg'), () => {})
    // the focus moves in at once; inert lands after the modal's first frame (#238)
    expect(document.activeElement).toBe($('in'))
    expect(inert($('side'))).toBe(false)
    await afterFrame()
    for (const id of ['toast', 'side']) expect(inert($(id)), id).toBe(true)
    expect(inert(document.querySelector('nav')!)).toBe(true)
    // The dialog, its ancestors and anything nested inside it stay usable.
    for (const id of ['app', 'host', 'dlg', 'pop', 'pick', 'tip', 'bd']) expect(inert($(id)), id).toBe(false)
    expect(document.activeElement).toBe($('in'))
    a.destroy()
    for (const id of ['toast', 'side']) expect(inert($(id)), id).toBe(false)
    expect(inert(document.querySelector('nav')!)).toBe(false)
    expect(inert($('already'))).toBe(true) // was inert before: left alone
    expect(document.activeElement).toBe($('opener'))
    document.body.innerHTML = ''
  })

  it('closed before its first frame, it never makes anything inert', async () => {
    document.body.innerHTML = '<p id="side">s</p><div id="dlg" tabindex="-1"><button>in</button></div>'
    modal(document.getElementById('dlg')!, () => {}).destroy()
    await afterFrame()
    expect(document.getElementById('side')!.hasAttribute('inert')).toBe(false)
    document.body.innerHTML = ''
  })

  it('a modal opened over another takes Escape and Tab first', () => {
    const { dialog } = setup()
    const outer = vi.fn()
    const a = modal(dialog, outer)
    const inner = document.createElement('div')
    inner.tabIndex = -1
    inner.innerHTML = '<button id="x">x</button>'
    dialog.append(inner)
    const innerClose = vi.fn()
    const b = modal(inner, innerClose)
    key(inner, 'Escape')
    expect(innerClose).toHaveBeenCalledTimes(1)
    expect(outer).not.toHaveBeenCalled()
    // Tab wraps inside the top one
    key(inner.querySelector('#x')!, 'Tab')
    expect(document.activeElement).toBe(inner.querySelector('#x'))
    b.destroy()
    key(dialog, 'Escape')
    expect(outer).toHaveBeenCalledTimes(1)
    a.destroy()
    document.body.innerHTML = ''
  })

  it('closing a modal twice leaves the others alone', () => {
    const { dialog } = setup()
    const a = modal(dialog, () => {})
    const other = document.createElement('div')
    other.tabIndex = -1
    other.innerHTML = '<button>y</button>'
    document.body.append(other)
    const otherClose = vi.fn()
    const b = modal(other, otherClose)
    a.destroy()
    a.destroy()
    key(other, 'Escape')
    expect(otherClose).toHaveBeenCalledTimes(1)
    b.destroy()
    document.body.innerHTML = ''
  })
})

describe('narrowerThan (#97)', () => {
  it('reports whether the element is narrower than the limit on every resize (the first before paint), and stops when removed', async () => {
    const { narrowerThan } = await import('./actions')
    let fire: (w: number) => void = () => {}
    let disconnected = false
    vi.stubGlobal('ResizeObserver', class {
      constructor(cb: (e: { contentRect: { width: number } }[]) => void) { fire = (w) => cb([{ contentRect: { width: w } }]) }
      observe() {}
      disconnect() { disconnected = true }
    })
    const el = document.createElement('div')
    let read = false
    Object.defineProperty(el, 'clientWidth', { get: () => ((read = true), 500) })
    const seen: boolean[] = []
    const a = narrowerThan(el, { px: 600, set: (v) => seen.push(v) })
    // no synchronous layout read (#173): the observer's first report, before paint, says it
    expect(read).toBe(false)
    expect(seen).toEqual([])
    fire(500)
    expect(seen).toEqual([true])
    fire(800)
    fire(590)
    expect(seen).toEqual([true, false, true])
    a.destroy()
    expect(disconnected).toBe(true)
    vi.unstubAllGlobals()
  })

  it('without ResizeObserver (old engines, tests) it never claims narrow', async () => {
    const { narrowerThan } = await import('./actions')
    vi.stubGlobal('ResizeObserver', undefined)
    const seen: boolean[] = []
    narrowerThan(document.createElement('div'), { px: 600, set: (v) => seen.push(v) }).destroy()
    expect(seen).toEqual([false])
    vi.unstubAllGlobals()
  })
})
