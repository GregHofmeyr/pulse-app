/** Rejects with `msg` if `p` hasn't settled within `ms`: a runner that waits forever would log nothing. */
export function withDeadline<T>(p: Promise<T>, ms: number, msg: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(msg)), ms)
  })
  return Promise.race([p, late]).finally(() => clearTimeout(timer))
}
