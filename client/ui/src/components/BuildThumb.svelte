<script lang="ts">
  // A build's picture (#617): its first screenshot, else its video's thumbnail, else a plain tile.
  import Icon from './Icon.svelte'
  import { buildThumb } from '../lib/builds'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { api } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { message }: { message: Message } = $props()

  const thumb = $derived(buildThumb(message))
  let shotUrl = $state<string | null>(null)
  let broken = $state(false)
  $effect(() => {
    const t = thumb
    shotUrl = null
    broken = false
    if (t.kind !== 'shot') return
    let stale = false
    attachmentLink(message.id, t.attachment.id, api.attachmentUrls).then(
      (url) => { if (!stale) shotUrl = url },
      () => { if (!stale) broken = true },
    )
    return () => { stale = true }
  })
  const src = $derived(
    thumb.kind === 'shot' ? shotUrl : thumb.kind === 'video' && isMediaHash(thumb.thumb.hash) ? mediaUrl(thumb.thumb.hash) : null,
  )
</script>

<div class="thumb">
  {#if src && !broken}
    <img {src} alt="" loading="lazy" decoding="async" draggable="false" onerror={() => (broken = true)} />
  {:else}
    <span class="tile" aria-hidden="true"><Icon name="hammer" size={22} /></span>
  {/if}
</div>

<style>
  .thumb { position: relative; width: 100%; height: 100%; background: var(--bg-3); overflow: hidden; }
  img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .tile { position: absolute; inset: 0; display: grid; place-items: center; color: var(--text-3); }
</style>
