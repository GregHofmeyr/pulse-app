<script lang="ts">
  // The drag handle between a side column and the main area (#160): it sits in the 8px gap between the panels.
  // Drag it, or focus it and use the arrow keys (Shift: bigger steps, Home / End: the limits); a double-click puts
  // the column back to its default. A drag moves the width live and saves it once, on release.
  import { onDestroy } from 'svelte'
  import { keyWidth, type Side } from '../lib/columns'

  let { side, width, min, max, controls, label, onResize, onReset }: {
    side: Side
    /** The column's width as shown. */
    width: number
    min: number
    max: number
    /** The column's element id. */
    controls: string
    label: string
    /** `done`: the end of a drag, or a key: save it. */
    onResize: (width: number, done: boolean) => void
    onReset: () => void
  } = $props()

  let drag = $state<null | { x: number; w: number; last: number; moved: boolean }>(null)
  const clamp = (v: number) => Math.max(min, Math.min(max, Math.round(v)))

  function move(e: MouseEvent) {
    if (!drag) return
    const dx = e.clientX - drag.x
    const w = clamp(drag.w + (side === 'left' ? dx : -dx))
    if (w === drag.last && !drag.moved) return
    drag.last = w
    drag.moved = true
    onResize(w, false)
  }
  function end() {
    if (!drag) return
    const { w, last, moved } = drag
    stop()
    // a click that didn't move (a double-click is two) leaves an unsaved default unsaved
    if (moved || last !== w) onResize(last, true)
  }
  function stop() {
    drag = null
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', end)
    window.removeEventListener('pointercancel', end)
    document.documentElement.classList.remove('resizing-columns')
  }
  function start(e: MouseEvent) {
    if (e.button !== 0) return
    e.preventDefault()
    drag = { x: e.clientX, w: width, last: width, moved: false }
    // on the window: the pointer leaves the thin handle at once
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', end)
    window.addEventListener('pointercancel', end)
    // the resize cursor everywhere, and no text selected on the way
    document.documentElement.classList.add('resizing-columns')
  }
  function keydown(e: KeyboardEvent) {
    const w = keyWidth(side, width, e.key, e.shiftKey, min, max)
    if (w === null) return
    e.preventDefault()
    onResize(w, true)
  }
  onDestroy(stop)
</script>

<!-- a focusable separator is a widget (it has a value the keys change): the linter takes every separator as static -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="handle" class:dragging={!!drag} data-side={side} role="separator" tabindex="0" aria-orientation="vertical" aria-label={label}
  aria-controls={controls} aria-valuenow={width} aria-valuemin={min} aria-valuemax={max} data-tip="Drag to resize · double-click to reset"
  onpointerdown={start} onkeydown={keydown} ondblclick={onReset}></div>

<style>
  /* In the body's gap: 8px wide, with -8px margins so the row's spacing is as it was without it. */
  .handle { position: relative; z-index: 2; flex: 0 0 8px; width: 8px; margin: 0 -8px; cursor: col-resize; touch-action: none; outline: none; }
  /* the line: drawn only while it's being used */
  .handle::after { content: ''; position: absolute; top: 12px; bottom: 12px; left: 3px; width: 2px; border-radius: 1px; background: transparent; transition: background-color var(--dur-fast) var(--ease); }
  .handle:hover::after { background: var(--line); }
  .handle.dragging::after, .handle:focus-visible::after { background: var(--focus-ring); }
</style>
