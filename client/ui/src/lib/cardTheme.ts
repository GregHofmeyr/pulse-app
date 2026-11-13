// Card themes (#156): two raw colours, drawn as picked, top to bottom, over the whole card, in every app theme. The
// text is chosen per card from those colours, not from the app theme: near-black on a light card, near-white on a dark
// one, whichever reads with the smaller change, and the colours are moved toward white or black only as far as body
// text (its secondary grey included) needs to reach WCAG AA (4.5:1). The card's inner surfaces are translucent overlays, so its colour shows through.
import { contrast, luminance, parseHex, readable, toHex, type Rgb } from './color'

export type ThemeKey = 'sunset' | 'night' | 'forest' | 'berry' | 'plain'
export type ThemePreset = { key: ThemeKey; label: string; top: string; bottom: string }

export const THEME_PRESETS: ThemePreset[] = [
  { key: 'sunset', label: 'Sunset', top: '#8a3cff', bottom: '#ff8a2a' },
  { key: 'night', label: 'Night sky', top: '#22325e', bottom: '#0d1430' },
  { key: 'forest', label: 'Forest', top: '#1f7a5a', bottom: '#0f2b3a' },
  { key: 'berry', label: 'Berry', top: '#c2457a', bottom: '#3a1f5e' },
  { key: 'plain', label: 'Plain', top: '#3a3d46', bottom: '#16171b' },
]

export const DEFAULT_THEME = THEME_PRESETS[4]

/** A card still loading (#173): flat neutral grey, not anyone's colours (nor the default theme's), so nothing recolours
 *  when the person's own theme arrives except into it. */
export const SKELETON_THEME = { top: '#4a4d55', bottom: '#4a4d55' }

/** Body text must reach this on every surface of the card (WCAG AA). */
export const BODY_RATIO = 4.5

/** One of a card's two looks: its text, the side its colours move toward when the text needs it, and its overlays. */
type Ink = {
  light: boolean
  text: string
  text2: string
  toward: Rgb
  /** The panels (bio skeleton, stats, actions, Listening to): white on a light card, black on a dark one. */
  panel: [Rgb, number]
  /** The message box: a little darker than the card on either. */
  field: [Rgb, number]
  ok: string
  danger: string
  /** The overlay rgb triplet ("23 24 28") lines, hovers and the skeleton are drawn in. */
  line: string
}

const WHITE: Rgb = [255, 255, 255]
const BLACK: Rgb = [0, 0, 0]

// A light card: near-black text, panels a whitish wash, the message box a faint dark one.
export const LIGHT_CARD: Ink = {
  light: true, text: '#17181c', text2: '#3f424b', toward: WHITE,
  panel: [WHITE, 0.4], field: [BLACK, 0.07], ok: '#1a7f4b', danger: '#c0262f', line: '23 24 28',
}
// A dark card: near-white text (as before), panels and the message box darker washes.
export const DARK_CARD: Ink = {
  light: false, text: '#f2f3f6', text2: '#d6d8df', toward: BLACK,
  panel: [BLACK, 0.22], field: [BLACK, 0.3], ok: '#5fd68f', danger: '#ff9d96', line: '242 243 246',
}

/** How far toward black (light card) or white (dark card) the border is: a stronger shade of the card's colour. */
export const EDGE_SHADE = 0.28

/** The app theme a card sits in: on a light one a soft shadow lifts it off the page. Nothing else follows it. */
export type CardLook = { light: boolean }
export const DARK_LOOK: CardLook = { light: false }

// Each raw colour at 45% alpha is its glow (reconciliation 8).
const GLOW_ALPHA = '73'

export function presetFor(top: string, bottom: string): ThemeKey | null {
  const t = top.toLowerCase()
  const b = bottom.toLowerCase()
  return THEME_PRESETS.find((p) => p.top === t && p.bottom === b)?.key ?? null
}

const rgbOf = (c: string): Rgb => parseHex(c) ?? parseHex(DEFAULT_THEME.bottom)!

/** `color` mixed `weight` (0..1) into `base`. */
export function mixHex(color: string, base: string, weight: number): string {
  const c = rgbOf(color)
  const b = rgbOf(base)
  return toHex(c.map((v, i) => Math.round(v * weight + b[i] * (1 - weight))) as Rgb)
}

/** `over` at `alpha` laid on `under`: the colour a translucent overlay shows. */
export function composite(under: string, [over, alpha]: [Rgb, number]): string {
  const u = rgbOf(under)
  return toHex(u.map((v, i) => Math.round(over[i] * alpha + v * (1 - alpha))) as Rgb)
}

const css = ([c, a]: [Rgb, number]) => `rgb(${c.join(' ')} / ${String(a).replace(/^0\./, '.')})`

/** Every surface text sits on, for these stops: the card's two ends and each overlay over them. */
function surfacesOf(stops: string[], ink: Ink): string[] {
  return stops.flatMap((s) => [s, composite(s, ink.panel), composite(s, ink.field)])
}

/** The least shift (0..100% toward white or black) of one stop at which `ink`'s text and secondary text read on it and
 *  on the overlays over it, the colour that makes, and the worst contrast there. */
function shiftFor(stop: string, ink: Ink): { k: number; color: string; worst: number } {
  for (let k = 0; k <= 100; k++) {
    const color = k ? mixHex(toHex(ink.toward), stop, k / 100) : stop
    // the secondary grey is the harder of the two: once it reads, so does the text
    const worst = Math.min(...surfacesOf([color], ink).map((s) => contrast(ink.text2, s)))
    if (worst >= BODY_RATIO) return { k, color, worst }
  }
  return { k: 101, color: toHex(ink.toward), worst: 0 }
}

/** A card's look from its two colours alone: light or dark, and the two stops it is drawn with (each as picked, unless
 *  the text needed it moved). The look that moves the colours least wins. */
export function cardInk(top: string, bottom: string): { light: boolean; top: string; bottom: string } {
  const [t, b] = [top, bottom].map((c) => (parseHex(c) ? c.toLowerCase() : DEFAULT_THEME.top))
  const fit = (ink: Ink) => {
    const [a, z] = [shiftFor(t, ink), shiftFor(b, ink)]
    return { a, z, cost: [Math.max(a.k, z.k), a.k + z.k, -Math.min(a.worst, z.worst)] }
  }
  const [onLight, onDark] = [fit(LIGHT_CARD), fit(DARK_CARD)]
  const i = onLight.cost.findIndex((c, n) => c !== onDark.cost[n])
  const light = i >= 0 && onLight.cost[i] < onDark.cost[i]
  const pick = light ? onLight : onDark
  return { light, top: pick.a.color, bottom: pick.z.color }
}

/** The CSS custom properties a card (or its preview, or the full profile) sets on its root. */
export function cardVars(top: string, bottom: string, look: CardLook = DARK_LOOK): Record<string, string> {
  return cardTheme(top, bottom, look).vars
}

/** cardVars, and whether the card is a light one (dark text). */
export function cardTheme(top: string, bottom: string, look: CardLook = DARK_LOOK): { light: boolean; vars: Record<string, string> } {
  const card = cardInk(top, bottom)
  const ink = card.light ? LIGHT_CARD : DARK_CARD
  const on = surfacesOf([card.top, card.bottom], ink)
  const edge = (c: string) => mixHex(card.light ? '#000000' : '#ffffff', c, EDGE_SHADE)
  const [rawTop, rawBottom] = [top, bottom].map((c) => (parseHex(c) ? c.toLowerCase() : card.top))
  const vars = {
    '--card-top': card.top,
    '--card-bottom': card.bottom,
    // the panels' overlay, and the solid colour it makes at each end (for what must match it: the DND dot's cut-out)
    '--card-panel': css(ink.panel),
    '--panel-top': composite(card.top, ink.panel),
    '--panel-bottom': composite(card.bottom, ink.panel),
    '--card-field': css(ink.field),
    '--card-border-top': edge(card.top),
    '--card-border-bottom': edge(card.bottom),
    '--glow-top': rawTop,
    '--glow-bottom': rawBottom,
    '--glow-top-a': `${rawTop}${GLOW_ALPHA}`,
    '--glow-bottom-a': `${rawBottom}${GLOW_ALPHA}`,
    '--card-text': readable(ink.text, on, BODY_RATIO),
    '--card-text-2': readable(ink.text2, on, BODY_RATIO),
    '--card-line': `rgb(${ink.line} / .1)`,
    '--card-hover': `rgb(${ink.line} / .08)`,
    '--card-skeleton': `rgb(${ink.line} / .12)`,
    '--card-edge': `rgb(${ink.line} / .16)`,
    '--card-danger': readable(ink.danger, on, BODY_RATIO),
    '--card-ok': readable(ink.ok, on, BODY_RATIO),
    // raised things on a light app theme are lifted by a soft shadow
    '--card-lift': look.light ? 'var(--shadow-2)' : '0 0 #0000',
  }
  return { light: card.light, vars }
}

/** Of a gradient's two stops, the one a coloured name reads worst on: the lighter on a dark card, the darker on a
 *  light one (names are moved toward white on dark surfaces and toward black on light ones). */
export function nameStop(top: string, bottom: string, light: boolean): string {
  const l = (c: string) => luminance(parseHex(c) ?? [0, 0, 0])
  return (l(top) > l(bottom)) !== light ? top : bottom
}
