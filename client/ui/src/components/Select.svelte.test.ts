// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import Select from './Select.svelte'
import { chooseOption, openSelect, selectOptions, selectTrigger, selectValue } from './selectHarness'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const options = [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Bravo' }]
const channels = [{ value: '', label: 'None (no cards)' }, { value: 'g', label: '#general' }, { value: 'm', label: '#memes', disabled: true }, { value: 's', label: '#sport' }]

function render(props: Record<string, unknown>) {
  const state = $state({ value: props.value as string | null })
  const onchange = vi.fn()
  cmp = mount(Select, { target: document.body, props: { label: 'Pick', options, onchange, ...props, get value() { return state.value }, set value(v) { state.value = v } } })
  flushSync()
  return { trigger: selectTrigger('Pick')!, onchange, state }
}
const press = (t: HTMLElement, key: string) => {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })
  t.dispatchEvent(e)
  flushSync()
  return e
}
const list = () => document.querySelector<HTMLElement>('[role=listbox]')
const activeLabel = (t: HTMLElement) => document.getElementById(t.getAttribute('aria-activedescendant')!)?.textContent?.trim()

describe('Select', () => {
  it('opens_downward_by_default', () => {
    render({ value: 'a' })
    expect(document.querySelector('.select')?.getAttribute('data-drop')).toBe('down')
  })
  it('drop_up_is_marked_for_the_upward_list', () => {
    render({ value: 'a', drop: 'up' })
    expect(document.querySelector('.select')?.getAttribute('data-drop')).toBe('up')
  })

  it('is a combobox button that opens a listbox of options, never a native select (#455)', () => {
    const { trigger } = render({ value: 'b' })
    expect(document.querySelector('select')).toBeNull()
    expect(trigger.getAttribute('role')).toBe('combobox')
    expect(trigger.getAttribute('aria-haspopup')).toBe('listbox')
    expect(trigger.getAttribute('aria-expanded')).toBe('false')
    expect(trigger.textContent).toContain('Bravo')
    const l = openSelect(trigger)
    expect(trigger.getAttribute('aria-expanded')).toBe('true')
    expect(l.getAttribute('role')).toBe('listbox')
    expect(trigger.getAttribute('aria-controls')).toBe(l.id)
    const opts = [...l.querySelectorAll('[role=option]')]
    expect(opts.map((o) => o.getAttribute('aria-selected'))).toEqual(['false', 'true'])
    expect(activeLabel(trigger)).toBe('Bravo')
  })

  it('the list is fixed by the button and capped in height, so a pane or card never clips it', () => {
    const { trigger } = render({ value: 'a' })
    openSelect(trigger)
    expect(list()!.style.top).not.toBe('')
    expect(list()!.style.maxHeight).toMatch(/px$/)
  })

  it('a click picks: value and onchange update, the list closes', () => {
    const { trigger, onchange, state } = render({ value: 'a' })
    chooseOption(trigger, 'b')
    expect(state.value).toBe('b')
    expect(onchange).toHaveBeenCalledWith('b')
    expect(list()).toBeNull()
    expect(selectValue(trigger)).toBe('b')
  })

  it('arrows, Home and End move past disabled options; Enter picks', () => {
    const { trigger, state } = render({ value: '', options: channels })
    press(trigger, 'ArrowDown')
    expect(list()).not.toBeNull()
    expect(activeLabel(trigger)).toBe('None (no cards)')
    press(trigger, 'ArrowDown')
    expect(activeLabel(trigger)).toBe('#general')
    press(trigger, 'ArrowDown') // over the disabled #memes
    expect(activeLabel(trigger)).toBe('#sport')
    press(trigger, 'ArrowDown') // stays at the end
    expect(activeLabel(trigger)).toBe('#sport')
    press(trigger, 'Home')
    expect(activeLabel(trigger)).toBe('None (no cards)')
    press(trigger, 'End')
    expect(activeLabel(trigger)).toBe('#sport')
    press(trigger, 'ArrowUp')
    expect(activeLabel(trigger)).toBe('#general')
    press(trigger, 'Enter')
    expect(state.value).toBe('g')
    expect(list()).toBeNull()
  })

  it('Escape closes without picking, and is marked handled so a modal around it stays open', () => {
    const { trigger, state } = render({ value: 'a' })
    press(trigger, 'Enter')
    press(trigger, 'ArrowDown')
    const e = press(trigger, 'Escape')
    expect(e.defaultPrevented).toBe(true)
    expect(list()).toBeNull()
    expect(state.value).toBe('a')
  })

  it('typing jumps to the option starting with it (a leading # skipped), opening the list', () => {
    const { trigger, state } = render({ value: '', options: channels })
    press(trigger, 's')
    expect(list()).not.toBeNull()
    expect(activeLabel(trigger)).toBe('#sport')
    press(trigger, 'Enter')
    expect(state.value).toBe('s')
  })

  it('a disabled option is marked and a click on it does nothing', () => {
    const { trigger, state, onchange } = render({ value: '', options: channels })
    expect(selectOptions(trigger).find((o) => o.value === 'm')!.disabled).toBe(true)
    chooseOption(trigger, 'm')
    expect(state.value).toBe('')
    expect(onchange).not.toHaveBeenCalled()
  })

  it('disabled: the button is disabled and keys do nothing', () => {
    const { trigger } = render({ value: 'a', disabled: true })
    expect(trigger.disabled).toBe(true)
    press(trigger, 'ArrowDown')
    expect(list()).toBeNull()
  })

  it('size sm takes the small field size', () => {
    render({ value: 'a', size: 'sm' })
    expect(document.querySelector('.select')!.classList.contains('sm')).toBe(true)
  })
})
