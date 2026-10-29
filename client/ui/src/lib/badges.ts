// Badge keys the UI knows (plan reconciliation 17). Empty in this push.
import type { IconName } from '../components/Icon.svelte'

export const BADGES: Record<string, { name: string; icon: IconName }> = {}

/** An unknown key shows its key in title case with the trophy ("Badge" if the key has no words). */
export function badgeInfo(key: string): { name: string; icon: IconName } {
  return (
    BADGES[key] ?? {
      name: key.split(/[_\-\s]+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(' ') || 'Badge',
      icon: 'trophy',
    }
  )
}
