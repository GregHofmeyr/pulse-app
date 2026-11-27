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

  it('rows_holding_names_dont_pin_height', () => {
    const rows: [string, string[]][] = [
      ['ChannelList.svelte', ['.row', '.occupant']],
      ['MemberList.svelte', ['.row']],
      ['ConversationList.svelte', ['.row', '.name']],
      ['UserTile.svelte', ['.name', '.text']],
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
