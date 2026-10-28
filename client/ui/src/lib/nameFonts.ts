// The eight bundled name fonts (plan reconciliation 9). `key` equals the server's NAME_FONTS;
// `scale` is the per-font size factor so names line up. A null key means the app font.
export type NameFont = { key: string; label: string; family: string; weight: number; scale: number }

const stack = (family: string) => `'${family}', 'Onest', system-ui, sans-serif`

export const NAME_FONTS: NameFont[] = [
  { key: 'cormorant-garamond', label: 'Elegant', family: stack('Cormorant Garamond'), weight: 600, scale: 1.15 },
  { key: 'fredoka', label: 'Rounded', family: stack('Fredoka'), weight: 500, scale: 1.0 },
  { key: 'press-start-2p', label: 'Pixel', family: stack('Press Start 2P'), weight: 400, scale: 0.68 },
  { key: 'pacifico', label: 'Script', family: stack('Pacifico'), weight: 400, scale: 0.95 },
  { key: 'caveat', label: 'Handwritten', family: stack('Caveat'), weight: 600, scale: 1.2 },
  { key: 'space-grotesk', label: 'Grotesque', family: stack('Space Grotesk'), weight: 600, scale: 1.0 },
  { key: 'bricolage-grotesque', label: 'Quirky', family: stack('Bricolage Grotesque'), weight: 600, scale: 1.0 },
  { key: 'orbitron', label: 'Techy', family: stack('Orbitron'), weight: 600, scale: 0.86 },
]

export function fontByKey(key: string | null): NameFont | null {
  return NAME_FONTS.find((f) => f.key === key) ?? null
}
