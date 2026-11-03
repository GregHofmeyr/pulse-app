// @vitest-environment jsdom
// The eye toggle inside every password field (#131).
import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { compile } from 'svelte/compiler'
import { afterEach, describe, expect, it } from 'vitest'
import PasswordInput from './PasswordInput.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

const show = (props: Record<string, unknown> = {}) => {
  cmp = mount(PasswordInput, { target: document.body, props: { value: '', ...props } })
  flushSync()
  return {
    input: document.querySelector<HTMLInputElement>('input')!,
    toggle: document.querySelector<HTMLButtonElement>('button')!,
  }
}

describe('PasswordInput', () => {
  it('starts hidden, and the eye shows then hides it again', () => {
    const { input, toggle } = show()
    expect(input.type).toBe('password')
    expect(toggle.type).toBe('button')
    expect(toggle.getAttribute('aria-label')).toBe('Show password')
    expect(toggle.getAttribute('aria-pressed')).toBe('false')
    toggle.click(); flushSync()
    expect(input.type).toBe('text')
    expect(toggle.getAttribute('aria-label')).toBe('Hide password')
    expect(toggle.getAttribute('aria-pressed')).toBe('true')
    toggle.click(); flushSync()
    expect(input.type).toBe('password')
    expect(toggle.getAttribute('aria-label')).toBe('Show password')
  })

  it('passes the field attributes through to the input', () => {
    const { input } = show({ class: 'input input-sm', required: true, autocomplete: 'new-password', minlength: 8 })
    expect(input.className).toContain('input-sm')
    expect(input.required).toBe(true)
    expect(input.getAttribute('autocomplete')).toBe('new-password')
    expect(input.minLength).toBe(8)
  })

  it('keeps focus and the caret in the field when the eye is clicked', () => {
    const { input, toggle } = show()
    input.value = 'secret12'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.focus()
    input.setSelectionRange(3, 3)
    // a pointer press doesn't take focus from the field
    const down = new MouseEvent('mousedown', { bubbles: true, cancelable: true })
    toggle.dispatchEvent(down)
    expect(down.defaultPrevented).toBe(true)
    toggle.click(); flushSync()
    expect(document.activeElement).toBe(input)
    expect(input.type).toBe('text')
    expect(input.selectionStart).toBe(3)
    expect(input.selectionEnd).toBe(3)
  })

  it('binds the value both ways', () => {
    let value = 'abc'
    cmp = mount(PasswordInput, {
      target: document.body,
      props: { get value() { return value }, set value(v: string) { value = v } },
    })
    flushSync()
    const input = document.querySelector('input')!
    expect(input.value).toBe('abc')
    input.value = 'abcd'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(value).toBe('abcd')
  })
})

describe('PasswordInput on submit', () => {
  it('hides the password again when its form is sent', () => {
    const form = document.createElement('form')
    document.body.append(form)
    cmp = mount(PasswordInput, { target: form, props: { value: '' } })
    flushSync()
    form.querySelector('button')!.click(); flushSync()
    expect(form.querySelector('input')!.type).toBe('text')
    form.dispatchEvent(new Event('submit', { cancelable: true }))
    flushSync()
    expect(form.querySelector('input')!.type).toBe('password')
    expect(form.querySelector('button')!.getAttribute('aria-label')).toBe('Show password')
  })
})

describe('PasswordInput CSS', () => {
  const scope = 'svelte-test'
  const css = compile(readFileSync(resolve(__dirname, 'PasswordInput.svelte'), 'utf8'),
    { filename: 'PasswordInput.svelte', css: 'external', cssHash: () => scope }).css!.code

  it('draws a focus ring on the eye from the focus token', () => {
    expect(css).toMatch(/\.eye\.svelte-test:focus-visible\s*\{[^}]*var\(--focus-ring\)/)
  })

  it('leaves room in the field for the eye, so text never runs under it', () => {
    expect(css).toMatch(/input\.svelte-test\s*\{[^}]*padding-right/)
  })
})

describe('every password field', () => {
  it('is a PasswordInput: no bare type="password" input left anywhere in the app', () => {
    const root = resolve(__dirname, '..')
    const files = readdirSync(root, { recursive: true, encoding: 'utf8' })
      .filter((f) => f.endsWith('.svelte') && !f.endsWith('PasswordInput.svelte'))
    expect(files.length).toBeGreaterThan(50)
    const bare = files.filter((f) => /type=["']password["']/.test(readFileSync(resolve(root, f), 'utf8')))
    expect(bare).toEqual([])
  })

  it('names its field itself: the eye sits inside the label, whose text would otherwise take in "Show password"', () => {
    const root = resolve(__dirname, '..')
    const uses = readdirSync(root, { recursive: true, encoding: 'utf8' })
      .filter((f) => f.endsWith('.svelte'))
      .flatMap((f) => readFileSync(resolve(root, f), 'utf8').match(/<PasswordInput\b[^>]*>/g) ?? [])
    expect(uses.length).toBeGreaterThanOrEqual(5)
    expect(uses.filter((u) => !/aria-label=/.test(u))).toEqual([])
  })
})
