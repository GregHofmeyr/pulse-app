<script lang="ts">
  // "Mute join sound" / "Unmute join sound" (#121): an ordinary item in every person's right-click menu (voice rows and
  // call tiles, member list, message author, DM list, profile card), like "Mute soundboard". On this device only: their
  // join sound plays as the normal Join cue for you. Shown for someone else who has a join sound in a server you share,
  // or whom you've muted already (so it can be undone after they clear theirs). Never for yourself.
  import { app } from '../lib/store.svelte'
  import { hasJoinSound } from '../lib/joinSounds'
  import { joinSounds, setJoinSoundMute } from '../lib/joinSounds.svelte'

  let { userId, pick }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const muted = $derived(!!joinSounds.mutes[userId])
  const shown = $derived(!self && (muted || hasJoinSound(app.state, userId)))
</script>

{#if shown}
  <button type="button" class="menu-item" role="menuitem" data-item="join-sound-mute" onclick={() => pick(() => setJoinSoundMute(userId, !muted))}>
    {muted ? 'Unmute join sound' : 'Mute join sound'}
  </button>
{/if}
