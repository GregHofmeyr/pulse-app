// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { describe, expect, it } from 'vitest'
import Icon from './Icon.svelte'

const pathOf = (name: string) => {
  const c = mount(Icon, { target: document.body, props: { name: name as never } })
  flushSync()
  const d = document.querySelector('path')!.getAttribute('d')!
  unmount(c)
  document.body.innerHTML = ''
  return d
}

describe('the soundboard icon (#221)', () => {
  it('reads as sound bursting out: not the old equaliser bars, and not the voice speaker or the volume', () => {
    const d = pathOf('soundboard')
    expect(d).not.toBe('M4 10v4M8 6v12M12 3v18M16 7v10M20 10v4')
    expect(d).not.toBe(pathOf('speaker'))
    expect(d).not.toBe(pathOf('volume'))
    // a horn and three straight lines bursting from its mouth, in the set's 24-unit grid
    expect(d.split('M').filter(Boolean).length).toBeGreaterThanOrEqual(4)
  })
})
