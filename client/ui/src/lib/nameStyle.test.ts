import { describe, expect, it } from 'vitest'
import type { User } from './protocol/User'
import { contrast, lightenForName, nameCss, NAME_GRADIENTS, NAME_SOLIDS, parseHex, resolveNameStyle } from './nameStyle'
import { NAME_LEVELS, surfacesFor, THEMES, TINTS } from './themes'

const user = (font: string | null, colors: string[]): User => ({ id: 'u1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font, colors } }) as User

describe('nameStyle', () => {
  it('lighten matches the shared vectors', () => {
    for (const [from, to] of [
      ['#000000', '#a8a8a8'],
      ['#0000ff', '#a1a1ff'],
      ['#ff0000', '#ff8585'],
      ['#123456', '#9baab8'],
      ['#7f7f7f', '#a8a8a8'],
      ['#2a9d8f', '#64b7ad'],
      ['#ffffff', '#ffffff'],
      // Either side of the 0.39 boundary.
      ['#a8a8a8', '#a8a8a8'],
      ['#a7a7a7', '#a8a8a8'],
      ['#40c040', '#40c040'],
      ['#3fbf3f', '#41c041'],
    ]) expect(lightenForName(from), from).toBe(to)
  })

  it('presets pass unchanged', () => {
    for (const c of [...NAME_SOLIDS.flatMap((s) => s.colors), ...NAME_GRADIENTS.flatMap((g) => g.colors)]) expect(lightenForName(c)).toBe(c)
    expect(NAME_SOLIDS.map((s) => s.colors[0])).toEqual(['#ffc4a3', '#d4c2ff', '#a8e6cf', '#a3d8ff'])
    expect(NAME_GRADIENTS.map((g) => g.colors)).toEqual([
      ['#ffbe7a', '#ff7fae'], ['#8fd0ff', '#c9a8ff'], ['#a2f7c2', '#8fd2ff'], ['#ffe58f', '#ff9d96'], ['#f6b6ff', '#9fb4ff'],
    ])
  })

  it('lightened names reach 4.5:1 on every app surface', () => {
    for (let r = 0; r <= 255; r += 0x33) for (let g = 0; g <= 255; g += 0x33) for (let b = 0; b <= 255; b += 0x33) {
      const c = '#' + [r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')
      expect(contrast(lightenForName(c), '#34363d'), c).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('role colour replaces colour and keeps the font', () => {
    const r = resolveNameStyle(user('fredoka', ['#ffbe7a', '#ff7fae']), { roleColor: '#000000', showStyles: true })
    expect(r.font?.key).toBe('fredoka')
    expect(r.paint).toEqual({ kind: 'solid', color: '#a8a8a8' })
  })

  it('show styles off: default font and colour, role colour kept', () => {
    const u = user('fredoka', ['#ffc4a3'])
    expect(resolveNameStyle(u, { roleColor: null, showStyles: false })).toEqual({ font: null, paint: { kind: 'plain' } })
    const r = resolveNameStyle(u, { roleColor: '#a3d8ff', showStyles: false })
    expect(r.font).toBeNull()
    expect(r.paint).toEqual({ kind: 'solid', color: '#a3d8ff' })
  })

  it('two colours make a gradient, one a solid, none plain', () => {
    const o = { roleColor: null, showStyles: true }
    expect(resolveNameStyle(user(null, ['#ffbe7a', '#ff7fae']), o).paint).toEqual({ kind: 'gradient', from: '#ffbe7a', to: '#ff7fae' })
    expect(resolveNameStyle(user(null, ['#ffc4a3']), o).paint).toEqual({ kind: 'solid', color: '#ffc4a3' })
    expect(resolveNameStyle(user(null, []), o).paint).toEqual({ kind: 'plain' })
    expect(resolveNameStyle(null, o)).toEqual({ font: null, paint: { kind: 'plain' } })
  })

  it('an unknown font key falls back to default', () => {
    expect(resolveNameStyle(user('comic-sans', []), { roleColor: null, showStyles: true }).font).toBeNull()
  })

  it('nameCss writes font and gradient', () => {
    const css = nameCss(resolveNameStyle(user('caveat', ['#ffbe7a', '#ff7fae']), { roleColor: null, showStyles: true }))
    expect(css).toContain('font-weight: 600')
    expect(css).toContain('--name-scale: 1.2')
    expect(css).toContain('background-image: linear-gradient(90deg, #ffbe7a, #ff7fae)')
    expect(css).toContain('color: transparent')
  })
})

describe('readable names (#42): moved to the chosen contrast on the surface they sit on', () => {
  const PRESETS = [...NAME_SOLIDS, ...NAME_GRADIENTS]
  // a stored name is already lightened by the server (luminance ≥ 0.39); role colours aren't
  const ODD = ['#ffffff', '#000000', '#5b5f6b', '#ff0000', '#ffe58f']
  const paintColors = (p: ReturnType<typeof resolveNameStyle>['paint']) => (p.kind === 'solid' ? [p.color] : p.kind === 'gradient' ? [p.from, p.to] : [])
  const VARIANTS = THEMES.flatMap((t) => (t.key === 'light' ? TINTS.map((n) => [t.key, n.key] as const) : [[t.key, 'plain'] as const]))

  it.each(VARIANTS)('%s (%s): every preset, role colour and odd colour, at every level, on the chat and the sidebar', (theme, tint) => {
    const s = surfacesFor(theme, tint)
    for (const bg of [s.bg1, s.bg2]) {
      for (const level of NAME_LEVELS) {
        const o = { roleColor: null, showStyles: true, bg, ratio: level.ratio }
        for (const p of PRESETS) {
          for (const c of paintColors(resolveNameStyle(user(null, p.colors.map(lightenForName)), o).paint)) expect(contrast(c, bg), `${p.label} ${c} on ${bg} @${level.key}`).toBeGreaterThanOrEqual(level.ratio)
        }
        for (const role of ODD) {
          const [c] = paintColors(resolveNameStyle(user(null, []), { ...o, roleColor: role }).paint)
          expect(contrast(c, bg), `role ${role} on ${bg} @${level.key}`).toBeGreaterThanOrEqual(level.ratio)
        }
      }
    }
  })

  it('a name that already reads is left exactly as stored', () => {
    const o = { roleColor: null, showStyles: true, bg: '#212228', ratio: 4.5 }
    expect(resolveNameStyle(user(null, ['#ffc4a3']), o).paint).toEqual({ kind: 'solid', color: '#ffc4a3' })
    expect(resolveNameStyle(user(null, ['#ffbe7a', '#ff7fae']), o).paint).toEqual({ kind: 'gradient', from: '#ffbe7a', to: '#ff7fae' })
  })

  it('a gradient moves each stop on its own (one may stay, the other move)', () => {
    const r = resolveNameStyle(user(null, ['#1a1a6a', '#ffe58f']), { roleColor: null, showStyles: true, bg: '#ffffff', ratio: 4.5 })
    expect(r.paint).toMatchObject({ kind: 'gradient', from: '#1a1a6a' })
    expect((r.paint as { to: string }).to).not.toBe('#ffe58f')
  })

  it('on a dark surface the contrast rule replaces "never darker than luminance 0.39": a dark role colour still lightens, just enough', () => {
    const [c] = paintColors(resolveNameStyle(null, { roleColor: '#000080', showStyles: true, bg: '#212228', ratio: 4.5 }).paint)
    expect(contrast(c, '#212228')).toBeGreaterThanOrEqual(4.5)
    expect(contrast(c, '#212228')).toBeLessThan(4.8)
  })

  it('keeps the hue when it moves a name', () => {
    const [c] = paintColors(resolveNameStyle(user(null, ['#ff7fae']), { roleColor: null, showStyles: true, bg: '#ffffff', ratio: 7 }).paint)
    const [r, g, b] = parseHex(c)!
    // pink stays pink: red the strongest channel, blue above green
    expect(r).toBeGreaterThan(b)
    expect(b).toBeGreaterThan(g)
  })
})
