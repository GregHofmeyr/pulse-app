// Styled names never clip (#727): every Name gets vertical room inside its own clipping box, tall fonts get more,
// and the rows that hold a name grow (min-height) instead of pinning a height that cuts it.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { NAME_FONTS } from '../lib/nameFonts'
import { nameCss, resolveNameStyle } from '../lib/nameStyle'
import type { User } from '../lib/protocol/User'

const style = (file: string) => readFileSync(resolve(__dirname, file), 'utf8').match(/<style[^>]*>([\s\S]*?)<\/style>/)![1]
const rules = (css: string) => [...css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{([^}]*)\}/g)].map((m) => ({ sels: m[1].split(',').map((s) => s.trim()), body: m[2] }))
const body = (css: string, sel: string) => rules(css).find((r) => r.sels.includes(sel))?.body ?? ''

describe('Name CSS', () => {
  const css = style('Name.svelte')

  it('every_name_gets_vertical_room', () => {
    const b = body(css, '.name')
    const m = b.match(/padding-block:\s*var\(--name-pad,\s*([\d.]+)em\)/)
    expect(m, '.name padding-block').not.toBeNull()
    expect(Number(m![1])).toBeGreaterThanOrEqual(0.12)
    expect(b).toMatch(/overflow:\s*clip/)
    expect(b).toMatch(/overflow-clip-margin:/)
    expect(body(css, ".name[data-fx]")).not.toMatch(/overflow/)
  })

  it('interactive_names_keep_their_vertical_room', () => {
    const b = body(css, 'button.name')
    expect(b).not.toMatch(/(^|[;\s])padding:/)
    expect(b).not.toMatch(/(^|[;\s])margin:/)
    expect(b).not.toMatch(/padding-block/)
    expect(b).not.toMatch(/margin-block/)
    expect(b).toMatch(/padding-inline:\s*0/)
    expect(b).toMatch(/margin-inline:\s*0/)
  })

  it('tall_fonts_get_more_room', () => {
    const tall = NAME_FONTS.filter((f) => f.tall)
    expect(tall.map((f) => f.key)).toEqual(expect.arrayContaining(['pacifico', 'kaushan-script', 'caveat', 'cormorant-garamond']))
    for (const f of tall) {
      const u = { id: 'u', style: { font: f.key, colors: [], effect: null } } as unknown as User
      const pad = nameCss(resolveNameStyle(u, { roleColor: null, showStyles: true }), true).match(/--name-pad:\s*([\d.]+)em/)
      expect(pad, f.key).not.toBeNull()
      expect(Number(pad![1])).toBeGreaterThanOrEqual(0.2)
    }
  })

  it('inline_names_dont_grow_the_line', () => {
    expect(body(css, '.name.flow')).toMatch(/margin-block:\s*calc\(-1 \* var\(--name-pad, \.14em\)\)/)
    // every Name mid-sentence ("X is spinning ...", "X spun ..."); the other sites are labels on a row of their own
    // (message meta, reply, quote speaker, clip person zone, composer option) and keep their padding
    for (const f of ['WheelLine.svelte', 'QuietSpinCard.svelte']) {
      const names = readFileSync(resolve(__dirname, f), 'utf8').match(/<Name [^>]*>/g) ?? []
      expect(names.length, f).toBeGreaterThan(0)
      for (const n of names) expect(n, f).toMatch(/\bflow\b/)
    }
  })

  it('a font keeps its own scale', () => {
    const u = { id: 'u', style: { font: 'press-start-2p', colors: [], effect: null } } as unknown as User
    const out = nameCss(resolveNameStyle(u, { roleColor: null, showStyles: true }), true)
    expect(out).toContain('--name-scale: 0.68')
    expect(out).not.toContain('--name-scale: 1')
  })

  it('rows_holding_names_dont_pin_height', () => {
    const rows: [string, string[]][] = [
      ['ChannelList.svelte', ['.row', '.occupant']],
      ['MemberList.svelte', ['.row']],
      ['ConversationList.svelte', ['.row', '.name']],
      ['UserTile.svelte', ['.name', '.text']],
      ['CallBand.svelte', ['.people', '.who']],
    ]
    for (const [file, sels] of rows) {
      const c = style(file)
      for (const sel of sels) {
        const b = body(c, sel)
        const pinned = /(^|[;\s])height:/.test(b) && /overflow:\s*hidden/.test(b)
        expect(pinned, `${file} ${sel}`).toBe(false)
        if (sel === '.row' || sel === '.occupant') expect(b, `${file} ${sel}`).not.toMatch(/(^|[;\s])height:/)
      }
    }
  })
})
