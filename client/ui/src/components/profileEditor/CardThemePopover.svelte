<script lang="ts">
  // The Card theme popover (#727): the preset themes as miniature cards in their own colours, and a custom pair (a top
  // and a bottom colour), with you on your card, in chat and in the channel list beside them. It writes the editor's
  // draft as you pick (the preview is the real draft) and Cancel puts the two colours back as they were on open.
  import { onDestroy, untrack } from 'svelte'
  import EditorPopover from './EditorPopover.svelte'
  import Icon from '../Icon.svelte'
  import MiniCard from './MiniCard.svelte'
  import YouPreview from './YouPreview.svelte'
  import { presetFor, THEME_PRESETS } from '../../lib/cardTheme'
  import { perFrame } from '../../lib/perFrame'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'

  let { state: st, onClose }: { state: ProfileEditorState; onClose: () => void } = $props()

  const before = untrack(() => ({ top: st.draft.themeTop, bottom: st.draft.themeBottom }))
  // a native colour picker fires `input` many times a frame: one write per frame, `change` writes at once
  const pickers = {
    top: perFrame((v: string) => (st.draft.themeTop = v)),
    bottom: perFrame((v: string) => (st.draft.themeBottom = v)),
  }
  onDestroy(() => {
    pickers.top.cancel()
    pickers.bottom.cancel()
  })

  const key = $derived(presetFor(st.draft.themeTop, st.draft.themeBottom))
  /** The custom pair's pickers are shown: you opened them, or the theme is already none of the presets. */
  let customOpen = $state(presetFor(before.top, before.bottom) === null)
  const custom = $derived(customOpen || key === null)

  function preset(top: string, bottom: string) {
    pickers.top.cancel()
    pickers.bottom.cancel()
    customOpen = false
    st.draft.themeTop = top
    st.draft.themeBottom = bottom
  }
  function cancel() {
    pickers.top.cancel()
    pickers.bottom.cancel()
    st.draft.themeTop = before.top
    st.draft.themeBottom = before.bottom
    onClose()
  }
  function apply() {
    pickers.top.flush()
    pickers.bottom.flush()
    onClose()
  }
  /** Arrow keys move along the radios, as a radio group does. */
  function arrows(e: KeyboardEvent & { currentTarget: HTMLElement }) {
    if (!['ArrowRight', 'ArrowLeft', 'ArrowDown', 'ArrowUp'].includes(e.key)) return
    const all = [...e.currentTarget.querySelectorAll<HTMLElement>('[role=radio]')]
    const at = all.indexOf(document.activeElement as HTMLElement)
    if (at < 0) return
    e.preventDefault()
    const to = all[(at + (e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : all.length - 1)) % all.length]
    to.focus()
    to.click()
  }
</script>

{#snippet tiles()}
  <div class="grid" role="radiogroup" aria-label="Card theme" tabindex="-1" onkeydown={arrows}>
    {#each THEME_PRESETS as p (p.key)}
      <button type="button" role="radio" class="tile" aria-label={p.label} aria-checked={!custom && key === p.key} tabindex={!custom && key === p.key ? 0 : -1}
        onclick={() => preset(p.top, p.bottom)}>
        <MiniCard top={p.top} bottom={p.bottom} /><span class="name">{p.label}</span>
      </button>
    {/each}
    <button type="button" role="radio" class="tile" aria-label="Custom" aria-checked={custom} tabindex={custom ? 0 : -1} onclick={() => (customOpen = true)}>
      {#if key === null}<MiniCard top={st.draft.themeTop} bottom={st.draft.themeBottom} />{:else}<span class="plus"><Icon name="plus" size={20} /></span>{/if}
      <span class="name">Custom</span>
    </button>
  </div>
  {#if custom}
    <div class="pickers">
      <label class="pick">
        <input type="color" aria-label="Top colour" value={st.draft.themeTop} oninput={(e) => pickers.top.push(e.currentTarget.value)} onchange={() => pickers.top.flush()} />
        <span>Top <code>{st.draft.themeTop}</code></span>
      </label>
      <label class="pick">
        <input type="color" aria-label="Bottom colour" value={st.draft.themeBottom} oninput={(e) => pickers.bottom.push(e.currentTarget.value)} onchange={() => pickers.bottom.flush()} />
        <span>Bottom <code>{st.draft.themeBottom}</code></span>
      </label>
    </div>
  {/if}
{/snippet}

<EditorPopover title="Card theme" onCancel={cancel} onApply={apply} {tiles} preview={preview} />
{#snippet preview()}<YouPreview state={st} />{/snippet}

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(112px, 1fr)); gap: 12px; }
  .tile { position: relative; display: flex; flex-direction: column; gap: 6px; min-width: 0; padding: 6px; border: 2px solid transparent; border-radius: var(--radius-lg); background: var(--bg-3); color: var(--text); font: inherit; font-size: 13px; cursor: pointer; }
  .tile:hover { background: var(--bg-4); }
  .tile:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .tile[aria-checked='true'] { border-color: var(--focus-ring); }
  .tile > :global(.mc), .plus { flex: none; height: 84px; border-radius: var(--radius-md); }
  .plus { display: grid; place-items: center; border: 1px dashed var(--line-strong); color: var(--text-2); }
  .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: center; }
  .pickers { display: flex; flex-wrap: wrap; gap: 16px; margin-top: 16px; }
  .pick { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); cursor: pointer; }
  .pick code { font-size: 12px; color: var(--text-3); }
  input[type='color'] { width: 44px; height: 32px; padding: 0; border: 1px solid var(--line-strong); border-radius: var(--radius-sm); background: transparent; cursor: pointer; overflow: hidden; }
  input[type='color']::-webkit-color-swatch-wrapper { padding: 0; }
  input[type='color']::-webkit-color-swatch { border: 0; border-radius: 0; }
  input[type='color']::-moz-color-swatch { border: 0; border-radius: 0; }
</style>
