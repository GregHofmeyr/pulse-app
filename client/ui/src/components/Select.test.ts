// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import Select from './Select.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const options = [{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }]

describe('Select', () => {
  it('opens_downward_by_default', () => {
    cmp = mount(Select, { target: document.body, props: { value: 'a', options, label: 'Pick' } })
    flushSync()
    expect(document.querySelector('.select')?.getAttribute('data-drop')).toBe('down')
  })
  it('drop_up_is_marked_for_the_upward_list', () => {
    cmp = mount(Select, { target: document.body, props: { value: 'a', options, label: 'Pick', drop: 'up' } })
    flushSync()
    expect(document.querySelector('.select')?.getAttribute('data-drop')).toBe('up')
  })
})
