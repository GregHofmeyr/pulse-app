// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'

const uiLog = vi.fn()
vi.mock('./tauri', () => ({ api: { uiLog: (...a: unknown[]) => uiLog(...a) } }))

import { logUncaughtErrors } from './uiErrors'

// A page error that isn't caught (the blank window after a sign-in, 2026-11-03, left nothing in the log): it goes to
// the app's log file, so the next one can be read there.
describe('logUncaughtErrors', () => {
  let stop: () => void = () => {}
  afterEach(() => {
    stop()
    uiLog.mockClear()
  })

  it('writes an uncaught error with the top of its stack', () => {
    stop = logUncaughtErrors()
    const err = new TypeError("undefined is not an object (evaluating 'z(je).id')")
    err.stack = 'TypeError: …\n    at En (index.js:1:18252)\n    at Tn (index.js:1:18059)\n    at a\n    at b\n    at c\n    at d\n    at e'
    window.dispatchEvent(new ErrorEvent('error', { error: err, message: err.message }))
    expect(uiLog).toHaveBeenCalledTimes(1)
    const [line, level] = uiLog.mock.calls[0]
    expect(level).toBe('warn')
    expect(line).toContain("uncaught: TypeError: undefined is not an object (evaluating 'z(je).id')")
    expect(line).toContain('En (index.js:1:18252)')
    expect(line.split('|').length).toBeLessThanOrEqual(7) // the top few frames, one line
  })

  it('writes an unhandled promise rejection', () => {
    stop = logUncaughtErrors()
    const ev = new Event('unhandledrejection') as Event & { reason: unknown }
    ev.reason = new Error('nope')
    window.dispatchEvent(ev)
    expect(uiLog.mock.calls[0][0]).toContain('unhandled rejection: Error: nope')
  })

  it('stops when asked', () => {
    logUncaughtErrors()()
    window.dispatchEvent(new ErrorEvent('error', { error: new Error('x'), message: 'x' }))
    expect(uiLog).not.toHaveBeenCalled()
  })
})
