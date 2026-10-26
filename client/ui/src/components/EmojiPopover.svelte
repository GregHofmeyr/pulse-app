<script lang="ts">
  // The emoji picker as a popover next to the button that opened it: the composer's, a message's
  // and a reaction row's all open it the same way.
  import { untrack } from 'svelte'
  import EmojiPicker from './EmojiPicker.svelte'
  import { clickOutside } from '../lib/actions'
  import type { Std } from '../lib/emoji'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { memberServers, popoverStyle } from '../lib/reactions'
  import { app } from '../lib/store.svelte'

  let { anchor, onPick, onClose }: {
    /** The button that opened it; give it `data-menu-toggle` so a press on it toggles instead. */
    anchor: HTMLElement
    onPick: (e: Std | CustomEmoji, opts: { keep: boolean }) => void
    onClose: () => void
  } = $props()

  const SIZE = { width: 360, height: 420 }
  // Placed once on open: it stays put while the picker is in use.
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))
</script>

<div class="emoji-popover" use:clickOutside={onClose} {style}>
  <EmojiPicker {onPick} {onClose} servers={app.state.servers} memberOf={memberServers(app.state)} />
</div>
