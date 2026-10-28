// "Show name styles" (plan reconciliation 12): per device, like "Animate emoji".
const KEY = 'pulse.showNameStyles'

function saved(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'false'
  } catch {
    return true
  }
}

export const appearance = $state({ showNameStyles: saved() })

export function setShowNameStyles(on: boolean): void {
  appearance.showNameStyles = on
  try {
    localStorage.setItem(KEY, String(on))
  } catch {
    /* ignore */
  }
}
