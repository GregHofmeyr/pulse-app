// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { chipBroken, offsetOf, positionAt, renderInto, serialize, tokenSpans } from './composerDom'

const pog = { id: '1', name: 'pog', hash: 'a'.repeat(64), still_hash: null, animated: false }
const party = { id: '2', name: 'party', hash: 'b'.repeat(64), still_hash: 'c'.repeat(64), animated: true }
const lookup = (id: string) => ({ '1': pog, '2': party })[id] as never
const box = () => {
  const el = document.createElement('div')
  el.contentEditable = 'true'
  document.body.replaceChildren(el)
  return el
}

describe('composer DOM', () => {
  it('finds the custom emoji tokens in the text', () => {
    expect(tokenSpans('a <:pog:1> b <a:party:2>')).toEqual([
      { start: 2, end: 10, id: '1' },
      { start: 13, end: 24, id: '2' },
    ])
    expect(tokenSpans('no tokens: <pog:1> <:x:1>')).toEqual([]) // too short a name, not a token
  })

  it('renders known tokens as images and round-trips the exact text', () => {
    const el = box()
    const text = 'hi <:pog:1> and <a:party:2>\nline two <:ghost:9> 😀'
    renderInto(el, text, lookup)
    const imgs = el.querySelectorAll('img')
    expect(imgs).toHaveLength(2)
    expect(imgs[0].alt).toBe(':pog:')
    expect(imgs[0].dataset.token).toBe('<:pog:1>')
    expect(imgs[0].getAttribute('contenteditable')).toBe('false')
    expect(imgs[1].dataset.token).toBe('<a:party:2>')
    // an emoji this client doesn't know stays as its text
    expect(el.textContent).toContain('<:ghost:9>')
    expect(serialize(el)).toBe(text)
  })

  it('a trailing newline gets a placeholder line break that is not part of the text', () => {
    const el = box()
    renderInto(el, 'a\n', lookup)
    expect(el.lastChild!.nodeName).toBe('BR')
    expect(serialize(el)).toBe('a\n')
    renderInto(el, '', lookup)
    expect(el.childNodes).toHaveLength(0)
    expect(serialize(el)).toBe('')
  })

  it('reads what the browser leaves after editing: wrapper divs, brs, stray spans', () => {
    const el = box()
    el.innerHTML = 'one<div>two <img data-token="&lt;:pog:1&gt;"></div><div><br></div><div>four<br></div><span>five</span>'
    expect(serialize(el)).toBe('one\ntwo <:pog:1>\n\nfour\nfive')
  })

  it('maps caret positions both ways, counting a token as its whole text', () => {
    const el = box()
    renderInto(el, 'ab <:pog:1> cd', lookup)
    const [t1, img, t2] = [...el.childNodes]
    expect(img.nodeName).toBe('IMG')
    expect(offsetOf(el, t1, 2)).toBe(2)
    expect(offsetOf(el, el, 2)).toBe('ab '.length + '<:pog:1>'.length) // after the image
    expect(offsetOf(el, t2, 1)).toBe('ab <:pog:1> '.length)
    expect(positionAt(el, 2)).toEqual({ node: t1, offset: 2 })
    // at or inside a token: right after its image
    expect(positionAt(el, 3)).toEqual({ node: t1, offset: 3 })
    expect(positionAt(el, 6)).toEqual({ node: el, offset: 2 })
    expect(positionAt(el, 11)).toEqual({ node: t2, offset: 0 }) // the same spot, in the text typing goes into
    expect(positionAt(el, 13)).toEqual({ node: t2, offset: 2 })
    expect(positionAt(el, 99)).toEqual({ node: t2, offset: 3 }) // past the end: the end
  })

  it('a leading emoji: offset 0 is before its image', () => {
    const el = box()
    renderInto(el, '<:pog:1> x', lookup)
    expect(positionAt(el, 0)).toEqual({ node: el, offset: 0 })
    expect(positionAt(el, 8)).toEqual({ node: el.childNodes[1], offset: 0 })
  })

  it('an offset never runs past the end of the text', () => {
    const el = box()
    renderInto(el, 'a\n', lookup)
    expect(offsetOf(el, el, el.childNodes.length)).toBe(2)
    el.innerHTML = 'a<br>'
    expect(offsetOf(el, el, 2)).toBe(1)
  })

  it('a no-break space reads as a plain space (pastes and autocorrect bring them)', () => {
    const el = box()
    el.innerHTML = 'a&nbsp;b'
    expect(serialize(el)).toBe('a b')
  })

  it('only a real media hash becomes an image, and only a real animation is marked animated', () => {
    const el = box()
    const odd = { id: '3', name: 'odd', hash: '../x', still_hash: null, animated: false }
    const same = { id: '4', name: 'same', hash: 'd'.repeat(64), still_hash: 'd'.repeat(64), animated: true }
    renderInto(el, '<:odd:3> <a:same:4>', (id) => ({ '3': odd, '4': same })[id] as never)
    const imgs = el.querySelectorAll('img')
    expect(imgs).toHaveLength(1)
    expect(imgs[0].dataset.anim).toBeUndefined()
  })
})

// #47/#48: a mention is written as `@username` (what the server reads) but the box shows the name
// that applies here, as one chip, never the username.
describe('mention chips', () => {
  const shown = (u: string) => ({ sam: 'Big Dog', kim: 'Kim 🌸' } as Record<string, string>)[u.toLowerCase()] ?? null
  it('shows a known @username as a chip with the shown name, and serializes back to the username', () => {
    const el = box()
    const text = 'hey @sam and @Kim, not @nobody or a@sam.com <:pog:1>'
    renderInto(el, text, lookup, shown)
    const chips = el.querySelectorAll<HTMLElement>('.mention-chip')
    expect([...chips].map((c) => c.textContent)).toEqual(['@Big Dog', '@Kim 🌸'])
    expect(chips[0].dataset.token).toBe('@sam')
    expect(chips[1].dataset.token).toBe('@Kim')
    expect(chips[0].getAttribute('contenteditable')).toBe('false')
    expect(el.textContent).not.toContain('@sam ')
    expect(el.textContent).toContain('@nobody')
    expect(serialize(el)).toBe(text)
  })

  it('leaves mentions inside inline code as text', () => {
    const el = box()
    renderInto(el, 'run `@sam` now @sam', lookup, shown)
    expect(el.querySelectorAll('.mention-chip')).toHaveLength(1)
    expect(serialize(el)).toBe('run `@sam` now @sam')
  })

  it('a trailing dot is not part of the name', () => {
    const el = box()
    renderInto(el, 'thanks @sam.', lookup, shown)
    expect(el.querySelector<HTMLElement>('.mention-chip')!.dataset.token).toBe('@sam')
    expect(serialize(el)).toBe('thanks @sam.')
  })

  it('a chip with a name character typed right after it no longer says what is sent', () => {
    const el = box()
    renderInto(el, 'hi @sam ok', lookup, shown)
    expect(chipBroken(el)).toBe(false)
    ;(el.lastChild as Text).data = 'antha'
    expect(serialize(el)).toBe('hi @samantha')
    expect(chipBroken(el)).toBe(true)
    ;(el.lastChild as Text).data = '. and'
    expect(chipBroken(el)).toBe(false)
    ;(el.lastChild as Text).data = '.x'
    expect(chipBroken(el)).toBe(true)
  })

  it('caret offsets count the username a chip stands for', () => {
    const el = box()
    renderInto(el, 'a @sam b', lookup, shown)
    const chip = el.querySelector('.mention-chip')!
    // inside the username snaps to just after the chip ('a @sam' is 6)
    const inside = positionAt(el, 4)
    expect(inside.node).toBe(el)
    expect(el.childNodes[inside.offset - 1]).toBe(chip)
    expect(offsetOf(el, inside.node, inside.offset)).toBe(6)
    const after = positionAt(el, 7)
    expect(offsetOf(el, after.node, after.offset)).toBe(7)
  })
})
