// The Quote of the Year voting window's open state, shared by the banner and the voice strip.
import type { QotyEvent } from './protocol/QotyEvent'

/** Whether the voting window is open. Call during a component's setup. The window closes for good
 *  when the vote stops being live or another server's vote shows: reopened (the same event) or
 *  switched to, it waits to be asked for again. A new copy of the same live vote leaves it open. */
export function qotyWindow(serverId: () => string, live: () => QotyEvent | null) {
  let open = $state(false)
  const shown = $derived.by(() => {
    const e = live()
    return e ? `${serverId()}/${e.id}` : null
  })
  $effect.pre(() => {
    void shown
    open = false
  })
  return {
    get open() {
      return open
    },
    show: () => (open = true),
    close: () => (open = false),
  }
}
