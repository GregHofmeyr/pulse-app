<script lang="ts">
  // A server's banner (#475), filling its box: the picture set for it (an animated one moves as a profile card's
  // banner does: on screen, focused, and not with reduced motion), else a solid gradient of the server's two colours
  // (never fading into the page). Nothing is ever drawn over it but the server's chip.
  import type { Attachment } from 'svelte/attachments'
  import { avatarColors } from '../lib/avatar'
  import { mediaUrl } from '../lib/emoji'
  import { shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'

  let { id, hash = null, still = null }: { id: string; hash?: string | null; still?: string | null } = $props()

  const colors = $derived(avatarColors(id))
  const wire = $derived<Attachment<HTMLImageElement>>((img) => {
    if (hash && still && still !== hash) return wireAnimated(img, mediaUrl(hash), mediaUrl(still), (s) => shouldAnimateProfile('card', s))
  })
</script>

<div class="server-banner" class:pic={!!hash} style:--srv={colors[0]} style:--srv-2={colors[1]} data-server-banner={hash ?? ''}>
  {#if hash}<img src={mediaUrl(still ?? hash)} alt="" draggable="false" {@attach wire} />{/if}
</div>

<style>
  .server-banner { width: 100%; height: 100%; background: linear-gradient(120deg, var(--srv), var(--srv-2)); }
  .server-banner.pic { background: var(--bg-3); }
  img { width: 100%; height: 100%; object-fit: cover; display: block; }
</style>
