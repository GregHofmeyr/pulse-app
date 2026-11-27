<script lang="ts">
  // The Change / Remove menu of a profile picture (#727), with what Change leads to: the file picker, the crop dialog,
  // then the upload into the editor's draft (nothing saves until Save). Remove clears the picture, and only shows when
  // there is one. The card's pencils and the avatar and banner tiles all use it.
  //
  // Without a `trigger` it draws the card's own: a click-through layer over the picture (the picture itself opens the
  // menu) and a pencil in its corner, `.pencil.avatar` bottom right, `.pencil.banner` top right, and a spinner over the
  // picture while it uploads. The menu is placed against the nearest positioned ancestor, so a custom trigger sits in
  // one (`position: relative`).
  //
  // The open menu is a nested layer of the editor: it carries `data-editor-layer` (an outside click closes the menu,
  // not the editor) and takes Escape first, marked handled, so the editor's close guard leaves it alone.
  import { tick, type Snippet } from 'svelte'
  import CropDialog from '../CropDialog.svelte'
  import Icon from '../Icon.svelte'
  import { clickOutside, portal } from '../../lib/actions'
  import type { CropRect } from '../../lib/crop'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'

  type Kind = 'avatar' | 'banner'
  let { kind, state: st, trigger }: {
    kind: Kind
    state: ProfileEditorState
    /** Your own trigger: wire `toggle` to a button with `data-menu-toggle` and `aria-haspopup="menu"`. */
    trigger?: Snippet<[{ toggle: () => void; open: boolean; busy: boolean }]>
  } = $props()

  const ACCEPT = 'image/png,image/jpeg,image/gif,image/webp'
  const label = $derived(kind === 'avatar' ? 'avatar' : 'banner')
  const has = $derived(!!(kind === 'avatar' ? st.draft.avatarHash : st.draft.bannerHash))
  const busy = $derived(st.uploading[kind])

  let open = $state(false)
  let input = $state<HTMLInputElement>()
  let list = $state<HTMLElement>()
  let cropping = $state<File | null>(null)
  let opener: HTMLElement | null = null

  function toggle() {
    if (open) return void close()
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
    open = true
    void tick().then(() => list?.querySelector<HTMLElement>('[role=menuitem]')?.focus())
  }
  function close(refocus = false) {
    open = false
    if (refocus) opener?.focus()
  }

  // Escape in the capture phase: ahead of the editor's own Escape, which then sees it handled.
  $effect(() => {
    if (!open) return
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      close(true)
    }
    window.addEventListener('keydown', key, true)
    return () => window.removeEventListener('keydown', key, true)
  })

  function arrows(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const all = [...(list?.querySelectorAll<HTMLElement>('[role=menuitem]') ?? [])]
    const at = all.indexOf(document.activeElement as HTMLElement)
    all[(at + (e.key === 'ArrowDown' ? 1 : all.length - 1)) % all.length]?.focus()
  }

  function change() {
    close()
    input?.click()
  }
  function picked(e: Event & { currentTarget: HTMLInputElement }) {
    const file = e.currentTarget.files?.[0]
    e.currentTarget.value = '' // the same file again is a new pick
    if (file) cropping = file
  }
  function apply(crop: CropRect) {
    const file = cropping
    cropping = null
    if (file) void st.upload(kind, file, crop)
  }
  function remove() {
    close()
    st.remove(kind)
  }
</script>

<span class="im">
  {#if trigger}
    {@render trigger({ toggle, open, busy })}
  {:else}
    <button type="button" class="hit {kind}" tabindex="-1" aria-hidden="true" data-menu-toggle onclick={toggle}></button>
    <button type="button" class="pencil {kind} icon-btn icon-btn-sm icon-btn-overlay round" aria-label="Edit {label}" aria-haspopup="menu" aria-expanded={open}
      data-menu-toggle onclick={toggle}><Icon name="pencil" size={16} /></button>
    {#if busy}<span class="busy {kind}" role="status" aria-label="Uploading {label}"><span class="spinner"></span></span>{/if}
  {/if}
  {#if open}
    <div bind:this={list} class="menu menu-surface pop-in {kind}" role="menu" aria-label="{kind === 'avatar' ? 'Avatar' : 'Banner'} options" tabindex="-1" data-editor-layer
      use:clickOutside={() => close()} onkeydown={arrows}>
      <button type="button" role="menuitem" class="menu-item" onclick={change}>Change</button>
      {#if has}<button type="button" role="menuitem" class="menu-item" onclick={remove}>Remove</button>{/if}
    </div>
  {/if}
  <input bind:this={input} class="file" type="file" accept={ACCEPT} data-kind={kind} aria-label="Choose {label}" tabindex="-1" onchange={picked} />
  {#if cropping}
    <div class="crop" use:portal>
      {#key cropping}<CropDialog file={cropping} {kind} onApply={apply} onCancel={() => (cropping = null)} />{/key}
    </div>
  {/if}
</span>

<style>
  .im { display: contents; }
  /* the picture itself opens the menu; the veil and dashed edge say it can be changed */
  .hit { position: absolute; z-index: 1; border: 0; padding: 0; background: transparent; cursor: pointer; }
  .hit.banner { inset: 0; }
  .hit.avatar { inset: 4px; border-radius: 50%; }
  .hit:hover { background: var(--scrim); outline: 2px dashed var(--on-video); outline-offset: -4px; }
  .pencil { position: absolute; z-index: 2; }
  .pencil.banner { top: 8px; right: 8px; }
  .pencil.avatar { right: 0; bottom: 0; }
  .busy { position: absolute; z-index: 3; display: grid; place-items: center; background: var(--scrim); pointer-events: none; }
  .busy.banner { inset: 0; }
  .busy.avatar { inset: 4px; border-radius: 50%; }
  .spinner { width: 24px; height: 24px; border-radius: 50%; border: 3px solid var(--on-video); border-top-color: transparent; animation: spin .8s linear infinite; }
  /* a slower turn, not a stop, under reduced motion: it still shows something is happening (app.css leaves .spinner out) */
  @media (prefers-reduced-motion: reduce) { .spinner { animation-duration: 2.4s; } }
  @keyframes spin { to { transform: rotate(360deg); } }
  .menu { position: absolute; z-index: 5; min-width: 120px; }
  .menu.banner { top: 44px; right: 8px; }
  .menu.avatar { top: 8px; left: calc(100% + 4px); }
  .menu-item { display: block; width: 100%; padding: 6px 12px; border: 0; border-radius: var(--radius-sm); background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .menu-item:hover, .menu-item:focus-visible { background: var(--bg-4); }
  .file { position: absolute; width: 1px; height: 1px; opacity: 0; overflow: hidden; pointer-events: none; }
</style>
