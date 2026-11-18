<script lang="ts">
  // A server's picture wherever a small one stands for it outside the rail (#80: the invite dialog and card, the
  // "Invite to server" flyout): its icon when it has one, else the rail's chip (its initial on its colour).
  import { avatarColor, initial } from '../lib/avatar'
  import { isMediaHash, mediaUrl } from '../lib/emoji'

  let { id, name, iconHash = null, size = 22 }: { id: string; name: string; iconHash?: string | null; size?: number } = $props()
  const src = $derived(iconHash && isMediaHash(iconHash) ? mediaUrl(iconHash) : null)
</script>

{#if src}
  <img class="chip" {src} alt="" style:width="{size}px" style:height="{size}px" />
{:else}
  <span class="chip" aria-hidden="true" style:width="{size}px" style:height="{size}px" style:font-size="{Math.round(size / 2)}px" style:background={avatarColor(id)}>{initial(name)}</span>
{/if}

<style>
  .chip { flex-shrink: 0; border-radius: 28%; color: #fff; font-weight: 700; display: grid; place-items: center; object-fit: cover; }
</style>
