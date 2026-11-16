// @vitest-environment jsdom
import { createRawSnippet, flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const uiLog = vi.fn()
vi.mock('../lib/tauri', () => ({ api: { uiLog: (...a: unknown[]) => uiLog(...a) } }))

import ShellBoundary from './ShellBoundary.svelte'

// #132: a throw while showing the Shell (a sign-out teardown left the window half-dead until a restart) shows
// "Something went wrong" with a Reload that mounts it afresh, and the error reaches the log.
describe('ShellBoundary', () => {
  let cmp: ReturnType<typeof mount> | undefined
  afterEach(() => {
    if (cmp) unmount(cmp)
    cmp = undefined
    uiLog.mockClear()
    document.body.innerHTML = ''
  })

  it('shows the fallback for a child that throws, logs it, and Reload recovers', () => {
    let broken = true
    const children = createRawSnippet(() => ({
      render: () => {
        if (broken) throw new TypeError('activeChannel is undefined')
        return '<p class="shell-ok">Shell</p>'
      },
    }))
    cmp = mount(ShellBoundary, { target: document.body, props: { children } })
    flushSync()

    const alert = document.querySelector('[role="alert"]')
    expect(alert?.querySelector('h1')?.textContent).toBe('Something went wrong')
    expect(alert?.textContent).toContain('Pulse hit an error showing this screen.')
    expect(document.querySelector('.shell-ok')).toBeNull()
    expect(uiLog).toHaveBeenCalledTimes(1)
    expect(uiLog.mock.calls[0][0]).toMatch(/^boundary shell: TypeError: activeChannel is undefined/)
    expect(uiLog.mock.calls[0][1]).toBe('warn')

    broken = false
    const reload = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Reload')!
    reload.click()
    flushSync()
    expect(document.querySelector('.shell-ok')?.textContent).toBe('Shell')
    expect(document.querySelector('[role="alert"]')).toBeNull()
  })

  it('shows the Shell untouched when nothing throws', () => {
    const children = createRawSnippet(() => ({ render: () => '<p class="shell-ok">Shell</p>' }))
    cmp = mount(ShellBoundary, { target: document.body, props: { children } })
    flushSync()
    expect(document.querySelector('.shell-ok')).not.toBeNull()
    expect(uiLog).not.toHaveBeenCalled()
  })
})
