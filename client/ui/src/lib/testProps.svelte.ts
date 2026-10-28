// Tests only: props a test can change after mounting (e.g. a banner moving to another server).
export function reactiveProps<T extends Record<string, unknown>>(props: T): T {
  const p = $state(props)
  return p
}
