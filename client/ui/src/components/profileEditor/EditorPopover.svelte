<script lang="ts">
  // The shared shell of the editor's popovers (#727): a large panel over the editor, which is dimmed behind and inert,
  // with tiles on the left, a live preview on the right, and Cancel / Apply. Name style, Card theme and Speaking ring
  // use it. The popover edits the editor's draft live (so the preview is real) and its owner puts the draft back on
  // Cancel; Apply only closes. Nothing saves until the editor's Save.
  //
  // It is a nested layer of the editor: `data-editor-layer` (the editor ignores a click on its backdrop while one is
  // on the page) and Escape taken first, in the capture phase and marked handled, so the editor's close guard leaves it
  // alone (the same as ImageMenu). It is placed inside the editor's own dialog (`portal`), over all of it.
  import { tick, type Snippet } from 'svelte'
  import { portal } from '../../lib/actions'

  let { title, onCancel, onApply, tiles, preview }: {
    title: string
    onCancel: () => void
    onApply: () => void
    tiles: Snippet
    preview: Snippet
  } = $props()

  let layer = $state<HTMLElement>()

  $effect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      onCancel()
    }
    window.addEventListener('keydown', key, true)
    return () => window.removeEventListener('keydown', key, true)
  })

  // The editor behind is inert while the popover is up; only what this sets is cleared again. The focus goes in, and
  // comes back to where it was when the popover closes.
  $effect(() => {
    const el = layer
    if (!el) return
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const inerted: Element[] = []
    for (const sib of el.parentElement?.children ?? []) {
      if (sib === el || sib.hasAttribute('inert') || sib.hasAttribute('data-editor-layer')) continue
      sib.setAttribute('inert', '')
      inerted.push(sib)
    }
    void tick().then(() => el.querySelector<HTMLElement>('[role=radio][tabindex="0"], [role=radio], button, input')?.focus())
    return () => {
      for (const s of inerted) s.removeAttribute('inert')
      if (opener?.isConnected) opener.focus()
    }
  })
</script>

<div bind:this={layer} class="layer" use:portal data-editor-layer role="dialog" aria-modal="true" aria-label={title}>
  <button type="button" class="scrim" tabindex="-1" aria-hidden="true" onclick={onCancel}></button>
  <div class="panel pop-in">
    <h2>{title}</h2>
    <div class="cols">
      <div class="tiles">{@render tiles()}</div>
      <div class="preview" aria-label="Preview">{@render preview()}</div>
    </div>
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" aria-label="Cancel" onclick={onCancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-primary" aria-label="Apply" onclick={onApply}>Apply</button>
    </div>
  </div>
</div>

<style>
  /* over the whole editor dialog, which is its positioned ancestor */
  .layer { position: absolute; inset: 0; z-index: 30; display: grid; place-items: center; padding: 24px; min-width: 0; min-height: 0; }
  .scrim { position: absolute; inset: 0; width: 100%; padding: 0; border: 0; background: var(--scrim-strong); cursor: default; }
  .panel { position: relative; display: flex; flex-direction: column; gap: 16px; width: min(960px, 100%); max-height: 100%; min-width: 0; min-height: 0; padding: 20px; box-sizing: border-box; border: 1px solid var(--line); border-radius: var(--radius-xl); background: var(--bg-2); box-shadow: var(--shadow-3); }
  h2 { margin: 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 16px; font-weight: 600; }
  .cols { flex: 1 1 auto; min-height: 0; display: flex; gap: 20px; }
  .tiles { flex: 1 1 0; min-width: 0; overflow-y: auto; overflow-x: hidden; padding: 4px; }
  .preview { flex: 0 1 380px; min-width: 0; overflow-y: auto; overflow-x: hidden; display: flex; flex-direction: column; align-items: center; gap: 16px; padding: 4px; border-radius: var(--radius-lg); background: var(--bg-1); }
  .actions { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: 8px; flex-shrink: 0; }
  @media (max-width: 760px) {
    .cols { flex-direction: column; overflow-y: auto; }
    .tiles, .preview { flex: none; overflow: visible; }
  }
</style>
