import { getContext, setContext } from 'svelte'

// --- what keeps the open chat from being trimmed (#561): a message being edited, or with its menu or a picker open,
// holds its list as it is. TextChannel provides it; elsewhere (search results, pins, a replies panel) nothing holds. ---
const TRIM_HOLD = Symbol('trimHold')
type Hold = (key: symbol, on: boolean) => void
/** For TextChannel, as it starts: `hold(key, on)` marks (or unmarks) one message's hold by a key of its own. */
export const provideTrimHold = (hold: Hold): void => void setContext(TRIM_HOLD, hold)
/** For a message, as it starts: its chat's hold (a no-op outside one). */
export const trimHold = (): Hold => getContext<Hold | undefined>(TRIM_HOLD) ?? (() => {})
