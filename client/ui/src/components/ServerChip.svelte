<script lang="ts">
  // A server's picture wherever a small one stands for it (#80: the invite dialog and card, the "Invite to server"
  // flyout; #55: the rail and Server settings › Overview): its icon when it has one, else the rail's chip (its initial
  // on its colour). An animated icon holds its still until the pointer is on it, and always with reduced motion.
  import type { Attachment } from 'svelte/attachments'
  import { avatarColor, initial } from '../lib/avatar'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'

  let { id, name, iconHash = null, still = null, size = 22, radius = '28%' }: {
    id: string
    name: string
    iconHash?: string | null
    /** An animated icon's still (#55). */
    still?: string | null
    size?: number
    radius?: string
  } = $props()
  const src = $derived(iconHash && isMediaHash(iconHash) ? mediaUrl(still && isMediaHash(still) ? still : iconHash) : null)
  const wire = $derived<Attachment<HTMLImageElement>>((img) => {
    if (iconHash && still && still !== iconHash && isMediaHash(still)) return wireAnimated(img, mediaUrl(iconHash), mediaUrl(still), (s) => shouldAnimateProfile('list', s))
  })
</script>

{#if src}
  <img class="chip" {src} alt="" draggable="false" style:width="{size}px" style:height="{size}px" style:border-radius={radius} {@attach wire} />
{:else}
  <span class="chip" aria-hidden="true" style:width="{size}px" style:height="{size}px" style:border-radius={radius} style:font-size="{Math.round(size / 2)}px" style:background={avatarColor(id)}>{initial(name)}</span>
{/if}

<style>
  .chip { flex-shrink: 0; color: #fff; font-weight: 700; display: grid; place-items: center; object-fit: cover; }
</style>
