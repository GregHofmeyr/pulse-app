// @vitest-environment jsdom
// The wheel drawn with weights and a palette (#259), and the editor's weights only on saved wheels.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import Wheel from './Wheel.svelte'
import WheelEditor from './WheelEditor.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const paths = () => [...document.querySelectorAll<SVGPathElement>('.slice path')]

describe('a weighted wheel', () => {
  it("gives each slice its weight's share: [1, 3] is a quarter and three quarters", () => {
    cmp = mount(Wheel, { target: document.body, props: { options: ['a', 'b'], weights: [1, 3] } })
    flushSync()
    const [a, b] = paths().map((p) => p.getAttribute('d')!)
    // slice a runs from 12 o'clock to 3 o'clock (a small arc); slice b the rest (a large one)
    expect(a).toMatch(/^M0 0L0.000 -100.000A100 100 0 0 1 100.000 -?0.000Z$/)
    expect(b).toContain('A100 100 0 1 1')
  })

  it("draws a palette's slices in theme tokens with edged labels", () => {
    cmp = mount(Wheel, { target: document.body, props: { options: ['a', 'b', 'c'], palette: 'signal' } })
    flushSync()
    expect(paths().map((p) => p.getAttribute('fill'))).toEqual(['var(--ok)', 'var(--warn)', 'var(--danger)'])
    expect(paths()[0].style.fill).toBe('var(--ok)')
    const label = document.querySelector<SVGTextElement>('.slice text')!
    expect(label.getAttribute('fill')).toBe('var(--on-video)')
    expect(label.classList.contains('edged')).toBe(true)
  })
})

describe('the editor', () => {
  it('has no weights or palettes for a quick wheel', () => {
    cmp = mount(WheelEditor, { target: document.body, props: { options: ['a', 'b'], onChange: () => {} } })
    flushSync()
    expect(document.querySelector('input[type=number]')).toBeNull()
    expect(document.querySelector('[aria-label=Colours]')).toBeNull()
  })

  it("keeps each option's weight beside it through Shuffle, Remove and Add", () => {
    const onChange = vi.fn()
    cmp = mount(WheelEditor, { target: document.body, props: { options: ['a', 'b', 'c'], weights: [5, 1, 1], weighted: true, onChange } })
    flushSync()
    expect(document.querySelector('[aria-label="Weight of a"]')!.closest('label')!.getAttribute('data-tip')).toBe('71% of the wheel')
    document.querySelector<HTMLButtonElement>('[aria-label="Remove b"]')!.click()
    expect(onChange).toHaveBeenLastCalledWith(['a', 'c'], [5, 1])
    // removing the heavy one leaves them all alike: no weights
    document.querySelector<HTMLButtonElement>('[aria-label="Remove a"]')!.click()
    expect(onChange).toHaveBeenLastCalledWith(['b', 'c'], undefined)
    const add = document.querySelector<HTMLInputElement>('input[aria-label="Add an option"]')!
    add.value = 'd'
    add.dispatchEvent(new Event('input'))
    flushSync()
    add.form!.requestSubmit()
    expect(onChange).toHaveBeenLastCalledWith(['a', 'b', 'c', 'd'], [5, 1, 1, 1])
    vi.spyOn(Math, 'random').mockReturnValue(0)
    ;[...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Shuffle')!.click()
    const [opts, ws] = onChange.mock.lastCall!
    expect(opts.map((o: string, i: number) => [o, ws[i]])).toEqual(expect.arrayContaining([['a', 5], ['b', 1], ['c', 1]]))
    vi.restoreAllMocks()
  })
})
