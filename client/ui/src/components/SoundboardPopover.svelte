<script lang="ts">
  // The soundboard as a popover above the call-controls button that opened it.
  import { untrack } from 'svelte'
  import Soundboard from './Soundboard.svelte'
  import { clickOutside } from '../lib/actions'
  import type { Sound } from '../lib/protocol/Sound'
  import { memberServers, popoverStyle } from '../lib/reactions'
  import { app } from '../lib/store.svelte'

  let { anchor, onClose, onAdd = () => {}, onEdit = () => {}, onTidy = () => {} }: {
    /** The button that opened it; give it `data-menu-toggle` so a press on it toggles instead. */
    anchor: HTMLElement
    onClose: () => void
    /** Hook for Task 9's add-sound dialog. */
    onAdd?: (serverId: string) => void
    /** Hook for Task 9's edit dialog. */
    onEdit?: (sound: Sound) => void
    /** Hook for Task 9's tidy-up list. */
    onTidy?: (serverId: string) => void
  } = $props()

  const SIZE = { width: 440, height: 460 }
  // Placed once on open. `popoverStyle` prefers below the button and flips above when it won't fit,
  // which is where the call controls (at the bottom of the window) put it.
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))
</script>

<div class="soundboard-popover pop-in" use:clickOutside={onClose} {style}>
  <Soundboard {onClose} {onAdd} {onEdit} {onTidy} servers={app.state.servers} memberOf={memberServers(app.state)} />
</div>
