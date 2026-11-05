// @vitest-environment jsdom
// Theming step 1 (#42, docs/superpowers/specs/2026-11-03-theming-design.md "S — tokens only"): every
// UI colour comes from an app.css :root token, so a theme only has to swap token values. Colours
// that are fixed on purpose (video chrome, the QR code, skin tones, embed brand colours, the profile
// card's own theme, avatar initials) are listed below, each with its reason. The token values are
// pinned to what the app looked like before the tokens existed: this step changes no pixel.
import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'

const src = (p: string) => readFileSync(resolve(__dirname, p), 'utf8')

let root: CSSStyleDeclaration
beforeAll(() => {
  const style = document.createElement('style')
  style.textContent = src('app.css')
  document.head.append(style)
  // Pulse Dark: the default block (`:root, [data-theme='dark']`); the other themes are checked in themeCss.test.ts
  const found = [...style.sheet!.cssRules].find((r): r is CSSStyleRule => r instanceof CSSStyleRule && /^:root,\s*\[data-theme=["']dark["']\]$/.test(r.selectorText))
  root = found!.style
})
// jsdom drops the spaces after commas in a custom property's value
const norm = (v: string) => v.trim().replace(/,\s+/g, ',')
const token = (name: string) => norm(root.getPropertyValue(name))

const svelteFiles = (() => {
  const files: string[] = []
  const walk = (dir: string) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) walk(join(dir, e.name))
      else if (e.name.endsWith('.svelte')) files.push(join(dir, e.name))
    }
  }
  walk(__dirname)
  return files
})()
const rel = (f: string) => f.slice(__dirname.length + 1)

/** Colour literals: hex, the colour functions, and white/black as a CSS value (not white-space). */
const LITERAL = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\([^)]*\)/g
const NAMED = /(?<![-\w$.])(?:white|black|red|green|blue|yellow|orange|purple|pink|gr[ae]y|silver|navy|teal|maroon|olive|lime|aqua|fuchsia)(?![-\w])/g
const styleBlocks = (text: string) => [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1])
/** The file without comments: `#112` in "Recently deleted (#112)" is an issue, not a colour. Line comments keep a
 *  preceding `:` (so a URL's `://` stays). */
const withoutComments = (text: string) =>
  text.replace(/<!--[\s\S]*?-->/g, '').replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1')
const literalsIn = (text: string) => {
  const found = new Set(withoutComments(text).match(LITERAL) ?? [])
  // named colours only inside declarations (comments and markup text say "white" too)
  for (const css of styleBlocks(text)) {
    for (const decl of css.replace(/\/\*[\s\S]*?\*\//g, '').match(/:[^;{}]*/g) ?? []) for (const n of decl.match(NAMED) ?? []) found.add(n)
  }
  return found
}

const VIDEO = 'video chrome: drawn over black video, frames or a thumbnail, so white-on-black in every theme'
const AVATAR = 'initials on the fixed fallback avatar palette (lib/avatar.ts): white reads on every swatch'

/** Colour literals allowed to stay in a component, each with why it is fixed. */
const ALLOWED: Record<string, { literals: string[]; reason: string }[]> = {
  'components/Avatar.svelte': [{ literals: ['#fff'], reason: AVATAR }],
  'routes/Shell.svelte': [{ literals: ['#fff'], reason: AVATAR + ' (the server chip)' }],
  'components/ClipCard.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, .55)', 'rgba(0, 0, 0, .75)', 'rgba(0, 0, 0, .6)'], reason: VIDEO }],
  'components/ClipPlayer.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, 0.6)', 'rgba(0, 0, 0, 0.8)', 'rgba(0, 0, 0, 0.85)'], reason: VIDEO }],
  'components/ClipEditor.svelte': [{ literals: ['#000'], reason: VIDEO }],
  'components/ClipTimeline.svelte': [{ literals: ['rgba(0, 0, 0, 0.5)', 'rgba(0, 0, 0, 0.6)'], reason: VIDEO + ' (the filmstrip)' }],
  'components/PendingClip.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, .75)'], reason: VIDEO }],
  'components/FullView.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, .5)'], reason: VIDEO }],
  'components/StreamTile.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, .5)', 'rgba(0, 0, 0, .55)', 'rgba(0, 0, 0, .75)', 'rgba(255, 255, 255, .16)'], reason: VIDEO }],
  'components/MiniPreview.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, .55)', 'rgba(0, 0, 0, .6)'], reason: VIDEO }],
  'components/LiveCard.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, .55)', 'rgba(0, 0, 0, .7)', 'rgba(255, 255, 255, .4)', 'rgba(255, 255, 255, .8)'], reason: VIDEO }],
  'components/GoLiveDialog.svelte': [{ literals: ['#000'], reason: VIDEO + ' (the preview)' }],
  'components/CropDialog.svelte': [{ literals: ['#000', '#fff', 'rgba(0, 0, 0, .6)'], reason: VIDEO + ' (the image being cropped, its mask and crop box)' }],
  'components/StreamDiagOverlay.svelte': [{ literals: ['rgba(0, 0, 0, .65)', '#e6e6e6'], reason: 'developer stream diagnostics, a fixed dark box over the video' }],
  'components/LinkEmbed.svelte': [
    { literals: ['#ff3344', '#fe2c55', '#1ed760', '#ff5500', '#9146ff', '#e1306c'], reason: "the providers' brand colours (the embed's left edge)" },
    { literals: ['#000', '#fff', 'rgba(10, 11, 13, .62)', 'rgba(10, 11, 13, .82)', 'rgba(0, 0, 0, .35)'], reason: VIDEO + ' (the player frame and the play button on the thumbnail)' },
  ],
  'components/TwoFactorSettings.svelte': [{ literals: ['#fff', '#000'], reason: 'the QR code: white quiet zone, black modules, or scanners fail' }],
  'components/EmojiPicker.svelte': [{ literals: ['#f7c948', '#f7dbc0', '#e8c29a', '#c89369', '#9a6a45', '#5f402c'], reason: 'skin-tone swatches: content colours' }],
  'components/AppearanceSettings.svelte': [{ literals: ['#3b4bd8'], reason: 'a sample role colour in the Name readability preview (user data, not UI)' }],
  'components/ProfileSettings.svelte': [{ literals: ['#ffffff'], reason: "the first custom name colour's starting value (user data, not UI)" }],
  'components/FullProfile.svelte': [{ literals: ['#000'], reason: "the right column's round-cornered scroll mask: only its alpha counts, never shown as a colour" }],
}

describe('colour literals', () => {
  it('no component hard-codes a colour outside the allow-list', () => {
    expect(svelteFiles.length).toBeGreaterThan(50)
    const offenders: string[] = []
    for (const f of svelteFiles) {
      const allowed = new Set((ALLOWED[rel(f)] ?? []).flatMap((e) => e.literals))
      for (const lit of literalsIn(readFileSync(f, 'utf8'))) if (!allowed.has(lit)) offenders.push(`${rel(f)}: ${lit}`)
    }
    expect(offenders).toEqual([])
  })

  it('every allow-list entry still exists and says why (no stale entries)', () => {
    const stale: string[] = []
    for (const [file, entries] of Object.entries(ALLOWED)) {
      const found = literalsIn(src(file))
      for (const e of entries) {
        expect(e.reason.length, file).toBeGreaterThan(10)
        for (const lit of e.literals) if (!found.has(lit)) stale.push(`${file}: ${lit}`)
      }
    }
    expect(stale).toEqual([])
  })

  it('app.css keeps its colour values in the theme blocks only', () => {
    const outside = src('app.css')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/(^|\n)(:root, )?\[data-theme='[a-z]+'\](\[data-tint='[a-z]+'\])? \{[\s\S]*?\n\}/g, '')
    expect(outside.match(LITERAL) ?? []).toEqual([])
  })
})

describe('the tokens (values pinned to the look before they existed: no visual change)', () => {
  it.each([
    // white hover tints on rows (messages, quotes, clips) and a quote's line
    ['--hover-tint', 'rgba(255, 255, 255, .02)'],
    ['--hover-tint-strong', 'rgba(255, 255, 255, .03)'],
    // the accent family: today's accent as plain rgba() (see the comment in app.css)
    ['--accent-soft', 'rgba(139, 156, 255, .15)'],
    ['--focus-soft', 'rgba(139, 156, 255, .45)'],
    ['--accent-faint', 'rgba(139, 156, 255, .08)'],
    ['--ok-soft', 'rgba(79, 209, 139, .12)'],
    ['--ok-deep', '#3a7d5a'],
    ['--danger-hatch', 'rgba(217, 58, 66, .55)'],
    ['--line-strong', '#44474f'],
    ['--presence-off', '#5b5f6b'],
    ['--tooltip-bg', '#0f1013'],
    ['--tooltip-text', '#e8e9ed'],
    ['--scrim-strong', 'rgba(10, 11, 13, .85)'],
    // three elevation levels (step 2 merged the six shadows): the jump pill; popups and notices; menus and dialogs
    ['--shadow-1', '0 4px 14px rgba(0, 0, 0, .35)'],
    ['--shadow-2', '0 8px 24px rgba(0, 0, 0, .4)'],
    ['--shadow-3', '0 18px 48px rgba(8, 9, 12, .5)'],
    // the Quote of the Year island: today's text colours, fixed so they read on --quote-surface in every theme
    ['--island-text', '#e8e9ed'],
    ['--island-text-2', '#b9bcc6'],
    ['--island-text-3', '#8e92a0'],
    // text vs fill: the same colour today; a theme may make the text one darker or lighter
    ['--accent-text', 'var(--accent)'],
    ['--warn-text', 'var(--warn)'],
    ['--ok-text', 'var(--ok)'],
    ['--quote-green-text', 'var(--quote-green)'],
    // the cut-out ring around avatars, presence dots and speaking rings
    ['--ring', 'var(--bg-1)'],
  ])('%s is %s', (name, value) => {
    expect(token(name)).toBe(norm(value))
  })

  it('the island and tooltip text match today\'s --text ladder', () => {
    expect(token('--island-text')).toBe(token('--text'))
    expect(token('--island-text-2')).toBe(token('--text-2'))
    expect(token('--island-text-3')).toBe(token('--text-3'))
    expect(token('--tooltip-text')).toBe(token('--text'))
  })

  it('the old shadow names are gone (#118): everything uses --shadow-1/2/3', () => {
    const OLD = /--shadow-(menu|pop|float|notice|notice-soft|picker|jump)\b/
    const offenders: string[] = []
    for (const f of [...svelteFiles, resolve(__dirname, 'app.css')]) {
      for (const line of readFileSync(f, 'utf8').split('\n')) if (OLD.test(line)) offenders.push(`${rel(f)}: ${line.trim()}`)
    }
    expect(offenders).toEqual([])
  })

  it('the unused --on-ok is gone', () => {
    expect(token('--on-ok')).toBe('')
    for (const f of svelteFiles) expect(readFileSync(f, 'utf8'), rel(f)).not.toContain('--on-ok')
  })
})

describe('text vs fill', () => {
  // Over video or on a dark island the bright fill colour is right in every theme.
  const FILL_AS_TEXT: Record<string, RegExp> = {
    'components/ClipEditor.svelte': /^\s*\.problem /, // over the black video screen
    'components/QotyCard.svelte': /^\s*\.(big|who) /, // on the Quote of the Year island
    'components/StreamDiagOverlay.svelte': /^\s*\.bad /, // the diagnostics box over video
  }
  it('accent, warn, ok and quote green as text use their -text token', () => {
    const offenders: string[] = []
    for (const f of [...svelteFiles, resolve(__dirname, 'app.css')]) {
      const allowed = FILL_AS_TEXT[rel(f)]
      for (const line of readFileSync(f, 'utf8').split('\n')) {
        if (/(^|[\s;{])(color|caret-color|-webkit-text-fill-color):\s*var\(--(accent|warn|ok|quote-green)\)/.test(line) && !(allowed && allowed.test(line))) offenders.push(`${rel(f)}: ${line.trim()}`)
      }
    }
    expect(offenders).toEqual([])
  })
})

describe('text on a fill (#42)', () => {
  // A theme's --text turns dark on Light: on an accent, danger or quote-green fill the text is the fill's own --on-*.
  it('a rule that fills with accent, danger or quote green colours its text with the matching --on- token', () => {
    const offenders: string[] = []
    for (const f of [...svelteFiles, resolve(__dirname, 'app.css')]) {
      for (const line of readFileSync(f, 'utf8').split('\n')) {
        const fill = /background(?:-color)?: var\(--(accent|danger|quote-green)\)/.exec(line)?.[1]
        const text = /(?:^|[\s;{])color: var\(--([\w-]+)\)/.exec(line)?.[1]
        if (fill && text && text !== `on-${fill}`) offenders.push(`${rel(f)}: ${line.trim()}`)
      }
    }
    expect(offenders).toEqual([])
  })

  // Any accent can be picked, even one that vanishes on the theme's surfaces; a border or outline that marks a selection
  // or focus uses --focus-ring (the accent moved to 3:1 there). Over video the bright accent itself is right.
  it('accent borders, outlines and edge shadows use --focus-ring', () => {
    const OVER_VIDEO = /^\s*\.open:focus-visible /
    const offenders: string[] = []
    for (const f of [...svelteFiles, resolve(__dirname, 'app.css')]) {
      for (const line of readFileSync(f, 'utf8').split('\n')) {
        if (/(border(-color|-left|-right|-bottom|-top)?|outline(-color)?|box-shadow): [^;]*var\(--accent\)/.test(line) && !(rel(f) === 'components/MiniPreview.svelte' && OVER_VIDEO.test(line))) offenders.push(`${rel(f)}: ${line.trim()}`)
      }
    }
    expect(offenders).toEqual([])
  })
})

describe('the Quote of the Year island', () => {
  it('its text uses the island tokens, never the theme text ladder', () => {
    const banner = styleBlocks(src('components/QotyBanner.svelte'))[0]
    expect(banner).toMatch(/\.qoty-banner \{[^}]*[\s;]color: var\(--island-text\)/)
    expect(banner).not.toMatch(/var\(--text/)
    const card = styleBlocks(src('components/QotyCard.svelte'))[0]
    expect(card).toMatch(/\.qoty-card \{[^}]*[\s;]color: var\(--island-text\)/)
    // the no-vote line (.none, .time) sits in the chat, not on the island
    const islandLines = card.split('\n').filter((l) => /var\(--text/.test(l))
    expect(islandLines.map((l) => l.trim().split(' ')[0])).toEqual(['.none', '.time'])
    // the voter stack's "+3" is a child component's text, on the island
    expect(card).toContain('.tally :global(.stack .more) { color: var(--island-text-3); }')
    expect(src('components/VoterStack.svelte')).toMatch(/\.more \{[^}]*color: var\(--text-3\)/)
  })
})

describe('the tooltip', () => {
  it('is a fixed dark chip with its own text colour and shadow', () => {
    const css = src('app.css')
    const tip = /\.pulse-tooltip \{([^}]*)\}/.exec(css)![1]
    expect(tip).toContain('background: var(--tooltip-bg)')
    expect(tip).toContain('color: var(--tooltip-text)')
    expect(tip).toContain('box-shadow: var(--shadow-2)')
  })
})
