import { invoke } from '@tauri-apps/api/core'
// Keybinds: recorded here, validated, saved and registered by the Rust core (keybinds.rs).

/** A key combination, by physical key (W3C `code`), so it survives a layout switch. */
export type Bind = { code: string; ctrl: boolean; shift: boolean; alt: boolean; meta: boolean }
export type Keybinds = { toggle_mute: Bind | null; toggle_deafen: Bind | null }
export type Action = keyof Keybinds

export const actions: { key: Action; label: string }[] = [
  { key: 'toggle_mute', label: 'Toggle mute' },
  { key: 'toggle_deafen', label: 'Toggle deafen' },
]

type Mods = { ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }
type KeyLike = Mods & { code: string }
type MouseLike = Mods & { button: number }
export type Recorded =
  | { kind: 'bind'; bind: Bind }
  | { kind: 'held'; label: string }
  | { kind: 'cancel' }
  | { kind: 'none' }

const MODIFIERS = ['ControlLeft', 'ControlRight', 'ShiftLeft', 'ShiftRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight']
/** `MouseEvent.button` → bindable button: wheel click and the two side buttons. */
const MOUSE: Record<number, string> = { 1: 'Mouse3', 3: 'Mouse4', 4: 'Mouse5' }

const mods = (e: Mods) => ({ ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, meta: e.metaKey })

/** One recording: a key (with modifiers), a mouse button, or a modifier pressed and released on
 *  its own (Right Ctrl). Left and right click are never read, so they can't become binds. */
export class Recorder {
  private pressed = new Set<string>()
  /** The one modifier pressed so far with nothing else, which becomes the bind on release. */
  private lone: string | null = null

  down(e: KeyLike): Recorded {
    if (e.code === 'Escape') return { kind: 'cancel' }
    const held = { kind: 'held' as const, label: [...modLabels(mods(e)), '…'].join(' + ') }
    if (!e.code) return held
    if (this.pressed.has(e.code)) return held // auto-repeat
    if (MODIFIERS.includes(e.code)) {
      this.lone = this.pressed.size === 0 ? e.code : null
      this.pressed.add(e.code)
      return held
    }
    this.lone = null
    return { kind: 'bind', bind: { code: e.code, ...mods(e) } }
  }

  up(e: KeyLike): Recorded {
    const lone = this.lone === e.code
    this.pressed.delete(e.code)
    this.lone = null
    if (!lone) return { kind: 'none' }
    // the released modifier itself isn't also its own modifier (Rust normalizes the same way)
    const flag = { Control: 'ctrl', Shift: 'shift', Alt: 'alt', Meta: 'meta' }[e.code.replace(/Left|Right/, '')]
    return { kind: 'bind', bind: { code: e.code, ...mods(e), ...(flag ? { [flag]: false } : {}) } }
  }

  mouse(e: MouseLike): Recorded {
    const code = MOUSE[e.button]
    if (!code) return { kind: 'none' }
    this.lone = null
    return { kind: 'bind', bind: { code, ...mods(e) } }
  }
}

function modLabels(m: { ctrl: boolean; shift: boolean; alt: boolean; meta: boolean }): string[] {
  return [m.ctrl && 'Ctrl', m.shift && 'Shift', m.alt && 'Alt', m.meta && 'Win'].filter((x): x is string => !!x)
}

const NAMED: Record<string, string> = {
  Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\',
  Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/',
  ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
  PageUp: 'Page Up', PageDown: 'Page Down', ScrollLock: 'Scroll Lock', PrintScreen: 'Print Screen',
  NumpadAdd: 'Num +', NumpadSubtract: 'Num -', NumpadMultiply: 'Num *', NumpadDivide: 'Num /',
  NumpadDecimal: 'Num .', NumpadEnter: 'Num Enter',
}

export function keyLabel(code: string): string {
  if (NAMED[code]) return NAMED[code]
  if (code.startsWith('Mouse')) return `Mouse ${code.slice(5)}`
  const side = code.match(/^(Control|Shift|Alt|Meta)(Left|Right)$/)
  if (side) return `${side[2]} ${{ Control: 'Ctrl', Shift: 'Shift', Alt: 'Alt', Meta: 'Win' }[side[1]]}`
  if (code.startsWith('Key')) return code.slice(3)
  if (code.startsWith('Digit')) return code.slice(5)
  if (code.startsWith('Numpad')) return `Num ${code.slice(6)}`
  return code
}

export function bindLabel(b: Bind | null): string {
  if (!b) return 'Not set'
  return [...modLabels(b), keyLabel(b.code)].join(' + ')
}

/** The binds in use, any that couldn't be registered (another app owns them), and whether the
 *  user has ever chosen (nothing is bound by default: first run nudges them to pick). */
export type Applied = { binds: Keybinds; problems: string[]; chosen: boolean }

export const keybindsApi = {
  get: () => invoke<Applied>('get_keybinds'),
  /** Rejects with a message if the set isn't usable (nothing changes). */
  set: (binds: Keybinds) => invoke<Applied>('set_keybinds', { binds }),
  pause: (paused: boolean) => invoke<void>('pause_keybinds', { paused }),
}
