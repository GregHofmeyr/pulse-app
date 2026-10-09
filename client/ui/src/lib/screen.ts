import type { ConnState } from './state'

export type Screen = 'boot' | 'login' | 'update' | 'shell'

/** Which top-level screen the app shows. */
export function screenFor(v: { booting: boolean; signedIn: boolean; conn: ConnState }): Screen {
  if (v.booting) return 'boot'
  if (!v.signedIn) return 'login'
  return v.conn === 'update_required' ? 'update' : 'shell'
}
