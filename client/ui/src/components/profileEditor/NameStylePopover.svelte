<script lang="ts">
  // The Name style popover (#727): the font as "Gg" tiles in each font, the effect as tiles drawn on your own name, and
  // the colour as solid and gradient swatches plus a custom one, with you on your card, in chat and in the channel list
  // beside them. It writes the editor's draft as you pick (the preview is the real draft); Cancel puts font, colour and
  // effect back as they were on open. Effects are drawn still here, as in the preview (nothing moves).
  import { onDestroy, untrack } from 'svelte'
  import EditorPopover from './EditorPopover.svelte'
  import YouPreview from './YouPreview.svelte'
  import Icon from '../Icon.svelte'
  import Name from '../Name.svelte'
  import Segmented from '../Segmented.svelte'
  import { NAME_EFFECTS } from '../../lib/nameEffects'
  import { NAME_FONTS } from '../../lib/nameFonts'
  import { lightenForName, NAME_GRADIENTS, NAME_SOLIDS, type NamePreset } from '../../lib/nameStyle'
  import { perFrame } from '../../lib/perFrame'
  import { previewUser } from '../../lib/profileDraft'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { app } from '../../lib/store.svelte'

  let { state: st, onClose }: { state: ProfileEditorState; onClose: () => void } = $props()

  const before = untrack(() => ({ font: st.draft.font, colors: [...st.draft.colors], effect: st.draft.effect }))
  // a native colour picker fires `input` many times a frame: one write per frame, `change` writes at once
  const pickers = [0, 1].map((i) => perFrame((v: string) => i < st.draft.colors.length && (st.draft.colors[i] = v)))
  const stopPickers = () => pickers.forEach((p) => p.cancel())
  onDestroy(stopPickers)

  const me = $derived(app.state.me)
  const pu = $derived(me ? previewUser(me, st.draft) : null)
  const same = (p: NamePreset) => p.colors.join() === st.draft.colors.join()
  let customOpen = $state(false)
  const colourKey = $derived.by(() => {
    if (customOpen) return 'custom'
    if (st.draft.colors.length === 0) return 'default'
    const solid = NAME_SOLIDS.find(same)
    if (solid) return `solid:${solid.key}`
    const gradient = NAME_GRADIENTS.find(same)
    return gradient ? `gradient:${gradient.key}` : 'custom'
  })
  const lightened = $derived(colourKey === 'custom' && st.draft.colors.some((c) => lightenForName(c) !== c.toLowerCase()))
  const fontKnown = $derived(st.draft.font === null || NAME_FONTS.some((f) => f.key === st.draft.font))
  const effectKnown = $derived(st.draft.effect === null || NAME_EFFECTS.some((e) => e.key === st.draft.effect))

  function setColours(colors: string[], custom = false) {
    stopPickers()
    customOpen = custom
    st.draft.colors = [...colors]
  }
  function setCustomCount(kind: 'solid' | 'gradient') {
    const [a, b] = st.draft.colors
    setColours(kind === 'solid' ? [a] : [a, b ?? a], true)
  }
  function cancel() {
    stopPickers()
    st.draft.font = before.font
    st.draft.colors = [...before.colors]
    st.draft.effect = before.effect
    onClose()
  }
  function apply() {
    pickers.forEach((p) => p.flush())
    onClose()
  }
  /** Arrow keys move along a group's radios, as a radio group does. */
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
  {#if me && pu}
    <div class="group">
      <h3 id="ns-font">Font</h3>
      <div class="grid" role="radiogroup" aria-label="Name font" tabindex="-1" onkeydown={arrows}>
        <button type="button" role="radio" class="tile" aria-label="Default" aria-checked={st.draft.font === null} tabindex={fontKnown ? (st.draft.font === null ? 0 : -1) : 0} onclick={() => (st.draft.font = null)}>
          <Name userId={me.id} serverId={null} user={{ ...pu, style: { ...pu.style, font: null, effect: null } }} text="Gg" interactive={false} />
        </button>
        {#each NAME_FONTS as f (f.key)}
          <button type="button" role="radio" class="tile" aria-label={f.label} aria-checked={st.draft.font === f.key} tabindex={st.draft.font === f.key ? 0 : -1} onclick={() => (st.draft.font = f.key)}>
            <Name userId={me.id} serverId={null} user={{ ...pu, style: { ...pu.style, font: f.key, effect: null } }} text="Gg" interactive={false} />
          </button>
        {/each}
      </div>
    </div>

    <div class="group">
      <h3 id="ns-effect">Effect</h3>
      <div class="grid" role="radiogroup" aria-label="Name effect" tabindex="-1" onkeydown={arrows}>
        <button type="button" role="radio" class="tile" aria-label="None" aria-checked={st.draft.effect === null} tabindex={effectKnown ? (st.draft.effect === null ? 0 : -1) : 0} onclick={() => (st.draft.effect = null)}>
          <span class="none-text">None</span>
        </button>
        {#each NAME_EFFECTS as e (e.key)}
          <button type="button" role="radio" class="tile" aria-label={e.label} data-tip={e.moves} aria-checked={st.draft.effect === e.key} tabindex={st.draft.effect === e.key ? 0 : -1} onclick={() => (st.draft.effect = e.key)}>
            <Name userId={me.id} serverId={null} user={{ ...pu, style: { ...pu.style, effect: e.key } }} text={e.label} interactive={false} />
          </button>
        {/each}
      </div>
    </div>

    <div class="group">
      <h3 id="ns-colour">Colour</h3>
      <div class="swatches" role="radiogroup" aria-label="Name colour" tabindex="-1" onkeydown={arrows}>
        <button type="button" role="radio" class="swatch none" aria-label="Default" data-tip="Default colour" aria-checked={colourKey === 'default'} tabindex={colourKey === 'default' ? 0 : -1} onclick={() => setColours([])}></button>
        {#each NAME_SOLIDS as p (p.key)}
          <button type="button" role="radio" class="swatch" aria-label={p.label} data-tip={p.label} aria-checked={colourKey === `solid:${p.key}`} tabindex={colourKey === `solid:${p.key}` ? 0 : -1}
            style:background={p.colors[0]} onclick={() => setColours(p.colors)}></button>
        {/each}
        {#each NAME_GRADIENTS as p (p.key)}
          <button type="button" role="radio" class="swatch" aria-label="{p.label} gradient" data-tip={p.label} aria-checked={colourKey === `gradient:${p.key}`} tabindex={colourKey === `gradient:${p.key}` ? 0 : -1}
            style:background="linear-gradient(135deg, {p.colors[0]}, {p.colors[1]})" onclick={() => setColours(p.colors)}></button>
        {/each}
        <button type="button" role="radio" class="swatch pick" aria-label="Custom" data-tip="Custom colour" aria-checked={colourKey === 'custom'} tabindex={colourKey === 'custom' ? 0 : -1}
          onclick={() => setColours(st.draft.colors.length ? st.draft.colors : ['#ffffff'], true)}><Icon name="plus" size={14} /></button>
      </div>
      {#if colourKey === 'custom'}
        <div class="custom">
          <Segmented label="Custom colour" value={st.draft.colors.length >= 2 ? 'gradient' : 'solid'} onchange={setCustomCount}
            options={[{ value: 'solid', label: 'Solid' }, { value: 'gradient', label: 'Gradient' }]} />
          <div class="pickers">
            {#each st.draft.colors as c, i}
              <input type="color" aria-label={st.draft.colors.length >= 2 ? (i === 0 ? 'First colour' : 'Second colour') : 'Colour'} value={c}
                oninput={(e) => pickers[i].push(e.currentTarget.value)} onchange={() => pickers[i].flush()} />
            {/each}
          </div>
          {#if lightened}<p class="hint">Lightened so it reads on dark backgrounds</p>{/if}
        </div>
      {/if}
    </div>
  {/if}
{/snippet}
{#snippet preview()}<YouPreview state={st} />{/snippet}

<EditorPopover title="Name style" onCancel={cancel} onApply={apply} {tiles} {preview} />

<style>
  .group { display: flex; flex-direction: column; gap: 8px; min-width: 0; margin-bottom: 20px; }
  .group:last-child { margin-bottom: 0; }
  h3 { margin: 0; font-size: 12px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 8px; min-width: 0; }
  .tile { display: grid; place-items: center; min-width: 0; height: 68px; padding: 0 8px; overflow: hidden; border: 2px solid transparent; border-radius: var(--radius-lg); background: var(--bg-3); color: var(--text); font: inherit; font-size: 20px; font-weight: 600; cursor: pointer; }
  .tile:hover { background: var(--bg-4); }
  .tile:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .tile[aria-checked='true'] { border-color: var(--focus-ring); }
  .tile :global(.name) { max-width: 100%; }
  .none-text { font-size: 14px; font-weight: 500; color: var(--text-2); }
  .swatches { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; min-width: 0; }
  .swatch { flex: none; width: 32px; height: 32px; padding: 0; border-radius: 50%; border: 2px solid transparent; outline: 1px solid var(--line-strong); outline-offset: -1px; cursor: pointer; }
  .swatch:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .swatch[aria-checked='true'] { outline: none; border-color: var(--text); box-shadow: 0 0 0 2px var(--focus-ring) inset; }
  .swatch.none { border-color: var(--bg-4); background: linear-gradient(135deg, transparent calc(50% - 1px), var(--text-3) calc(50% - 1px) calc(50% + 1px), transparent calc(50% + 1px)); }
  .swatch.pick { border-color: var(--bg-4); background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; }
  .swatch.pick:hover { color: var(--text); }
  .swatch.none[aria-checked='true'], .swatch.pick[aria-checked='true'] { border-color: var(--text); }
  .custom { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; margin-top: 4px; min-width: 0; }
  .pickers { display: flex; gap: 8px; }
  .hint { flex-basis: 100%; margin: 0; font-size: 12px; color: var(--text-3); }
  input[type='color'] { width: 44px; height: 32px; padding: 0; border: 1px solid var(--line-strong); border-radius: var(--radius-sm); background: transparent; cursor: pointer; overflow: hidden; }
  input[type='color']::-webkit-color-swatch-wrapper { padding: 0; }
  input[type='color']::-webkit-color-swatch { border: 0; border-radius: 0; }
  input[type='color']::-moz-color-swatch { border: 0; border-radius: 0; }
</style>
