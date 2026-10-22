/** Rejects with `msg` if `p` hasn't settled within `ms`: a runner that waits forever would log nothing. */
export function withDeadline<T>(p: Promise<T>, ms: number, msg: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(msg)), ms)
  })
  return Promise.race([p, late]).finally(() => clearTimeout(timer))
}

type FrameSource = { requestVideoFrameCallback(cb: (now: number, meta: VideoFrameCallbackMetadata) => void): number }

/** Calls `onFrame` for every painted frame until the returned stop function runs. */
export function frameLoop(el: FrameSource, onFrame: (now: number, meta: VideoFrameCallbackMetadata) => void): () => void {
  let live = true
  const tick = (now: number, meta: VideoFrameCallbackMetadata) => {
    if (!live) return
    onFrame(now, meta)
    el.requestVideoFrameCallback(tick)
  }
  el.requestVideoFrameCallback(tick)
  return () => {
    live = false
  }
}
