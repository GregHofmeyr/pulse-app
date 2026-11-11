<script lang="ts">
  // The GIF picker (#58) as a popover next to the composer's GIF button, placed as the emoji picker's is.
  import { untrack } from 'svelte'
  import GifPicker from './GifPicker.svelte'
  import { clickOutside } from '../lib/actions'
  import { popoverStyle } from '../lib/reactions'
  import type { Gif } from '../lib/protocol/Gif'

  let { anchor, onPick, onClose }: {
    /** The button that opened it; give it `data-menu-toggle` so a press on it toggles instead. */
    anchor: HTMLElement
    onPick: (gif: Gif, link: string) => void
    onClose: () => void
  } = $props()

  const SIZE = { width: 360, height: 420 }
  // Placed once on open: it stays put while the picker is in use.
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))
</script>

<div class="emoji-popover pop-in" use:clickOutside={onClose} {style}>
  <GifPicker {onPick} {onClose} />
</div>
