import { describe, expect, it } from 'vitest'
import { Recorder, bindLabel, keyLabel } from './keybinds'

const none = { ctrl: false, shift: false, alt: false, meta: false }
const click = (button: number, mods: Partial<{ ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }> = {}) => ({
  button, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods,
})
const ev = (code: string, mods: Partial<{ ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }> = {}) => ({
  code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods,
})

describe('recording a bind', () => {
  it('a key with modifiers becomes the bind', () => {
    expect(new Recorder().down(ev('KeyM', { ctrlKey: true, shiftKey: true }))).toEqual({
      kind: 'bind', bind: { code: 'KeyM', ctrl: true, shift: true, alt: false, meta: false },
    })
  })

  it('modifiers alone keep waiting for the key, showing what is held', () => {
    const r = new Recorder().down(ev('ControlLeft', { ctrlKey: true }))
    expect(r).toEqual({ kind: 'held', label: 'Ctrl + …' })
  })

  it('Escape cancels', () => {
    expect(new Recorder().down(ev('Escape'))).toEqual({ kind: 'cancel' })
  })

  it('a lone modifier is recorded on release', () => {
    const r = new Recorder()
    expect(r.down(ev('ControlRight', { ctrlKey: true })).kind).toBe('held')
    expect(r.down(ev('ControlRight', { ctrlKey: true })).kind).toBe('held') // auto-repeat
    expect(r.up(ev('ControlRight'))).toEqual({ kind: 'bind', bind: { code: 'ControlRight', ...none } })
  })

  it('two modifiers released with no key record nothing', () => {
    const r = new Recorder()
    const seen = [
      r.down(ev('ControlLeft', { ctrlKey: true })),
      r.down(ev('ShiftLeft', { ctrlKey: true, shiftKey: true })),
      r.up(ev('ShiftLeft', { ctrlKey: true })),
      r.up(ev('ControlLeft')),
    ]
    expect(seen.map((x) => x.kind)).toEqual(['held', 'held', 'none', 'none'])
  })

  it('a modifier then a key records the combo, not the modifier', () => {
    const r = new Recorder()
    r.down(ev('ControlLeft', { ctrlKey: true }))
    expect(r.down(ev('KeyC', { ctrlKey: true }))).toEqual({ kind: 'bind', bind: { code: 'KeyC', ...none, ctrl: true } })
    expect(r.up(ev('ControlLeft'))).toEqual({ kind: 'none' })
  })

  // AltGr layouts: the browser sees a fake Left Ctrl before Right Alt; Windows' hook doesn't
  it('AltGr + a key records Alt + key, not Ctrl + Alt + key', () => {
    const r = new Recorder()
    r.down(ev('ControlLeft', { ctrlKey: true }))
    r.down(ev('AltRight', { ctrlKey: true, altKey: true }))
    expect(r.down(ev('KeyQ', { ctrlKey: true, altKey: true }))).toEqual({ kind: 'bind', bind: { code: 'KeyQ', ...none, alt: true } })
  })

  it('AltGr alone records Right Alt, whichever key-up comes first', () => {
    for (const first of ['ControlLeft', 'AltRight']) {
      const r = new Recorder()
      r.down(ev('ControlLeft', { ctrlKey: true }))
      r.down(ev('AltRight', { ctrlKey: true, altKey: true }))
      const ups = [r.up(ev(first)), r.up(ev(first === 'AltRight' ? 'ControlLeft' : 'AltRight'))]
      expect(ups.filter((u) => u.kind === 'bind')).toEqual([{ kind: 'bind', bind: { code: 'AltRight', ...none } }])
    }
  })

  it('mouse buttons: wheel click and the side buttons, never left or right click', () => {
    const r = new Recorder()
    expect(r.mouse(click(1))).toEqual({ kind: 'bind', bind: { code: 'Mouse3', ...none } })
    expect(r.mouse(click(3, { ctrlKey: true }))).toEqual({ kind: 'bind', bind: { code: 'Mouse4', ...none, ctrl: true } })
    expect(r.mouse(click(4))).toEqual({ kind: 'bind', bind: { code: 'Mouse5', ...none } })
    expect(r.mouse(click(0))).toEqual({ kind: 'none' })
    expect(r.mouse(click(2))).toEqual({ kind: 'none' })
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
    expect(keyLabel('Mouse3')).toBe('Mouse 3')
    expect(keyLabel('ControlRight')).toBe('Right Ctrl')
    expect(keyLabel('MetaLeft')).toBe('Left Win')
    expect(keyLabel('AltRight')).toBe('Right Alt')
  })

  it('a bind lists its modifiers in a fixed order', () => {
    expect(bindLabel({ code: 'KeyD', ctrl: true, shift: true, alt: false, meta: false })).toBe('Ctrl + Shift + D')
    expect(bindLabel({ code: 'F13', ctrl: false, shift: false, alt: false, meta: false })).toBe('F13')
    expect(bindLabel(null)).toBe('Not set')
    expect(bindLabel({ code: 'Mouse4', ctrl: true, shift: false, alt: false, meta: false })).toBe('Ctrl + Mouse 4')
  })
})
