// Test helpers for the themed Select (#455): it's a button and a listbox, not a native <select>.
import { flushSync } from 'svelte'

/** A Select's button: by its aria-label, or the first one in `root`. */
export function selectTrigger(label?: string, root: ParentNode = document): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>(label ? `.select .trigger[aria-label="${label}"]` : '.select .trigger')
}
export const allSelectTriggers = (root: ParentNode = document) => [...root.querySelectorAll<HTMLButtonElement>('.select .trigger')]

/** The value shown on the button ('' for null). */
export const selectValue = (t: HTMLButtonElement) => t.dataset.value ?? ''

export function openSelect(t: HTMLButtonElement): HTMLElement {
  if (t.getAttribute('aria-expanded') !== 'true') {
    t.click()
    flushSync()
  }
  return document.getElementById(t.getAttribute('aria-controls')!)!
}
export function closeSelect(t: HTMLButtonElement) {
  if (t.getAttribute('aria-expanded') !== 'true') return
  t.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
  flushSync()
}

/** Its options, each with label, value and whether it's disabled (opens the list and closes it again). */
export function selectOptions(t: HTMLButtonElement) {
  const list = openSelect(t)
  const opts = [...list.querySelectorAll<HTMLElement>('[role=option]')].map((o) => ({ label: o.textContent!.trim(), value: o.dataset.value ?? '', disabled: o.getAttribute('aria-disabled') === 'true' }))
  closeSelect(t)
  return opts
}
export const selectLabels = (t: HTMLButtonElement) => selectOptions(t).map((o) => o.label)

/** Picks the option with this value ('' for null), as a click does. */
export function chooseOption(t: HTMLButtonElement, value: string) {
  const list = openSelect(t)
  const o = [...list.querySelectorAll<HTMLElement>('[role=option]')].find((x) => (x.dataset.value ?? '') === value)
  if (!o) throw new Error(`no option ${value}`)
  o.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }))
  flushSync()
}
