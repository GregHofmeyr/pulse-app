import { describe, expect, it } from 'vitest'
import { bindLabel, fromKeyEvent, keyLabel } from './keybinds'

const ev = (code: string, mods: Partial<{ ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }> = {}) => ({
  code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods,
})

describe('recording a bind', () => {
  it('a key with modifiers becomes the bind', () => {
    expect(fromKeyEvent(ev('KeyM', { ctrlKey: true, shiftKey: true }))).toEqual({
      kind: 'bind', bind: { code: 'KeyM', ctrl: true, shift: true, alt: false, meta: false },
    })
  })

  it('modifiers alone keep waiting for the key, showing what is held', () => {
    const r = fromKeyEvent(ev('ControlLeft', { ctrlKey: true }))
    expect(r).toEqual({ kind: 'held', label: 'Ctrl + …' })
  })

  it('Escape cancels', () => {
    expect(fromKeyEvent(ev('Escape'))).toEqual({ kind: 'cancel' })
  })
})

describe('labels', () => {
  it('reads like the keyboard', () => {
    expect(keyLabel('KeyM')).toBe('M')
    expect(keyLabel('Digit7')).toBe('7')
    expect(keyLabel('Numpad1')).toBe('Num 1')
    expect(keyLabel('F13')).toBe('F13')
    expect(keyLabel('Backquote')).toBe('`')
    expect(keyLabel('ArrowUp')).toBe('↑')
    expect(keyLabel('Pause')).toBe('Pause')
  })

  it('a bind lists its modifiers in a fixed order', () => {
    expect(bindLabel({ code: 'KeyD', ctrl: true, shift: true, alt: false, meta: false })).toBe('Ctrl + Shift + D')
    expect(bindLabel({ code: 'F13', ctrl: false, shift: false, alt: false, meta: false })).toBe('F13')
    expect(bindLabel(null)).toBe('Not set')
  })
})
