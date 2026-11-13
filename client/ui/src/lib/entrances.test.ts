// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { markFinishedEntrances } from './entrances'

// #156 follow-up: the Settings dialog and its full-screen overlay flashed on Linux while a native colour picker was
// open. The picker is its own window, so the app's window blurs and refocuses; `.unfocused` turns the entrances off
// (animation: none) and turning them back on restarts them: the overlay faded in again from transparent, the dialog
// with it. An entrance that has finished is marked, and the off switch leaves marked ones alone (app.css).
describe('finished entrances', () => {
  let off: (() => void) | undefined
  afterEach(() => off?.())
  const end = (el: Element, animationName: string) => el.dispatchEvent(Object.assign(new Event('animationend', { bubbles: true }), { animationName }))

  it('an element whose pop-in or fade-in ended is marked entered; other animations are not', () => {
    off = markFinishedEntrances(document)
    document.body.innerHTML = '<div class="pop-in" id="a"></div><div class="fade-in" id="b"></div><div class="spin" id="c"></div>'
    const [a, b, c] = ['a', 'b', 'c'].map((id) => document.getElementById(id)!)
    end(a, 'pop-in')
    end(b, 'fade-in')
    end(c, 'spin')
    expect([a, b, c].map((el) => el.classList.contains('entered'))).toEqual([true, true, false])
  })

  it('stops listening when asked', () => {
    markFinishedEntrances(document)()
    document.body.innerHTML = '<div class="pop-in" id="a"></div>'
    end(document.getElementById('a')!, 'pop-in')
    expect(document.getElementById('a')!.classList.contains('entered')).toBe(false)
  })
})
