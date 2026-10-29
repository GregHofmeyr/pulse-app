<script lang="ts">
  import { onMount } from 'svelte'
  import { modal } from '../lib/actions'
  import { ASPECT, initialCrop, panCrop, roundCrop, withoutPngExif, zoomCrop, type CropRect } from '../lib/crop'

  let { file, kind, onApply, onCancel }: {
    file: File
    kind: 'avatar' | 'banner'
    onApply: (crop: CropRect) => void
    onCancel: () => void
  } = $props()

  const MAX_BYTES = 8 * 1024 * 1024
  const aspect = $derived(ASPECT[kind])
  const tooBig = $derived(file.size > MAX_BYTES)

  let src = $state('')
  let img = $state<HTMLImageElement>()
  let stage = $state<HTMLDivElement>()
  let natW = $state(0)
  let natH = $state(0)
  /** In fractional source pixels while it moves; rounded on Apply. */
  let crop = $state<CropRect | null>(null)
  let drag: { x: number; y: number } | null = null
  /** The file couldn't be read or decoded as a picture: only Cancel is left. */
  let unreadable = $state(false)

  // The CSP allows data: images and not blob:, so the picture goes in as a data URL. A PNG's eXIf
  // is left out: the server ignores its orientation, and this way every webview does too.
  onMount(() => {
    if (tooBig) return
    const reader = new FileReader()
    reader.onload = () => (src = String(reader.result))
    reader.onerror = () => (unreadable = true)
    file.arrayBuffer().then(
      (b) => reader.readAsDataURL(new Blob([withoutPngExif(new Uint8Array(b))], { type: file.type })),
      () => (unreadable = true),
    )
  })

  function loaded() {
    if (!img?.naturalWidth || !img.naturalHeight) {
      unreadable = true
      return
    }
    natW = img.naturalWidth
    natH = img.naturalHeight
    crop = initialCrop(natW, natH, aspect)
  }

  const maxZoom = $derived(natW ? Math.max(1, Math.min(natW, natH * aspect) / (aspect >= 1 ? 32 * aspect : 32)) : 1)
  const zoom = $derived(crop && natW ? Math.min(natW, natH * aspect) / crop.w : 1)

  function zoomBy(factor: number, focus?: { x: number; y: number }) {
    if (!crop) return
    crop = zoomCrop(crop, natW, natH, aspect, factor, focus ?? { x: crop.x + crop.w / 2, y: crop.y + crop.h / 2 })
  }

  /** Source pixels per screen pixel (1 when the picture has no size on screen yet). */
  const ratio = () => {
    const w = img?.getBoundingClientRect().width
    return w ? natW / w : 1
  }

  function down(e: PointerEvent) {
    drag = { x: e.clientX, y: e.clientY }
    stage?.setPointerCapture?.(e.pointerId)
  }
  function move(e: PointerEvent) {
    if (!drag || !crop) return
    const r = ratio()
    crop = panCrop(crop, natW, natH, -(e.clientX - drag.x) * r, -(e.clientY - drag.y) * r)
    drag = { x: e.clientX, y: e.clientY }
  }
  const up = () => (drag = null)

  function key(e: KeyboardEvent) {
    if (!crop) return
    const step = e.shiftKey ? 0.1 : 0.01
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-natW * step, 0],
      ArrowRight: [natW * step, 0],
      ArrowUp: [0, -natH * step],
      ArrowDown: [0, natH * step],
    }
    const m = moves[e.key]
    if (m) crop = panCrop(crop, natW, natH, m[0], m[1])
    else if (e.key === '+' || e.key === '=') zoomBy(1.1)
    else if (e.key === '-' || e.key === '_') zoomBy(1 / 1.1)
    else return
    e.preventDefault()
  }

  // Not passive, so the wheel zooms the crop instead of scrolling the dialog.
  $effect(() => {
    const el = stage
    if (!el) return
    const wheel = (e: WheelEvent) => {
      e.preventDefault()
      const box = img?.getBoundingClientRect()
      const r = ratio()
      zoomBy(e.deltaY < 0 ? 1.1 : 1 / 1.1, box && { x: (e.clientX - box.left) * r, y: (e.clientY - box.top) * r })
    }
    el.addEventListener('wheel', wheel, { passive: false })
    return () => el.removeEventListener('wheel', wheel)
  })

  const pct = (v: number, of: number) => `${(v / of) * 100}%`
</script>

<div class="overlay">
  <div class="backdrop" role="presentation" data-backdrop onclick={onCancel}></div>
  <div class="modal" role="dialog" aria-modal="true" aria-label={kind === 'avatar' ? 'Crop avatar' : 'Crop banner'} tabindex="-1" use:modal={onCancel}>
    {#if tooBig}
      <p class="error" role="alert">Profile pictures and banners can be up to 8 MB.</p>
    {:else if unreadable}
      <p class="error" role="alert">That file isn't a picture Pulse can read (PNG, JPEG, GIF or WebP).</p>
    {:else}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div class="stage" role="application" aria-label="Crop area" tabindex="0" bind:this={stage}
        onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} onkeydown={key}>
        {#if src}
          <div class="frame">
            <img bind:this={img} {src} alt="" draggable="false" onload={loaded} onerror={() => (unreadable = true)} />
            {#if crop}
              <div class="box" class:round={kind === 'avatar'}
                style:left={pct(crop.x, natW)} style:top={pct(crop.y, natH)}
                style:width={pct(crop.w, natW)} style:height={pct(crop.h, natH)}></div>
            {/if}
          </div>
        {/if}
      </div>
      <label class="zoom">Zoom
        <input type="range" min="1" max={maxZoom} step="0.01" value={zoom} disabled={!crop}
          oninput={(e) => zoomBy(Number(e.currentTarget.value) / zoom)} />
      </label>
    {/if}
    <div class="actions">
      <button type="button" onclick={onCancel}>Cancel</button>
      {#if !tooBig && !unreadable}
        <button type="button" class="primary" disabled={!crop} onclick={() => crop && onApply(roundCrop($state.snapshot(crop), natW, natH))}>Apply</button>
      {/if}
    </div>
  </div>
</div>

<style>
  /* flex-centred (not transform): WebKitGTK blurs transformed text */
  .overlay { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .modal { position: relative; width: min(520px, 100%); max-height: calc(100vh - 32px); overflow-y: auto; padding: 22px; border-radius: 16px; background: var(--bg-2); border: 1px solid var(--line); display: flex; flex-direction: column; gap: 12px; }
  .stage { display: grid; place-items: center; background: #000; border-radius: 10px; overflow: hidden; touch-action: none; cursor: grab; outline-offset: 2px; }
  /* its overflow clips the crop box's shadow */
  .frame { position: relative; display: block; line-height: 0; overflow: hidden; }
  img { display: block; max-width: 100%; max-height: 50vh; user-select: none; }
  /* the shadow dims everything outside the crop */
  .box { position: absolute; box-sizing: border-box; border: 2px solid #fff; box-shadow: 0 0 0 9999px rgba(0, 0, 0, .6); pointer-events: none; }
  .box.round { border-radius: 50%; }
  .zoom { display: flex; align-items: center; gap: 12px; color: var(--text-3); font-size: 12px; font-weight: 600; }
  .zoom input { flex: 1; }
  .error { margin: 0; color: var(--text-2); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
