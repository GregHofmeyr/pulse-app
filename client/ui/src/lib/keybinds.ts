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

type KeyLike = { code: string; ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }
export type Recorded = { kind: 'bind'; bind: Bind } | { kind: 'held'; label: string } | { kind: 'cancel' }

const MODIFIERS = ['ControlLeft', 'ControlRight', 'ShiftLeft', 'ShiftRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight']

/** One keydown while recording. Only keyboard keys are read, so mouse clicks never become binds. */
export function fromKeyEvent(e: KeyLike): Recorded {
  if (e.code === 'Escape') return { kind: 'cancel' }
  const mods = { ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, meta: e.metaKey }
  if (MODIFIERS.includes(e.code) || !e.code) return { kind: 'held', label: [...modLabels(mods), '…'].join(' + ') }
  return { kind: 'bind', bind: { code: e.code, ...mods } }
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
