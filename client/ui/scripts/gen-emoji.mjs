// Builds src/lib/emoji-data.json (committed) from emojibase-data and copies the Twemoji SVGs the
// picker can show into public/emoji/ (gitignored). Entries without a Twemoji SVG are dropped, so
// the picker never offers an emoji it can't draw. Run by predev/prebuild.
import { copyFileSync, mkdirSync, readdirSync, rmSync, writeFileSync, statSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const svgDir = dirname(require.resolve('@twemoji/svg/package.json'))
const data = require('emojibase-data/en/data.json')
const shortcodes = require('emojibase-data/en/shortcodes/emojibase.json')
const groupMeta = require('emojibase-data/meta/groups.json').groups

const have = new Set(readdirSync(svgDir).filter((f) => f.endsWith('.svg')).map((f) => f.slice(0, -4)))

// Twemoji's file rule: lowercase code points (no leading zeros) joined by '-', FE0F dropped unless the sequence is a ZWJ one.
const fileFor = (hexcode) => {
  const cps = hexcode.toLowerCase().split('-').map((c) => c.replace(/^0+/, ''))
  return (cps.includes('200d') ? cps : cps.filter((c) => c !== 'fe0f')).join('-')
}

const LABELS = {
  'smileys-emotion': 'Smileys & Emotion',
  'people-body': 'People & Body',
  'animals-nature': 'Animals & Nature',
  'food-drink': 'Food & Drink',
  'travel-places': 'Travel & Places',
  activities: 'Activities',
  objects: 'Objects',
  symbols: 'Symbols',
  flags: 'Flags',
}
// Emojibase group 2 is skin-tone/hair components, not something to pick.
const groups = Object.entries(groupMeta)
  .filter(([, key]) => key in LABELS)
  .map(([id, key]) => ({ id: Number(id), key, label: LABELS[key] }))
const groupIndex = new Map(groups.map((g, i) => [g.id, i]))

// Emojibase's `emoji` string carries a stray FE0F after emoji-by-default characters (👍 + FE0F), but
// keyboards and other apps send them bare. Match those, so a stored reaction is the same string.
const norm = (str) => str.replace(/(\p{Emoji_Presentation})\uFE0F/gu, '$1')

const codes = (hexcode) => {
  const v = shortcodes[hexcode]
  return v === undefined ? [] : Array.isArray(v) ? v : [v]
}

const used = new Set()
const emoji = []
let dropped = 0
for (const e of data) {
  if (e.group === undefined || !groupIndex.has(e.group)) continue
  const file = fileFor(e.hexcode)
  if (!have.has(file)) {
    dropped++
    continue
  }
  used.add(file)
  // The first plain-word shortcode is the name; the rest (and the CLDR tags) are search keywords.
  const names = codes(e.hexcode)
  const n = names.find((c) => /^[a-z0-9_]+$/.test(c)) ?? e.label.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '')
  const k = [...new Set([...names.filter((c) => c !== n), ...(e.tags ?? [])])]
  const entry = { u: norm(e.emoji), n, k, g: groupIndex.get(e.group) }
  const tones = (e.skins ?? []).filter((s) => typeof s.tone === 'number')
  if (tones.length === 5 && tones.every((s) => have.has(fileFor(s.hexcode)))) {
    entry.s = tones.map((s) => norm(s.emoji))
    for (const s of tones) used.add(fileFor(s.hexcode))
  }
  emoji.push(entry)
}

writeFileSync(
  join(root, 'src/lib/emoji-data.json'),
  JSON.stringify({ groups: groups.map(({ key, label }) => ({ key, label })), emoji }),
)

const out = join(root, 'public/emoji')
rmSync(out, { recursive: true, force: true })
mkdirSync(out, { recursive: true })
for (const f of used) copyFileSync(join(svgDir, `${f}.svg`), join(out, `${f}.svg`))

const size = statSync(join(root, 'src/lib/emoji-data.json')).size
console.log(`emoji: ${emoji.length} kept, ${dropped} dropped (no Twemoji SVG), ${used.size} SVGs copied, data ${(size / 1024).toFixed(0)} KB`)
if (size > 600 * 1024) {
  console.error('emoji-data.json is over the 600 KB budget')
  process.exit(1)
}
