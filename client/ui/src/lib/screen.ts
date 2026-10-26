import type { ConnState } from './state'

export type Screen = 'boot' | 'login' | 'update' | 'waiting' | 'shell'

/** Which top-level screen the app shows. */
export function screenFor(v: { booting: boolean; signedIn: boolean; conn: ConnState; restoreRefused?: boolean }): Screen {
  if (v.booting) return 'boot'
  // startup was turned away while signed in: wait it out, the app opens on Ready
  if (!v.signedIn) return v.restoreRefused && v.conn !== 'logged_out' ? 'waiting' : 'login'
  return v.conn === 'update_required' ? 'update' : 'shell'
}
