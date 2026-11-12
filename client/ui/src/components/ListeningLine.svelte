<script lang="ts">
  // "Listening to <song>" in one line (#275), where a status shows (member lists, a DM's header). Spotify's
  // single-colour icon attributes it (#196): beside the words, or (`icon={false}`, a member row too short for it
  // under the name) drawn by the row at its end. The profile card has the art and the link-out.
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { activityLine } from '../lib/activity'
  import type { Listening } from '../lib/protocol/Listening'

  let { listening, icon = true }: { listening: Listening; icon?: boolean } = $props()
  const line = $derived(activityLine(listening))
</script>

<span class="listening-line" data-tip={line}>{#if icon}<SpotifyIcon />{/if}<span class="text">{line}</span></span>

<style>
  .listening-line { display: inline-flex; align-items: center; gap: 6px; min-width: 0; max-width: 100%; color: var(--text-3); font-size: 12px; font-weight: 400; }
  .text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
