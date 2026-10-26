import type { CustomEmoji } from './protocol/CustomEmoji'
import data from './emoji-data.json'

/**
 * Where the webview loads a stored image: the core's `pulse-media` scheme, which adds the login
 * and caches on disk. WebView2 (Windows) exposes custom schemes as `http://<scheme>.localhost`.
 */
export function mediaUrl(hash: string, ua: string = navigator.userAgent): string {
  return ua.includes('Windows') ? `http://pulse-media.localhost/${hash}` : `pulse-media://localhost/${hash}`
}

/** A stored image's URL through the core's scheme, as `mediaUrl` builds it (either platform's form). */
const MEDIA_URL = /^(pulse-media:\/\/localhost|http:\/\/pulse-media\.localhost)\/[0-9a-f]{64}$/
export const isMediaUrl = (src: string): boolean => MEDIA_URL.test(src)
/** A stored image's content hash. */
export const isMediaHash = (h: string): boolean => /^[0-9a-f]{64}$/.test(h)

/** One standard emoji: `u` Unicode, `n` primary shortcode, `k` keywords + other shortcodes, `g` index into `groups`, `s` the 5 skin-tone variants. */
export type Std = { u: string; n: string; k: string[]; g: number; s?: string[] }

export const groups: { key: string; label: string }[] = data.groups
export const standard: Std[] = data.emoji

// Primary names win over aliases, so a keyword can never shadow another emoji's own name.
const byName = new Map<string, Std>()
for (const e of standard) byName.set(e.n, e)
for (const e of standard) for (const k of e.k) if (!byName.has(k)) byName.set(k, e)

export function byShortcode(name: string): Std | undefined {
  return byName.get(name.toLowerCase())
}

const MAX_RESULTS = 50

/** Prefix matches on names/shortcodes first (custom before standard), then keyword matches. */
export function search(q: string, custom: CustomEmoji[]): Array<Std | CustomEmoji> {
  const needle = q.trim().replace(/^:|:$/g, '').toLowerCase()
  if (!needle) return []
  const live = custom.filter((c) => c.hash !== null)
  const out: Array<Std | CustomEmoji> = live.filter((c) => c.name.toLowerCase().startsWith(needle))
  const seen = new Set<Std>()
  for (const e of standard) if (e.n.startsWith(needle)) { out.push(e); seen.add(e) }
  for (const e of standard) if (!seen.has(e) && e.k.some((k) => k.startsWith(needle))) { out.push(e); seen.add(e) }
  for (const e of standard) if (!seen.has(e) && e.k.some((k) => k.includes(needle))) out.push(e)
  return out.slice(0, MAX_RESULTS)
}

/**
 * The key an emoji is stored under, as the server computes it: U+FE0F only asks for emoji
 * presentation, so `❤` and `❤️` are one emoji. `custom:<id>` keys never contain it.
 */
export function canonicalKey(u: string): string {
  return u.replaceAll('\uFE0F', '')
}

// Canonical key -> the sequence as emoji-data spells it, which is what Twemoji's file names follow.
const spelled = new Map<string, string>()
for (const e of standard) for (const u of [e.u, ...(e.s ?? [])]) spelled.set(canonicalKey(u), u)

/**
 * Twemoji's file rule: code points joined by '-', FE0F dropped unless the sequence is a ZWJ one.
 * A canonical key (FE0F stripped) is first spelled out again, so a ZWJ file that needs FE0F is found.
 */
export function twemojiUrl(u: string): string {
  u = spelled.get(canonicalKey(u)) ?? u
  const cps = [...u].map((c) => c.codePointAt(0)!.toString(16))
  return `/emoji/${(cps.includes('200d') ? cps : cps.filter((c) => c !== 'fe0f')).join('-')}.svg`
}

/** 0 is the default yellow; 1..5 pick a skin tone where the emoji has them. */
export type Tone = 0 | 1 | 2 | 3 | 4 | 5

export function withSkin(e: Std, tone: Tone): string {
  return (tone > 0 && e.s?.[tone - 1]) || e.u
}

const TONE_KEY = 'pulse.skinTone'

/** The skin tone last chosen in the picker (this device only). */
export function savedTone(): Tone {
  try {
    const n = Number(localStorage.getItem(TONE_KEY))
    return n >= 1 && n <= 5 ? (n as Tone) : 0
  } catch {
    return 0
  }
}

export function saveTone(t: Tone): void {
  try {
    localStorage.setItem(TONE_KEY, String(t))
  } catch {
    /* ignore */
  }
}

/** How a custom emoji is written in message text. */
export function customToken(e: CustomEmoji): string {
  return `<${e.animated ? 'a' : ''}:${e.name}:${e.id}>`
}

/** `<:name:id>` / `<a:name:id>`. Not global: clone with `new RegExp(TOKEN_RE, 'g')` to scan text. */
export const TOKEN_RE = /<(a?):([A-Za-z0-9_]{2,32}):([A-Za-z0-9]+)>/

/** The key a reaction is stored under: the canonical Unicode, or `custom:<id>`. */
export function reactionKey(e: Std | CustomEmoji): string {
  return 'u' in e ? canonicalKey(e.u) : `custom:${e.id}`
}

/** "Big Pog!.gif" -> "Big_Pog": emoji names are 2-32 of [A-Za-z0-9_]. */
export function defaultName(file: string): string {
  const base = file.replace(/\.[^.]*$/, '').replace(/[^A-Za-z0-9_]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 32)
  return base.padEnd(2, '_')
}

/** The server's rule for an emoji name (checked again there). */
export const isValidEmojiName = (n: string): boolean => /^[A-Za-z0-9_]{2,32}$/.test(n)
