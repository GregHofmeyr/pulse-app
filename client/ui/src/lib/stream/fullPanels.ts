// Full view's side panels: the shell owns them (it hides them while a full view is on screen), and
// the full view's top bar has the buttons that bring each back. The shell hands this down as context.
import { getContext, setContext } from 'svelte'

export type PanelSide = 'left' | 'right'

export interface FullPanels {
  /** Shown over the full view (the panel's toggle is pressed). */
  readonly left: boolean
  readonly right: boolean
  /** What the left panel holds: 'Channels' on a server, 'Conversations' at Home. */
  readonly leftName: string
  /** There's a member list to show (a server, or a group conversation). */
  readonly hasRight: boolean
  toggle(side: PanelSide): void
}

/** The ids of the toggles, and of what they control (the shell moves focus to a toggle as its panel hides). */
export const PANEL_TOGGLE_ID = { left: 'full-toggle-channels', right: 'full-toggle-members' } as const
export const PANEL_ID = { left: 'channels-panel', right: 'members-panel' } as const

const KEY = Symbol('fullPanels')
export const setFullPanels = (p: FullPanels) => setContext(KEY, p)
/** Undefined outside the shell (a full view mounted on its own has no panels to toggle). */
export const getFullPanels = () => getContext<FullPanels | undefined>(KEY)
/** For `mount(…, { context })`: what the shell would set. */
export const fullPanelsContext = (p: FullPanels) => new Map<symbol, FullPanels>([[KEY, p]])
