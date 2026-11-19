// Name effects (#468): a third part of a name's style, beside its font and colour. Each effect has a still look (what
// every name shows) and a moving one that plays on hover, in the profile editor and while a full profile is open
// (Name.svelte's `.fx-play`). Neon, Toon, Pop, Gummy, Shimmer and Glitch are drawn from the person's own name colour;
// Prism keeps its rainbow and Ember its fire, with a charcoal ash drifting up. The colours each effect needs are
// worked out here and handed to Name.svelte's CSS as custom properties (no color-mix(): an older WebKitGTK drops it).
import { fromOklch, mixRgb, parseHex, toHex, toOklch, type Rgb } from './color'
import type { Paint } from './nameStyle'

export type NameEffectKey = 'neon' | 'toon' | 'pop' | 'gummy' | 'prism' | 'shimmer' | 'glitch' | 'ember'
export type NameEffect = { key: NameEffectKey; label: string; /** What it does when it plays. */ moves: string }

/** The effects, in the editor's order. The keys are the server's `NAME_EFFECTS` (server/src/profiles/mod.rs). */
export const NAME_EFFECTS: readonly NameEffect[] = [
  { key: 'neon', label: 'Neon', moves: 'Glow pulses' },
  { key: 'toon', label: 'Toon', moves: 'Wobbles' },
  { key: 'pop', label: 'Pop', moves: 'Letters bounce' },
  { key: 'gummy', label: 'Gummy', moves: 'Squishes' },
  { key: 'prism', label: 'Prism', moves: 'Colours flow' },
  { key: 'shimmer', label: 'Shimmer', moves: 'Light sweeps' },
  { key: 'glitch', label: 'Glitch', moves: 'Jitters' },
  { key: 'ember', label: 'Ember', moves: 'Flickers, ash rises' },
]

/** The effect for a stored key; an unknown key (a newer app's) or none draws the name without one. */
export function effectByKey(key: string | null | undefined): NameEffect | null {
  return NAME_EFFECTS.find((e) => e.key === key) ?? null
}

// The effects' own colours: the white core of a glow and a shimmer's light, Prism's rainbow, Ember's fire and ash.
const HIGHLIGHT = '#ffffff'
const PRISM = ['#ff5f6d', '#ffc371', '#47e6a0', '#4fa8ff', '#b16cff', '#ff5f6d']
const FIRE = ['#ff3d00', '#ff9a1f', '#ffe08a']
const FIRE_GLOW = '#ff7a1e'
const ASH = ['#2b2523', '#4a403b']
const BLACK: Rgb = [0, 0, 0]
const WHITE: Rgb = [255, 255, 255]

const shade = (hex: string, toward: Rgb, k: number) => toHex(mixRgb(parseHex(hex)!, toward, k))
/** `hex` with its hue turned `deg` degrees in OKLCH (lightness and chroma kept): a partner colour for Toon's and
 *  Glitch's second shadow when the name has one colour. */
function turn(hex: string, deg: number): string {
  const [L, C, H] = toOklch(parseHex(hex)!)
  return toHex(fromOklch([L, Math.max(C, 0.08), (H + deg + 360) % 360]))
}

/** The two colours a painted name gives an effect (a solid's twice); none for a name in the surface's text colour. */
function stops(paint: Paint): [string, string] | null {
  if (paint.kind === 'solid') return [paint.color, paint.color]
  if (paint.kind === 'gradient') return [paint.from, paint.to]
  return null
}

/** A grapheme-safe split of a name into letters (Pop bounces each one). */
export function letters(text: string): string[] {
  if (typeof Intl !== 'undefined' && 'Segmenter' in Intl) {
    return [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text)].map((s) => s.segment)
  }
  return Array.from(text)
}

/** Each of Pop's letters' colours: a gradient name's colour at that letter (a transformed letter can't show its
 *  parent's clipped gradient), else none (they take the name's colour). */
export function popColors(paint: Paint, count: number): (string | null)[] {
  if (paint.kind !== 'gradient') return Array(count).fill(null)
  const [a, b] = [parseHex(paint.from)!, parseHex(paint.to)!]
  return Array.from({ length: count }, (_, i) => toHex(mixRgb(a, b, count > 1 ? (i * 100) / (count - 1) : 0)))
}

/**
 * The inline CSS an effect adds to a name drawn with `paint`, and whether it replaces the paint's own declarations
 * (Prism, Ember and Gummy fill the letters themselves; a Pop gradient goes letter by letter).
 */
export function effectCss(effect: NameEffect, paint: Paint): { css: string[]; replacesPaint: boolean } {
  const s = stops(paint)
  const css: string[] = [`--fx-hi: ${HIGHLIGHT}`]
  if (s) {
    const [a, b] = s
    const partner = paint.kind === 'gradient' ? b : turn(a, 150)
    css.push(`--fx-a: ${a}`, `--fx-b: ${partner}`, `--fx-dark: ${shade(a, BLACK, 62)}`, `--fx-glow: ${shade(a, BLACK, 20)}`)
    if (effect.key === 'glitch') css.push(`--fx-g1: ${turn(a, 60)}`, `--fx-g2: ${turn(a, -60)}`)
  }
  const fill = (image: string, size?: string) => [
    `background-image: ${image}`, ...(size ? [`background-size: ${size}`] : []),
    '-webkit-background-clip: text', 'background-clip: text', '-webkit-text-fill-color: transparent',
  ]
  switch (effect.key) {
    case 'prism':
      return { css: [...css, ...fill(`linear-gradient(90deg, ${PRISM.join(', ')})`, '300% 100%')], replacesPaint: true }
    case 'ember':
      return {
        css: [...css, ...fill(`linear-gradient(0deg, ${FIRE[0]}, ${FIRE[1]} 55%, ${FIRE[2]})`), `--fx-fire: ${FIRE_GLOW}`, `--fx-ash: ${ASH[0]}`, `--fx-ash-2: ${ASH[1]}`],
        replacesPaint: true,
      }
    case 'gummy': {
      if (!s) return { css, replacesPaint: false }
      const [a, b] = s
      const top = shade(a, WHITE, 45)
      const bottom = paint.kind === 'gradient' ? b : shade(a, BLACK, 22)
      return { css: [...css, ...fill(`linear-gradient(180deg, ${top} 0%, ${a} 55%, ${bottom} 100%)`)], replacesPaint: true }
    }
    case 'shimmer': {
      const [a, b] = s ?? ['currentColor', 'currentColor']
      return { css: [...css, ...fill(`linear-gradient(100deg, ${a} 0%, ${a} 40%, ${HIGHLIGHT} 50%, ${b} 60%, ${b} 100%)`, '250% 100%')], replacesPaint: true }
    }
    case 'pop':
      return { css, replacesPaint: paint.kind === 'gradient' }
    default:
      return { css, replacesPaint: false }
  }
}
