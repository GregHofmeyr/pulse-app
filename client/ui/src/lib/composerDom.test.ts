// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { offsetOf, positionAt, renderInto, serialize, tokenSpans } from './composerDom'

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
})
