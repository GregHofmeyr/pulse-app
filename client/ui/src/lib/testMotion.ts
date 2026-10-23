// Test doubles for what jsdom lacks: IntersectionObserver and matchMedia. Not used by the app.

export class FakeIntersectionObserver {
  static all: FakeIntersectionObserver[] = []
  readonly root = null
  readonly rootMargin = ''
  readonly thresholds = []
  targets = new Set<Element>()
  constructor(private cb: (entries: { target: Element; isIntersecting: boolean }[]) => void) {
    FakeIntersectionObserver.all.push(this)
  }
  observe(el: Element) { this.targets.add(el) }
  unobserve(el: Element) { this.targets.delete(el) }
  disconnect() { this.targets.clear() }
  takeRecords() { return [] }
  /** Reports `el` entering or leaving the viewport to every observer watching it. */
  static set(el: Element, isIntersecting: boolean) {
    for (const io of FakeIntersectionObserver.all) if (io.targets.has(el)) io.cb([{ target: el, isIntersecting }])
  }
  static watching() {
    return FakeIntersectionObserver.all.reduce((n, io) => n + io.targets.size, 0)
  }
}

/** A `matchMedia` whose one query can be flipped, firing `change` like the real one. */
export function fakeMatchMedia(matches = false) {
  const listeners = new Set<(e: { matches: boolean }) => void>()
  const mql = {
    matches,
    media: '',
    addEventListener: (_: 'change', fn: (e: { matches: boolean }) => void) => listeners.add(fn),
    removeEventListener: (_: 'change', fn: (e: { matches: boolean }) => void) => listeners.delete(fn),
  }
  return {
    matchMedia: (q: string) => Object.assign(mql, { media: q }),
    set(m: boolean) {
      mql.matches = m
      for (const fn of listeners) fn({ matches: m })
    },
  }
}
