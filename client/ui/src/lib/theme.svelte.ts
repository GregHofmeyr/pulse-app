// The theme, Light's tint, the accent and Name readability (#42): per device, like "Show name styles". applyTheme puts
// them on <html> (data-theme / data-tint pick the app.css block; a custom accent's derived tokens go inline) and tells
// the app window its background colour, so a restart on Light doesn't flash dark.
import { invoke } from '@tauri-apps/api/core'
import { parseHex } from './color'
import { type CardLook } from './cardTheme'
import { DEFAULT_NAME_LEVEL, DEFAULT_THEME, DEFAULT_TINT, deriveAccent, NAME_LEVELS, surfacesFor, themeByKey, THEMES, TINTS, type NameLevel, type Surfaces, type ThemeKey, type TintKey } from './themes'

const KEYS = { key: 'pulse.theme', tint: 'pulse.lightTint', accent: 'pulse.accent', names: 'pulse.nameReadability' } as const

function read(k: string): string | null {
  try {
    return localStorage.getItem(k)
  } catch {
    return null
  }
}
function write(k: string, v: string | null): void {
  try {
    if (v === null) localStorage.removeItem(k)
    else localStorage.setItem(k, v)
  } catch {
    /* ignore */
  }
}
function pick<T extends string>(v: string | null, known: readonly { key: T }[], fallback: T): T {
  return known.find((x) => x.key === v)?.key ?? fallback
}

export const theme = $state({
  key: pick(read(KEYS.key), THEMES, DEFAULT_THEME) as ThemeKey,
  tint: pick(read(KEYS.tint), TINTS, DEFAULT_TINT) as TintKey,
  accent: ((v) => (v && parseHex(v) ? v.toLowerCase() : null))(read(KEYS.accent)) as string | null,
  names: pick(read(KEYS.names), NAME_LEVELS, DEFAULT_NAME_LEVEL) as NameLevel,
})

export function setTheme(k: ThemeKey): void {
  theme.key = k
  write(KEYS.key, k)
  applyTheme()
}
export function setTint(t: TintKey): void {
  theme.tint = t
  write(KEYS.tint, t)
  applyTheme()
}
/** A `#rrggbb` colour, or null for the theme's own accent. */
export function setAccent(hex: string | null): void {
  theme.accent = hex && parseHex(hex) ? hex.toLowerCase() : null
  write(KEYS.accent, theme.accent)
  applyTheme()
}
export function setNameReadability(l: NameLevel): void {
  theme.names = l
  write(KEYS.names, l)
}

export function currentSurfaces(): Surfaces {
  return surfacesFor(theme.key, theme.tint)
}

/** The contrast names are moved to (Name readability). */
export function nameRatio(): number {
  return (NAME_LEVELS.find((l) => l.key === theme.names) ?? NAME_LEVELS[1]).ratio
}

/** What profile cards take from the current theme (lib/cardTheme.ts): only whether it is light, where a soft shadow
 *  lifts them off the page; their colours and text come from the card itself (#156). Reads the theme: call it inside a
 *  $derived. */
export function cardLook(): CardLook {
  return { light: themeByKey(theme.key).light }
}

export type NameSurface = 'chat' | 'side' | 'menu'

/** The surface a name is measured against: the chat (--bg-2), the sidebar and member lists (--bg-1), or a menu
 *  (--surface-menu: the @-mention list, search suggestions). On a profile card, Name is given the card's own colour. */
export function nameBg(surface: NameSurface): string {
  const s = currentSurfaces()
  return surface === 'side' ? s.bg1 : surface === 'menu' ? s.menu : s.bg2
}

/** What resolveNameStyle needs to make a name readable where it sits (reads the theme: call it inside a $derived). */
export function nameOpts(surface: NameSurface): { bg: string; ratio: number } {
  return { bg: nameBg(surface), ratio: nameRatio() }
}

const ACCENT_TOKENS = Object.keys(deriveAccent('#000000', THEMES[0].surfaces, false))

export function applyTheme(root: HTMLElement = document.documentElement): void {
  const t = themeByKey(theme.key)
  root.dataset.theme = t.key
  if (t.key === 'light') root.dataset.tint = theme.tint
  else delete root.dataset.tint
  if (theme.accent) {
    for (const [k, v] of Object.entries(deriveAccent(theme.accent, currentSurfaces(), t.light))) root.style.setProperty(k, v)
  } else {
    for (const k of ACCENT_TOKENS) root.style.removeProperty(k)
  }
  const bg = currentSurfaces().bg0
  if (bg !== sentBg && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    sentBg = bg
    invoke('set_window_background', { color: bg }).catch(() => {
      sentBg = null
    })
  }
}

// The window colour last sent: an accent change (or dragging the colour picker) doesn't resend it.
let sentBg: string | null = null
