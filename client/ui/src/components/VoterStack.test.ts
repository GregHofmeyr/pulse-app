// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import VoterStack from './VoterStack.svelte'

let cmp: ReturnType<typeof mount> | undefined
const names: Record<string, string> = { U1: 'alex', U2: 'sam', U3: 'kim', U4: 'lee', U5: 'ray', U6: 'mia', U7: 'zed' }
const show = (userIds: string[], max?: number) => {
  cmp = mount(VoterStack, { target: document.body, props: { userIds, nameOf: (id: string | null) => names[id ?? ''] ?? '?', ...(max ? { max } : {}) } })
  flushSync()
}
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('VoterStack', () => {
  it('shows up to 5 then +N and names every voter on hover', () => {
    show(['U1', 'U2', 'U3', 'U4', 'U5', 'U6', 'U7'])
    expect(document.querySelectorAll('.av').length).toBe(5)
    expect(document.querySelector('.more')?.textContent).toBe('+2')
    const tip = document.querySelector('.stack')!.getAttribute('data-tip')
    expect(tip).toBe('alex, sam, kim, lee, ray, mia and zed')
  })

  it('shows no +N when everyone fits', () => {
    show(['U1', 'U2'])
    expect(document.querySelectorAll('.av').length).toBe(2)
    expect(document.querySelector('.more')).toBeNull()
  })

  it('renders nothing for no voters', () => {
    show([])
    expect(document.body.innerHTML.replace(/<!--.*?-->/g, '').trim()).toBe('')
  })
})
