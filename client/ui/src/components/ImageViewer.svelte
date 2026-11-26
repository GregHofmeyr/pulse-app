<script lang="ts">
  // A message's images full size (#57): ←/→ (or the side buttons) for the next and previous, the wheel or +/− to zoom,
  // Open original size for 1:1, Save image. Esc, the X or a click on the backdrop closes it.
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import type { Attachment } from '../lib/protocol/Attachment'
  import { api, errorText } from '../lib/tauri'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { saveFile } from '../lib/clipDownload'
  import { downloads, fileKey, holdSaved } from '../lib/downloads.svelte'
  import Loading from './Loading.svelte'
  import ProgressRing from './ProgressRing.svelte'

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
  /** The image on show has its bytes (until then the stage says it's loading, never sits empty). */
  let loaded = $state(false)
  const img = $derived(images[index])
  // Save image (#205): a ring and Cancel while it saves, Saved for a moment
  const dl = $derived(downloads.view(fileKey(img.id)))
  const percent = $derived(dl?.phase === 'downloading' && dl.fraction !== null ? Math.round(dl.fraction * 100) : null)

  $effect(() => {
    const a = img
    src = null
    loaded = false
    zoom = 1
    original = false
    let gone = false
    attachmentLink(messageId, a.id, api.attachmentUrls).then(
      (url) => !gone && (src = url),
      (e) => !gone && onError?.(errorText(e)),
    )
    return () => (gone = true)
  })

  // The images either side of the one on show, fetched ahead (H796T1): a build's screenshots past its fourth tile (and
  // a message's spoilered or off-screen ones) are never drawn before the viewer reaches them, so stepping onto one
  // left the stage empty while its bytes came. Each is fetched once; the signed link is the one the stage then uses
  // (attachmentLinks reuses it), so the stage's image comes from the HTTP cache.
  const ahead = new Map<string, HTMLImageElement>()
  $effect(() => {
    const n = images.length
    if (n < 2) return
    const near = new Set([(index + 1) % n, (index - 1 + n) % n])
    near.delete(index)
    for (const i of near) {
      const a = images[i]
      attachmentLink(messageId, a.id, api.attachmentUrls).then((url) => {
        if (ahead.has(url)) return
        const im = new Image()
        im.decoding = 'async'
        im.src = url
        ahead.set(url, im)
      }, () => {})
    }
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

  const save = () => void saveFile(messageId, img, (text) => onError?.(text))
  const reveal = () => void downloads.reveal(fileKey(img.id), (text) => onError?.(text))
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<div class="viewer" role="dialog" aria-modal="true" aria-label={img.name} tabindex="-1" use:modal={onClose} onkeydown={key}
  onclick={(e) => e.target === e.currentTarget && onClose()}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="stage" class:original onwheel={wheel} onclick={(e) => e.target === e.currentTarget && onClose()}>
    {#if src}
      <img {src} alt={img.name} draggable="false" style:transform={original ? null : `scale(${zoom})`} class:pending={!loaded}
        width={img.width ?? undefined} height={img.height ?? undefined} onload={() => (loaded = true)} onerror={() => (loaded = true)} />
    {/if}
    {#if !loaded}<div class="wait"><Loading label="Loading image…" /></div>{/if}
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
    <button type="button" class="btn btn-quiet save" onclick={save}
      aria-label={dl?.phase === 'downloading' ? `Cancel saving${percent === null ? '' : `, ${percent}%`}` : undefined}>
      {#if dl?.phase === 'downloading'}
        <span class="ring-box"><ProgressRing fraction={dl.fraction} size={16} /></span>{percent === null ? '' : `${percent}% · `}Cancel
      {:else if dl?.phase === 'saved'}
        <Icon name="check" size={14} /> Saved
      {:else}
        <Icon name="download" size={14} /> Save image
      {/if}
    </button>
    {#if dl?.phase === 'saved'}
      <button type="button" class="btn btn-quiet" use:holdSaved={fileKey(img.id)} onclick={reveal}><Icon name="folder" size={14} /> Show in folder</button>
    {/if}
  </div>
  <button type="button" class="close icon-btn round modal-close" aria-label="Close" onclick={onClose}><Icon name="x" size={16} /></button>
</div>

<style>
  /* the quote image lightbox's backdrop (QuoteItem .lightbox) */
  .viewer { position: fixed; inset: 0; z-index: 70; background: var(--scrim-strong); display: flex; flex-direction: column; outline: none; }
  .stage { position: relative; flex: 1; min-height: 0; display: grid; place-items: center; overflow: hidden; padding: 48px 72px 0; }
  .stage img { max-width: 100%; max-height: 100%; width: auto; height: auto; object-fit: contain; transition: transform var(--dur) var(--ease); }
  /* 1:1: the image at its own size, scrolled when bigger than the window */
  /* still loading: the image's box stays (no jump when it lands), the Loading line over it */
  .stage img.pending { visibility: hidden; }
  .wait { position: absolute; inset: 48px 72px 0; display: grid; place-items: center; pointer-events: none; }
  .stage.original { overflow: auto; place-items: start center; }
  .stage.original img { max-width: none; max-height: none; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 12px 16px 16px; justify-content: center; flex-wrap: wrap; }
  .name { color: var(--text); font-size: 13px; max-width: 40vw; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-right: 8px; }
  .count { color: var(--text-3); }
  .save { display: inline-flex; align-items: center; gap: 6px; font-variant-numeric: tabular-nums; }
  .ring-box { position: relative; width: 16px; height: 16px; flex: none; }
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
