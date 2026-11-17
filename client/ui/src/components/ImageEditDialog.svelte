<script lang="ts">
  // Edit a queued image before it's sent (#290): crop (free or a ratio), turn it a quarter either way, or draw on it.
  // All on this device, on a canvas; Save hands back a PNG, which the composer queues in place of the original (and
  // converts to WebP, as every queued image is). The geometry is lib/imageEdit.ts.
  import { onMount } from 'svelte'
  import { modal } from '../lib/actions'
  import { api, type StagedFile } from '../lib/tauri'
  import { panCrop } from '../lib/crop'
  import {
    RATIOS, dragCorner, editedName, fromTurned, fullCrop, growCrop, turn, turnCrop, turnedSize, turnMatrix, unchanged, wholeCrop,
    type Corner, type CropRect, type Point, type Quarter, type Stroke,
  } from '../lib/imageEdit'
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'

  let { file, onSave, onCancel }: {
    file: StagedFile
    /** The edited picture (PNG bytes) and its name. */
    onSave: (bytes: Uint8Array, name: string) => void | Promise<void>
    onCancel: () => void
  } = $props()

  /** The display canvas's longest side at most (the saved picture keeps every pixel). */
  const VIEW_MAX = 1200
  /** Pens, from the theme's own tokens (read when the dialog opens). */
  const PENS: { token: string; label: string }[] = [
    { token: '--accent', label: 'Accent' },
    { token: '--danger', label: 'Red' },
    { token: '--warn', label: 'Yellow' },
    { token: '--ok', label: 'Green' },
    { token: '--on-video', label: 'White' },
  ]
  const CORNERS: Corner[] = ['nw', 'ne', 'sw', 'se']

  let src = $state('')
  let img = $state<HTMLImageElement>()
  let canvas = $state<HTMLCanvasElement>()
  let W = $state(0)
  let H = $state(0)
  let q = $state<Quarter>(0)
  let ratio = $state<number | null>(null)
  let crop = $state<CropRect | null>(null)
  let strokes = $state<Stroke[]>([])
  let mode = $state<'crop' | 'draw'>('crop')
  let pen = $state(0)
  let penColours = $state<string[]>([])
  let unreadable = $state(false)
  let saving = $state(false)
  let error = $state<string | null>(null)
  /** What the pointer is doing on the picture. */
  let gesture: { kind: 'move'; last: Point } | { kind: 'corner'; corner: Corner } | { kind: 'draw' } | null = null

  const size = $derived(turnedSize(W, H, q))
  const scale = $derived(W ? Math.min(1, VIEW_MAX / Math.max(size.w, size.h)) : 1)
  const penWidth = $derived(Math.max(2, Math.round(Math.max(W, H) / 200)))

  // The CSP allows data: images and not blob:, so the picture goes in as a data URL.
  onMount(() => {
    const css = getComputedStyle(document.documentElement)
    penColours = PENS.map((p) => css.getPropertyValue(p.token).trim())
    api.stagedImageBytes(file.id).then(
      (buf) => {
        const reader = new FileReader()
        reader.onload = () => (src = String(reader.result))
        reader.onerror = () => (unreadable = true)
        reader.readAsDataURL(new Blob([buf], { type: file.content_type || 'image/webp' }))
      },
      () => (unreadable = true),
    )
  })

  function loaded() {
    if (!img?.naturalWidth || !img.naturalHeight) {
      unreadable = true
      return
    }
    W = img.naturalWidth
    H = img.naturalHeight
    crop = fullCrop(W, H, null)
  }

  /** Draws the turned picture and its strokes, `k` canvas pixels per picture pixel, moved by (-dx, -dy). */
  function paint(ctx: CanvasRenderingContext2D, k: number, dx = 0, dy = 0) {
    if (!img) return
    const [a, b, c, d, e, f] = turnMatrix(W, H, q)
    ctx.setTransform(1, 0, 0, 1, 0, 0)
    ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height)
    ctx.setTransform(a * k, b * k, c * k, d * k, (e - dx) * k, (f - dy) * k)
    ctx.drawImage(img, 0, 0)
    ctx.lineCap = 'round'
    ctx.lineJoin = 'round'
    for (const s of strokes) {
      if (!s.points.length) continue
      ctx.strokeStyle = s.color
      ctx.lineWidth = s.width
      ctx.beginPath()
      ctx.moveTo(s.points[0].x, s.points[0].y)
      // one point is a dot: a zero-length line with round caps
      for (const p of s.points.length > 1 ? s.points.slice(1) : s.points) ctx.lineTo(p.x, p.y)
      ctx.stroke()
    }
  }

  $effect(() => {
    if (!canvas || !W) return
    // read what the picture depends on, so it redraws when any of them changes
    void q
    void strokes.length
    for (const s of strokes) void s.points.length
    canvas.width = Math.max(1, Math.round(size.w * scale))
    canvas.height = Math.max(1, Math.round(size.h * scale))
    const ctx = canvas.getContext('2d')
    if (ctx) paint(ctx, scale)
  })

  function rotate(by: 1 | -1) {
    if (!crop) return
    const before = size
    q = turn(q, by)
    crop = ratio === null ? turnCrop(crop, before.w, before.h, by) : fullCrop(size.w, size.h, ratio)
  }

  function pickRatio(r: number | null) {
    ratio = r
    if (r !== null) crop = fullCrop(size.w, size.h, r)
  }

  /** The pointer, in turned picture pixels. */
  function at(e: PointerEvent): Point {
    const r = canvas!.getBoundingClientRect()
    return {
      x: Math.min(size.w, Math.max(0, ((e.clientX - r.left) / (r.width || 1)) * size.w)),
      y: Math.min(size.h, Math.max(0, ((e.clientY - r.top) / (r.height || 1)) * size.h)),
    }
  }

  function down(e: PointerEvent) {
    if (!crop || !canvas || e.button !== 0) return
    const p = at(e)
    const corner = (e.target as HTMLElement).dataset?.corner as Corner | undefined
    if (mode === 'draw') {
      strokes = [...strokes, { color: penColours[pen] || 'currentColor', width: penWidth, points: [fromTurned(p, W, H, q)] }]
      gesture = { kind: 'draw' }
    } else if (corner) {
      gesture = { kind: 'corner', corner }
    } else if (p.x >= crop.x && p.x <= crop.x + crop.w && p.y >= crop.y && p.y <= crop.y + crop.h) {
      gesture = { kind: 'move', last: p }
    } else return
    ;(e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId)
    e.preventDefault()
  }

  function move(e: PointerEvent) {
    if (!gesture || !crop) return
    const p = at(e)
    if (gesture.kind === 'draw') {
      const last = strokes[strokes.length - 1]
      last.points.push(fromTurned(p, W, H, q))
    } else if (gesture.kind === 'corner') {
      crop = dragCorner(crop, size.w, size.h, gesture.corner, p, ratio)
    } else {
      crop = panCrop(crop, size.w, size.h, p.x - gesture.last.x, p.y - gesture.last.y)
      gesture.last = p
    }
  }
  const up = () => (gesture = null)

  function undoStroke() {
    strokes = strokes.slice(0, -1)
  }

  function key(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z' && strokes.length) {
      undoStroke()
      e.preventDefault()
      return
    }
    if (!crop || mode !== 'crop') return
    const step = e.shiftKey ? 0.1 : 0.01
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-size.w * step, 0],
      ArrowRight: [size.w * step, 0],
      ArrowUp: [0, -size.h * step],
      ArrowDown: [0, size.h * step],
    }
    const m = moves[e.key]
    if (m) crop = panCrop(crop, size.w, size.h, m[0], m[1])
    else if (e.key === '+' || e.key === '=') crop = growCrop(crop, size.w, size.h, 1.1)
    else if (e.key === '-' || e.key === '_') crop = growCrop(crop, size.w, size.h, 1 / 1.1)
    else return
    e.preventDefault()
  }

  async function save() {
    if (!crop || !img || saving) return
    if (unchanged(crop, size.w, size.h, q, strokes)) return onCancel()
    saving = true
    error = null
    try {
      const c = wholeCrop(crop, size.w, size.h)
      const out = document.createElement('canvas')
      out.width = c.w
      out.height = c.h
      const ctx = out.getContext('2d')
      if (!ctx) throw new Error('no canvas')
      paint(ctx, 1, c.x, c.y)
      // a canvas holds pixels only: the PNG carries no EXIF or other metadata
      const blob = await new Promise<Blob | null>((ok) => out.toBlob(ok, 'image/png'))
      if (!blob) throw new Error('no image')
      await onSave(new Uint8Array(await blob.arrayBuffer()), editedName(file.name))
    } catch {
      error = "The edit couldn't be saved. Try again."
      saving = false
    }
  }

  const pct = (v: number, of: number) => `${(v / (of || 1)) * 100}%`
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onCancel}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={`Edit ${file.name}`} tabindex="-1" use:modal={onCancel}>
    <h2 class="title">Edit image</h2>
    {#if unreadable}
      <p class="error" role="alert">This picture can't be edited.</p>
    {:else}
      <div class="tools">
        <Segmented label="Tool" options={[{ value: 'crop' as const, label: 'Crop' }, { value: 'draw' as const, label: 'Draw' }]} value={mode} onchange={(v) => (mode = v)} />
        <div class="turns">
          <button type="button" class="icon-btn" aria-label="Rotate left" data-tip="Rotate left" disabled={!crop} onclick={() => rotate(-1)}><Icon name="rotateLeft" size={16} /></button>
          <button type="button" class="icon-btn" aria-label="Rotate right" data-tip="Rotate right" disabled={!crop} onclick={() => rotate(1)}><Icon name="rotateRight" size={16} /></button>
        </div>
      </div>
      {#if mode === 'crop'}
        <div class="row" role="radiogroup" aria-label="Crop ratio">
          {#each RATIOS as r (r.label)}
            <button type="button" role="radio" class="ratio" class:on={ratio === r.value} aria-checked={ratio === r.value} onclick={() => pickRatio(r.value)}>{r.label}</button>
          {/each}
        </div>
      {:else}
        <div class="row">
          <div class="pens" role="radiogroup" aria-label="Pen colour">
            {#each PENS as p, i (p.token)}
              <button type="button" role="radio" class="pen" class:on={pen === i} aria-checked={pen === i} aria-label={p.label} data-tip={p.label}
                style:background={`var(${p.token})`} onclick={() => (pen = i)}></button>
            {/each}
          </div>
          <button type="button" class="icon-btn" aria-label="Undo the last stroke" data-tip="Undo (Ctrl+Z)" disabled={!strokes.length} onclick={undoStroke}><Icon name="undo" size={16} /></button>
        </div>
      {/if}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div class="stage" class:drawing={mode === 'draw'} role="application" tabindex="0"
        aria-label={mode === 'crop' ? 'Crop area: arrow keys move the crop (Shift for bigger steps), + and − resize it' : 'Drawing area: draw with the pointer; Ctrl+Z undoes a stroke'}
        onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} onkeydown={key}>
        {#if src}
          <img class="source" bind:this={img} {src} alt="" onload={loaded} onerror={() => (unreadable = true)} />
        {/if}
        <div class="frame">
          <canvas bind:this={canvas}></canvas>
          {#if crop}
            <div class="box" class:quiet={mode === 'draw'}
              style:left={pct(crop.x, size.w)} style:top={pct(crop.y, size.h)} style:width={pct(crop.w, size.w)} style:height={pct(crop.h, size.h)}>
              {#if mode === 'crop'}
                {#each CORNERS as c (c)}<span class="handle {c}" data-corner={c}></span>{/each}
              {/if}
            </div>
          {/if}
        </div>
      </div>
      {#if crop}<p class="dims">{Math.round(crop.w)} × {Math.round(crop.h)}</p>{/if}
      <p class="note">Saving makes a new copy of the picture without its metadata (EXIF), such as where and when a photo was taken.</p>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    {/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" onclick={onCancel}>Cancel</button>
      {#if !unreadable}
        <button type="button" class="btn btn-lg btn-primary" disabled={!crop || saving} onclick={() => void save()}>{saving ? 'Saving…' : 'Save'}</button>
      {/if}
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 40; }
  .modal { width: min(720px, 100%); }
  .title { margin: 0; font-size: 16px; }
  .tools, .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .row { justify-content: flex-start; gap: 6px; }
  .turns { display: flex; gap: 4px; }
  .ratio { padding: 4px 10px; border: 1px solid var(--line); border-radius: var(--radius-md); background: transparent; color: var(--text-2); font-size: 12px; font-weight: 600; }
  .ratio:not(.on):hover { background: var(--bg-3); color: var(--text); }
  .ratio.on { background: var(--accent); border-color: var(--focus-ring); color: var(--on-accent); }
  .pens { display: flex; gap: 6px; margin-right: 6px; }
  .pen { width: 22px; height: 22px; padding: 0; border-radius: 50%; border: 2px solid var(--line-strong); }
  .pen.on { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .stage { display: grid; place-items: center; background: var(--bg-inset); border-radius: var(--radius-md); overflow: hidden; touch-action: none; outline-offset: 2px; user-select: none; }
  .stage.drawing { cursor: crosshair; }
  /* decoded here, drawn on the canvas */
  .source { display: none; }
  .frame { position: relative; line-height: 0; }
  canvas { display: block; max-width: 100%; max-height: 55vh; }
  /* the shadow dims everything outside the crop */
  .box { position: absolute; box-sizing: border-box; border: 2px solid var(--on-video); box-shadow: 0 0 0 9999px var(--video-chrome-strong); cursor: move; }
  .box.quiet { pointer-events: none; border-style: dashed; }
  .handle { position: absolute; width: 14px; height: 14px; box-sizing: border-box; background: var(--on-video); border: 2px solid var(--video-chrome-strong); border-radius: 50%; }
  .handle.nw { left: -8px; top: -8px; cursor: nwse-resize; }
  .handle.se { right: -8px; bottom: -8px; cursor: nwse-resize; }
  .handle.ne { right: -8px; top: -8px; cursor: nesw-resize; }
  .handle.sw { left: -8px; bottom: -8px; cursor: nesw-resize; }
  .dims, .note { margin: 0; font-size: 12px; color: var(--text-3); }
  .error { margin: 0; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
