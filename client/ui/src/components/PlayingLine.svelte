<script lang="ts">
  // "Playing <game>" in one line (#392), where a status shows (member lists, a DM's header), as ListeningLine does for
  // a song. The profile card and voice tiles have the art.
  import { playingLine } from '../lib/game'
  import type { Playing } from '../lib/protocol/Playing'

  let { playing }: { playing: Playing } = $props()
  const line = $derived(playingLine(playing))
</script>

<span class="playing-line" data-playing-line data-tip={line}><span class="text">{line}</span></span>

<style>
  .playing-line { display: inline-flex; align-items: center; min-width: 0; max-width: 100%; color: var(--text-3); font-size: 12px; font-weight: 400; }
  .text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
