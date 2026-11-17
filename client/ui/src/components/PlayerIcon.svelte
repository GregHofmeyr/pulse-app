<script lang="ts">
  // Which player a "Listening to" is from (#322): Spotify's own single-colour icon (#196), or for any other player a
  // generic music icon carrying the player's name (no other brand's logo).
  import Icon from './Icon.svelte'
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { isSpotify, playerName, playerOf } from '../lib/activity'
  import type { Listening } from '../lib/protocol/Listening'

  let { listening }: { listening: Pick<Listening, 'player'> } = $props()
  const name = $derived(playerName(playerOf(listening)))
</script>

{#if isSpotify(listening)}<SpotifyIcon />{:else}<span class="player-icon" data-tip={name}><Icon name="music" size={18} label={name} /></span>{/if}

<style>
  .player-icon { display: inline-flex; flex: none; }
</style>
