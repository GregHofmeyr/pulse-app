// One update per animation frame from a control that fires faster than the screen draws: a native colour picker sends
// `input` many times a frame while you drag, and each one restyled the profile preview or the whole app (the accent)
// (#156 follow-up). The latest value wins; flush applies a pending one at once (on `change`, the picker closing).

export type PerFrame<T> = { push(v: T): void; flush(): void; cancel(): void }

export function perFrame<T>(apply: (v: T) => void): PerFrame<T> {
  let pending: { v: T } | null = null
  let frame: number | null = null
  const run = () => {
    frame = null
    const p = pending
    pending = null
    if (p) apply(p.v)
  }
  return {
    push(v) {
      pending = { v }
      if (frame === null) frame = requestAnimationFrame(run)
    },
    flush() {
      if (frame !== null) cancelAnimationFrame(frame)
      run()
    },
    cancel() {
      if (frame !== null) cancelAnimationFrame(frame)
      frame = null
      pending = null
    },
  }
}
