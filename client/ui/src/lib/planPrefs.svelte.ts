// The per-device "Plan something" setting (#387), on its own so the alert path can read it without the plan module.
const SETTING_KEY = 'pulse.planSomething'

function savedOn(): boolean {
  try {
    return localStorage.getItem(SETTING_KEY) !== 'off'
  } catch {
    return true
  }
}

/** "Plan something" (this device only; on by default). Off: no Plan button or `/plan`, no forecasts on the cards and
 *  no notification for a plan's reminder (it still lands in the inbox). Plans others post still show. */
export const planPrefs = $state({ on: savedOn() })

export function setPlanSomething(v: boolean): void {
  planPrefs.on = v
  try {
    localStorage.setItem(SETTING_KEY, v ? 'on' : 'off')
  } catch {
    /* ignore */
  }
}
