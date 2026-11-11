<script lang="ts">
  // A message's images full size (#57): ←/→ (or the side buttons) for the next and previous, the wheel or +/− to zoom,
  // Open original size for 1:1, Save image. Esc, the X or a click on the backdrop closes it.
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import type { Attachment } from '../lib/protocol/Attachment'
  import { api, errorText } from '../lib/tauri'
  import { attachmentLink } from '../lib/attachmentLinks'

  let { images, messageId, start = 0, onClose, onError }: {
    images: readonly Attachment[]
    messageId: string
    start?: number
    onClose: () => void
    onError?: (text: string) => void
  } = $props()

  const ZOOM_MIN = 1
  const ZOOM_MAX = 4
  const ZOOM_STEP = 0.25

  // svelte-ignore state_referenced_locally
  let index = $state(start)
  let zoom = $state(1)
  let original = $state(false)
  let src = $state<string | null>(null)
  let saving = $state(false)
  const img = $derived(images[index])

  $effect(() => {
    const a = img
    src = null
    zoom = 1
    original = false
    let gone = false
    attachmentLink(messageId, a.id, api.attachmentUrls).then(
      (url) => !gone && (src = url),
      (e) => !gone && onError?.(errorText(e)),
    )
    return () => (gone = true)
  })

  const go = (step: number) => (index = (index + step + images.length) % images.length)
  const setZoom = (z: number) => (zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z)))

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowRight' && images.length > 1) go(1)
    else if (e.key === 'ArrowLeft' && images.length > 1) go(-1)
    else if (e.key === '+' || e.key === '=') setZoom(zoom + ZOOM_STEP)
    else if (e.key === '-') setZoom(zoom - ZOOM_STEP)
    else return
    e.preventDefault()
  }

  function wheel(e: WheelEvent) {
    e.preventDefault()
    setZoom(zoom + (e.deltaY < 0 ? ZOOM_STEP : -ZOOM_STEP))
  }

  async function save() {
    if (saving) return
    saving = true
    try {
      await api.saveAttachment(messageId, img.id, img.name)
    } catch (e) {
      onError?.(errorText(e))
    } finally {
      saving = false
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<div class="viewer" role="dialog" aria-modal="true" aria-label={img.name} tabindex="-1" use:modal={onClose} onkeydown={key}
  onclick={(e) => e.target === e.currentTarget && onClose()}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="stage" class:original onwheel={wheel} onclick={(e) => e.target === e.currentTarget && onClose()}>
    {#if src}
      <img {src} alt={img.name} draggable="false" style:transform={original ? null : `scale(${zoom})`}
        width={img.width ?? undefined} height={img.height ?? undefined} />
    {/if}
  </div>
  {#if images.length > 1}
    <button type="button" class="nav prev icon-btn round" aria-label="Previous image" onclick={() => go(-1)}><Icon name="chevron" size={18} /></button>
    <button type="button" class="nav next icon-btn round" aria-label="Next image" onclick={() => go(1)}><Icon name="chevron" size={18} /></button>
  {/if}
  <div class="bar">
    <span class="name">{img.name}{#if images.length > 1}<span class="count"> · {index + 1} of {images.length}</span>{/if}</span>
    <button type="button" class="btn btn-quiet" aria-label="Zoom out" disabled={original || zoom <= ZOOM_MIN} onclick={() => setZoom(zoom - ZOOM_STEP)}>−</button>
    <button type="button" class="btn btn-quiet" aria-label="Zoom in" disabled={original || zoom >= ZOOM_MAX} onclick={() => setZoom(zoom + ZOOM_STEP)}>+</button>
    <button type="button" class="btn btn-quiet" aria-pressed={original} onclick={() => (original = !original)}>{original ? 'Fit to window' : 'Open original size'}</button>
    <button type="button" class="btn btn-quiet" disabled={saving} onclick={save}><Icon name="download" size={14} /> Save image</button>
  </div>
  <button type="button" class="close icon-btn round modal-close" aria-label="Close" onclick={onClose}><Icon name="x" size={16} /></button>
</div>

<style>
  /* the quote image lightbox's backdrop (QuoteItem .lightbox) */
  .viewer { position: fixed; inset: 0; z-index: 70; background: var(--scrim-strong); display: flex; flex-direction: column; outline: none; }
  .stage { flex: 1; min-height: 0; display: grid; place-items: center; overflow: hidden; padding: 48px 72px 0; }
  .stage img { max-width: 100%; max-height: 100%; width: auto; height: auto; object-fit: contain; transition: transform var(--dur) var(--ease); }
  /* 1:1: the image at its own size, scrolled when bigger than the window */
  .stage.original { overflow: auto; place-items: start center; }
  .stage.original img { max-width: none; max-height: none; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 12px 16px 16px; justify-content: center; flex-wrap: wrap; }
  .name { color: var(--text); font-size: 13px; max-width: 40vw; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-right: 8px; }
  .count { color: var(--text-3); }
  .nav { position: absolute; top: 50%; translate: 0 -50%; }
  .prev { left: 16px; }
  .prev :global(svg) { rotate: 90deg; }
  .next { right: 16px; }
  .next :global(svg) { rotate: -90deg; }
  .close { position: absolute; top: 16px; right: 16px; }
  @media (prefers-reduced-motion: reduce) {
    .stage img { transition: none; }
  }
</style>
