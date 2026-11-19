// @vitest-environment jsdom
// Theming step 1 (#42, docs/superpowers/specs/2026-11-03-theming-design.md "S — tokens only"): every
// UI colour comes from an app.css :root token, so a theme only has to swap token values. Colours
// that are fixed on purpose (video chrome, the QR code, skin tones, the profile
// card's own theme, avatar initials) are listed below, each with its reason and how many times it is
// used (#116), so a new use of an allowed literal is caught too. Components, the .ts colour sources and
// the other stylesheets are all scanned. The token values are pinned to what the app looked like
// before the tokens existed: this step changes no pixel.
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

const filesWhere = (keep: (name: string) => boolean) => {
  const files: string[] = []
  const walk = (dir: string) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) walk(join(dir, e.name))
      else if (keep(e.name)) files.push(join(dir, e.name))
    }
  }
  walk(__dirname)
  return files
}
const svelteFiles = filesWhere((n) => n.endsWith('.svelte'))
/** Code that can hold a colour: not tests, not type declarations. */
const tsFiles = filesWhere((n) => n.endsWith('.ts') && !n.endsWith('.test.ts') && !n.endsWith('.d.ts'))
/** app.css has its own check below (colours only in the theme blocks). */
const cssFiles = filesWhere((n) => n.endsWith('.css') && n !== 'app.css')
const rel = (f: string) => f.slice(__dirname.length + 1)

/** Colour literals: hex, the colour functions, and white/black as a CSS value (not white-space). */
const LITERAL = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\([^)]*\)/g
const NAMED = /(?<![-\w$.])(?:white|black|red|green|blue|yellow|orange|purple|pink|gr[ae]y|silver|navy|teal|maroon|olive|lime|aqua|fuchsia)(?![-\w])/g
const styleBlocks = (text: string) => [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1])
/** The file without comments: `#112` in "Recently deleted (#112)" is an issue, not a colour. Line comments keep a
 *  preceding `:` (so a URL's `://` stays). */
const withoutComments = (text: string) =>
  text.replace(/<!--[\s\S]*?-->/g, '').replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1')
/** The CSS a file holds: its <style> blocks, inline `style="…"` strings and `style:prop="…"` values (a .svelte
 *  file), or all of it (a stylesheet). */
const cssIn = (text: string, file: string) => {
  if (file.endsWith('.css')) return [text]
  const markup = withoutComments(text.replace(/<style[^>]*>[\s\S]*?<\/style>/g, ''))
  const inline = [...markup.matchAll(/\bstyle=(["'])([\s\S]*?)\1/g)].map((m) => `{${m[2]}}`)
  const directives = [...markup.matchAll(/\bstyle:([\w-]+)(?:\|important)?=(?:(["'])([\s\S]*?)\2|\{\s*(["'`])([\s\S]*?)\4\s*\})/g)]
    .map((m) => `{${m[1]}: ${m[3] ?? m[5]}}`)
  return [...styleBlocks(text), ...inline, ...directives]
}
/** A named colour set from script: a canvas fill or stroke, or a style property (`el.style.color = 'white'`). */
const SCRIPT_NAMED = /\b(?:fillStyle|strokeStyle|color|background(?:Color)?|borderColor|outlineColor|fill|stroke)\s*[:=]\s*(["'`])([^"'`]*)\1/g

/** Every colour literal in a file, with how many times it appears. */
const literalCounts = (text: string, file: string) => {
  const found = new Map<string, number>()
  const add = (lit: string) => found.set(lit, (found.get(lit) ?? 0) + 1)
  for (const lit of withoutComments(text).match(LITERAL) ?? []) add(lit)
  // named colours only inside declarations (comments and markup text say "white" too)
  for (const css of cssIn(text, file)) {
    for (const decl of css.replace(/\/\*[\s\S]*?\*\//g, '').match(/:[^;{}]*/g) ?? []) for (const n of decl.match(NAMED) ?? []) add(n)
  }
  if (file.endsWith('.ts')) {
    for (const m of withoutComments(text).matchAll(SCRIPT_NAMED)) for (const n of m[2].match(NAMED) ?? []) add(n)
  }
  return found
}
const literalsIn = (text: string, file = 'x.svelte') => new Set(literalCounts(text, file).keys())

const VIDEO = 'video chrome: drawn over black video, frames or a thumbnail, so white-on-black in every theme'
const AVATAR = 'initials on the fixed fallback avatar palette (lib/avatar.ts): white reads on every swatch'

const CONTENT = 'content colours, not UI: a theme never recolours them'

/** Colour literals allowed to stay in a file: each with why it is fixed and how many times it is used. A new use of
 *  an allowed literal changes the count and fails, so it gets looked at (#116). */
type Allowed = { uses: Record<string, number>; reason: string }
/** A palette file (a table of content colours): only its number of literals is pinned. */
type Palette = { total: number; reason: string }
const ALLOWED: Record<string, (Allowed | Palette)[]> = {
  'components/Avatar.svelte': [{ uses: { '#fff': 1 }, reason: AVATAR }],
  'routes/Shell.svelte': [{ uses: { '#fff': 1 }, reason: AVATAR + ' (the server chip)' }],
  'components/QuickSwitcher.svelte': [{ uses: { '#fff': 1 }, reason: AVATAR + " (a server row's chip, as the tab's)" }],
  'components/ServerChip.svelte': [{ uses: { '#fff': 1 }, reason: AVATAR + " (a server's chip in invites, as the tab's)" }],
  'components/ClipCard.svelte': [{ uses: { '#000': 2, '#fff': 3, 'rgba(0, 0, 0, .55)': 1, 'rgba(0, 0, 0, .75)': 2, 'rgba(0, 0, 0, .6)': 1 }, reason: VIDEO }],
  'components/ClipDownload.svelte': [
    { uses: { '#fff': 2, 'rgba(0, 0, 0, .6)': 2, 'rgba(0, 0, 0, .75)': 2, 'rgba(255, 255, 255, .25)': 1 }, reason: VIDEO + ' (the download icon and Show in folder)' },
  ],
  'components/ClipPlayer.svelte': [{ uses: { '#000': 2, '#fff': 3, 'rgba(0, 0, 0, 0.6)': 1, 'rgba(0, 0, 0, 0.8)': 1, 'rgba(0, 0, 0, 0.85)': 2 }, reason: VIDEO }],
  'components/ClipEditor.svelte': [{ uses: { '#000': 1 }, reason: VIDEO }],
  'components/ClipTimeline.svelte': [{ uses: { 'rgba(0, 0, 0, 0.5)': 1, 'rgba(0, 0, 0, 0.6)': 1 }, reason: VIDEO + ' (the filmstrip)' }],
  'components/PendingClip.svelte': [{ uses: { '#000': 1, '#fff': 1, 'rgba(0, 0, 0, .75)': 1 }, reason: VIDEO }],
  'components/FullView.svelte': [{ uses: { '#000': 2, '#fff': 1, 'rgba(0, 0, 0, .5)': 1 }, reason: VIDEO }],
  'components/StreamTile.svelte': [
    { uses: { '#000': 2, '#fff': 4, 'rgba(0, 0, 0, .5)': 1, 'rgba(0, 0, 0, .55)': 1, 'rgba(0, 0, 0, .75)': 1, 'rgba(255, 255, 255, .16)': 1 }, reason: VIDEO },
  ],
  'components/MiniPreview.svelte': [{ uses: { '#000': 2, '#fff': 1, 'rgba(0, 0, 0, .55)': 1 }, reason: VIDEO }],
  'components/LiveCard.svelte': [
    { uses: { '#000': 2, '#fff': 3, 'rgba(0, 0, 0, .55)': 3, 'rgba(0, 0, 0, .7)': 1, 'rgba(255, 255, 255, .4)': 1, 'rgba(255, 255, 255, .8)': 1 }, reason: VIDEO },
  ],
  'components/GoLiveDialog.svelte': [{ uses: { '#000': 1 }, reason: VIDEO + ' (the preview)' }],
  'components/CropDialog.svelte': [{ uses: { '#000': 1, '#fff': 1, 'rgba(0, 0, 0, .6)': 1 }, reason: VIDEO + ' (the image being cropped, its mask and crop box)' }],
  'components/StreamDiagOverlay.svelte': [{ uses: { 'rgba(0, 0, 0, .65)': 1, '#e6e6e6': 1 }, reason: 'developer stream diagnostics, a fixed dark box over the video' }],
  'components/LinkEmbed.svelte': [
    { uses: { '#000': 1, '#fff': 1, 'rgba(10, 11, 13, .62)': 1, 'rgba(10, 11, 13, .82)': 1, 'rgba(0, 0, 0, .35)': 1 }, reason: VIDEO + ' (the player frame and the play button on the thumbnail)' },
  ],
  'components/TwoFactorSettings.svelte': [{ uses: { '#fff': 1, '#000': 1 }, reason: 'the QR code: white quiet zone, black modules, or scanners fail' }],
  'components/EmojiPicker.svelte': [
    { uses: { '#f7c948': 1, '#f7dbc0': 1, '#e8c29a': 1, '#c89369': 1, '#9a6a45': 1, '#5f402c': 1 }, reason: 'skin-tone swatches: content colours' },
  ],
  'components/AppearanceSettings.svelte': [{ uses: { '#3b4bd8': 1 }, reason: 'a sample role colour in the Name readability preview (user data, not UI)' }],
  'components/ProfileSettings.svelte': [{ uses: { '#ffffff': 1 }, reason: "the first custom name colour's starting value (user data, not UI)" }],
  'components/FullProfile.svelte': [{ uses: { '#000': 7 }, reason: "the right column's round-cornered scroll mask: only its alpha counts, never shown as a colour" }],
  // the .ts colour sources
  'lib/stream/stream.svelte.ts': [{ uses: { '#000': 1 }, reason: VIDEO + ' (the letterbox the stream canvas paints round a frame)' }],
  'lib/theme.svelte.ts': [{ uses: { '#000000': 1 }, reason: 'a dummy accent, only to list the token names deriveAccent sets (never shown)' }],
  'lib/color.ts': [
    { uses: { 'rgba(hex: string, alpha: number)': 1, 'rgba(${r}, ${g}, ${b}, ${String(alpha)': 1 }, reason: 'the rgba() helper itself: its signature and the string it builds, no colour of its own' },
  ],
  'lib/avatar.ts': [{ total: 8, reason: 'the fallback avatar palette: ' + CONTENT }],
  'lib/nameStyle.ts': [{ total: 15, reason: 'the name colour presets people pick from: ' + CONTENT }],
  'lib/nameEffects.ts': [{ total: 13, reason: "the name effects' own colours (#468: a glow's white core, Prism's rainbow, Ember's fire and ash): " + CONTENT }],
  'lib/packColors.ts': [{ total: 38, reason: "the invite pack's gold, holo rainbow and the light and shade on its foil (#511): " + CONTENT }],
  'lib/cardTheme.ts': [{ total: 30, reason: "the profile card's own themes, drawn the same in every app theme: " + CONTENT }],
  'lib/themes.ts': [{ total: 79, reason: 'the theme table app.css mirrors (themeCss.test.ts keeps the two equal) and the accent presets' }],
  'lib/wheelColours.ts': [{ total: 3, reason: "the picker wheel's slices and their label colours (see below): " + CONTENT }],
}

describe('colour literals', () => {
  const scanned = [...svelteFiles, ...tsFiles, ...cssFiles]
  const pinned = (file: string) => {
    const uses = new Map<string, number>()
    let total: number | null = null
    for (const e of ALLOWED[file] ?? []) {
      if ('total' in e) total = (total ?? 0) + e.total
      else for (const [lit, n] of Object.entries(e.uses)) uses.set(lit, (uses.get(lit) ?? 0) + n)
    }
    return { uses, total }
  }

  it('no file hard-codes a colour outside the allow-list, nor uses an allowed one more often', () => {
    expect(svelteFiles.length).toBeGreaterThan(50)
    expect(tsFiles.length).toBeGreaterThan(50)
    expect(cssFiles.map(rel).sort()).toEqual(['fonts.css', 'nameFonts.css'])
    const offenders: string[] = []
    for (const f of scanned) {
      const found = literalCounts(readFileSync(f, 'utf8'), f)
      const { uses, total } = pinned(rel(f))
      if (total !== null) {
        const n = [...found.values()].reduce((a, b) => a + b, 0)
        if (n !== total) offenders.push(`${rel(f)}: ${n} colour literals, ${total} allowed`)
        continue
      }
      for (const [lit, n] of found) {
        const allowed = uses.get(lit) ?? 0
        if (n > allowed) offenders.push(`${rel(f)}: ${lit}${allowed ? ` used ${n} times, ${allowed} allowed` : ''}`)
      }
    }
    expect(offenders).toEqual([])
  })

  it('every allow-list entry still exists, as often as it says, and says why (no stale entries)', () => {
    const stale: string[] = []
    for (const [file, entries] of Object.entries(ALLOWED)) {
      for (const e of entries) expect(e.reason.length, file).toBeGreaterThan(10)
      const found = literalCounts(src(file), file)
      const { uses } = pinned(file)
      for (const [lit, n] of uses) if ((found.get(lit) ?? 0) !== n) stale.push(`${file}: ${lit} used ${found.get(lit) ?? 0} times, ${n} allowed`)
    }
    expect(stale).toEqual([])
  })

  it('finds colours in inline styles, style: directives, stylesheets and script', () => {
    const svelte = (t: string) => [...literalsIn(t, 'X.svelte')]
    expect(svelte('<div style="color: white; gap: 4px"></div>')).toEqual(['white'])
    expect(svelte("<div style='background:black'></div>")).toEqual(['black'])
    expect(svelte('<div style:color="white" style:background-color={\'black\'}></div>')).toEqual(['white', 'black'])
    expect(svelte('<div style:color={`#123456`}></div>')).toEqual(['#123456'])
    // words in markup text and attribute names aren't colours
    expect(svelte('<p class="white-space">Black or white?</p>')).toEqual([])
    expect([...literalsIn('.a { color: white; }', 'x.css')]).toEqual(['white'])
    expect([...literalsIn("ctx.fillStyle = 'black'\nel.style.color = 'white'", 'x.ts')]).toEqual(['black', 'white'])
    expect([...literalsIn("const kind = 'white'; say('black')", 'x.ts')]).toEqual([])
    expect([...literalCounts('<style>.a { color: #fff; } .b { color: #fff; }</style>', 'X.svelte')]).toEqual([['#fff', 2]])
  })

  it('app.css keeps its colour values in the theme blocks only', () => {
    const outside = src('app.css')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/(^|\n)(:root, )?\[data-theme='[a-z]+'\](\[data-tint='[a-z]+'\])? \{[\s\S]*?\n\}/g, '')
    expect(outside.match(LITERAL) ?? []).toEqual([])
  })
})

describe("the picker wheel's slices (#158)", () => {
  // The one exception to "every colour is a token": a wheel's slices run through the neon spectrum (the owner's pick),
  // generated per wheel from its slice count, with each label's colour picked to read on its slice. They are content,
  // like skin tones, so a theme doesn't recolour them. lib/wheelColours.ts is the one place they are made; components
  // only pass its results on, so the literal scan above finds nothing in them.
  it('are made in lib/wheelColours.ts and nowhere else', () => {
    const made: string[] = []
    const walk = (dir: string) => {
      for (const e of readdirSync(dir, { withFileTypes: true })) {
        const f = join(dir, e.name)
        if (e.isDirectory()) walk(f)
        else if (/\.(ts|svelte)$/.test(e.name) && !/\.test\.ts$/.test(e.name) && /\bhsla?\(/.test(withoutComments(readFileSync(f, 'utf8')))) made.push(rel(f))
      }
    }
    walk(__dirname)
    expect(made).toEqual(['lib/wheelColours.ts'])
  })

  it('the wheel components hold no colour literal of their own', () => {
    for (const f of ['components/Wheel.svelte', 'components/WheelLine.svelte', 'components/WheelEditor.svelte', 'components/WheelPopover.svelte']) {
      expect([...literalsIn(src(f))], f).toEqual([])
    }
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

  // An SVG's fill or stroke in the accent family is a mark drawn like text (an icon, a glyph, a thin line), so it
  // uses the -text token too (#116), in a rule, a style: directive or an attribute. Shapes and areas keep the fill.
  const FILL_AS_FILL: Record<string, RegExp> = {
    'components/ProgressRing.svelte': /^\s*\.fill /, // the download ring: often over video (the clip icon), where the bright accent reads
    'components/SoundDialog.svelte': /^\s*\.(bar\.on|grip|sel) /, // the waveform's played bars, its handles and the selection: areas
  }
  it('accent, warn, ok and quote green as an SVG fill or stroke use their -text token', () => {
    const FILL_STROKE = /(?:(?:^|[\s;{])(?:fill|stroke):\s*|\b(?:fill|stroke)=["{]\s*["'`]?|\bstyle:(?:fill|stroke)=["{]\s*["'`]?)var\(--(?:accent|warn|ok|quote-green)\)/
    const offenders: string[] = []
    for (const f of [...svelteFiles, ...tsFiles, resolve(__dirname, 'app.css')]) {
      const allowed = FILL_AS_FILL[rel(f)]
      for (const line of readFileSync(f, 'utf8').split('\n')) {
        if (FILL_STROKE.test(line) && !(allowed && allowed.test(line))) offenders.push(`${rel(f)}: ${line.trim()}`)
      }
    }
    expect(offenders).toEqual([])
    // and the check sees each form
    for (const line of ['  .x { fill: var(--accent); }', '<path stroke="var(--ok)" />', "<circle style:fill={'var(--warn)'} />", '<rect fill={`var(--quote-green)`} />']) {
      expect(FILL_STROKE.test(line), line).toBe(true)
    }
    expect(FILL_STROKE.test('  .x { fill: var(--accent-text); }')).toBe(false)
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
