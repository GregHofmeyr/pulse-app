<script lang="ts">
  // The Speaking ring popover (#727): every style as a tile drawing that ring on a sample avatar (still), grouped
  // "Still or looping" and "Follows your voice", then the colour row, with a live ring and you on your card, in chat and
  // in the channel list beside them. It writes the editor's draft as you pick; Cancel puts style and colour back as they
  // were on open, Apply only closes. The big ring talks (a fake speech envelope, written straight onto its stage) only
  // for a style that follows your voice, while motion is allowed, the window is focused and the stage is on screen.
  import { untrack } from 'svelte'
  import EditorPopover from './EditorPopover.svelte'
  import YouPreview from './YouPreview.svelte'
  import Avatar from '../Avatar.svelte'
  import SpeakingRing from '../SpeakingRing.svelte'
  import { motion, observe } from '../../lib/animate.svelte'
  import { previewUser } from '../../lib/profileDraft'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { envelopeStep, isReactive, PREVIEW_D, RING_COLORS, RING_STYLES, ringState, speakCss, type RingColor } from '../../lib/speakingRing'
  import { app } from '../../lib/store.svelte'
  import { nameBg } from '../../lib/theme.svelte'
  import { windowFocus } from '../../lib/windowFocus.svelte'

  let { state: st, onClose }: { state: ProfileEditorState; onClose: () => void } = $props()

  const before = untrack(() => ({ style: st.draft.ringStyle, color: st.draft.ringColor }))
  const GROUPS = [
    { key: 'looping', label: 'Still or looping', styles: RING_STYLES.filter((r) => !r.reactive) },
    { key: 'voice', label: 'Follows your voice', styles: RING_STYLES.filter((r) => r.reactive) },
  ]
  /** A style tile's avatar; its ring box is 1.32 times this, so the tile leaves room round it. */
  const TILE_D = 40

  const me = $derived(app.state.me)
  const pu = $derived(me ? previewUser(me, st.draft) : null)
  /** A swatch's colour, as the ring would draw it on this theme (the light themes' White is ink). */
  const swatchCss = (c: RingColor) => speakCss(pu, c, nameBg('side'))
  const profileTip = $derived(st.draft.colors.length ? 'Your name colour, from Name style' : "Your name colour, from Name style. You haven't set one, so it's green.")

  // The big ring as SpeakingRing draws it: still under reduced motion or while you're live.
  const previewRing = $derived(ringState({ lit: true, place: 'preview', style: st.draft.ringStyle, reducedMotion: motion.reducedMotion, streaming: motion.streaming, animating: true }))
  const caption = $derived(previewRing !== 'still' ? 'Preview · talking' : motion.reducedMotion ? 'Preview · still (reduced motion is on)' : "Preview · still (you're live)")
  let stage = $state<HTMLElement>()
  let stageSeen = $state(false)
  $effect(() => {
    const el = stage
    if (!el) return
    return observe(el, (v) => (stageSeen = v))
  })
  const envelopeOn = $derived(previewRing === 'on' && stageSeen && windowFocus.focused && isReactive(st.draft.ringStyle))
  $effect(() => {
    const el = stage
    if (!el || !envelopeOn) return
    let s = { lvl: 0, pause: 0 }
    const id = setInterval(() => {
      s = envelopeStep(s, Math.random())
      el.style.setProperty('--lvl', String(s.lvl))
    }, 70)
    return () => {
      clearInterval(id)
      el.style.removeProperty('--lvl')
    }
  })

  function cancel() {
    st.draft.ringStyle = before.style
    st.draft.ringColor = before.color
    onClose()
  }
  /** Arrow keys move along a group's radios, as a radio group does (one run across both style groups). */
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
    <div class="styles" role="radiogroup" aria-label="Ring style" tabindex="-1" onkeydown={arrows}>
      {#each GROUPS as g (g.key)}
        <div class="group" role="group" aria-label={g.label}>
          <h3>{g.label}</h3>
          <div class="grid">
            {#each g.styles as r (r.key)}
              <button type="button" role="radio" class="tile" aria-label={r.label} aria-checked={st.draft.ringStyle === r.key} tabindex={st.draft.ringStyle === r.key ? 0 : -1} onclick={() => (st.draft.ringStyle = r.key)}>
                <span class="face" style:--d="{TILE_D}px">
                  <Avatar id={me.id} name={pu.display_name} user={pu} size={TILE_D} />
                  <SpeakingRing userId={me.id} d={TILE_D} speaking place="list" own style={r.key} color={st.draft.ringColor} user={pu} />
                </span>
                <span class="label">{r.label}</span>
              </button>
            {/each}
          </div>
        </div>
      {/each}
    </div>

    <div class="group">
      <h3>Colour</h3>
      <div class="swatches" role="radiogroup" aria-label="Ring colour" tabindex="-1" onkeydown={arrows}>
        {#each RING_COLORS as c (c.key)}
          <button type="button" role="radio" class="swatch" aria-label={c.label} data-tip={c.key === 'profile' ? profileTip : c.label}
            aria-checked={st.draft.ringColor === c.key} tabindex={st.draft.ringColor === c.key ? 0 : -1} style:--swatch={swatchCss(c.key)} onclick={() => (st.draft.ringColor = c.key)}></button>
        {/each}
      </div>
    </div>
  {/if}
{/snippet}
{#snippet preview()}
  {#if me && pu}
    <div class="big">
      <div class="ring-stage" bind:this={stage}>
        <Avatar id={me.id} name={pu.display_name} user={pu} size={PREVIEW_D} />
        <SpeakingRing userId={me.id} d={PREVIEW_D} speaking place="preview" style={st.draft.ringStyle} color={st.draft.ringColor} user={pu} />
      </div>
      <p class="caption">{caption}</p>
    </div>
  {/if}
  <YouPreview state={st} />
{/snippet}

<EditorPopover title="Speaking ring" onCancel={cancel} onApply={onClose} {tiles} {preview} />

<style>
  .styles { display: flex; flex-direction: column; gap: 20px; min-width: 0; }
  .group { display: flex; flex-direction: column; gap: 8px; min-width: 0; margin-bottom: 20px; }
  .styles .group { margin-bottom: 0; }
  .group:last-child { margin-bottom: 0; }
  h3 { margin: 0; font-size: 12px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(104px, 1fr)); gap: 8px; min-width: 0; }
  .tile { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; min-width: 0; padding: 8px; overflow: hidden; border: 2px solid transparent; border-radius: var(--radius-lg); background: var(--bg-3); color: var(--text); font: inherit; cursor: pointer; }
  .tile:hover { background: var(--bg-4); }
  .tile:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .tile[aria-checked='true'] { border-color: var(--focus-ring); }
  /* the avatar, with the room its 1.32 D ring box needs (.16 D each side) so the ring never reaches the tile's edge */
  .face { position: relative; flex: none; display: block; width: var(--d); height: var(--d); margin: calc(var(--d) * .16); }
  .label { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; font-weight: 500; color: var(--text-2); }
  .swatches { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; min-width: 0; }
  .swatch { flex: none; width: 32px; height: 32px; padding: 0; border-radius: 50%; border: 2px solid transparent; background: var(--swatch); outline: 1px solid var(--line-strong); outline-offset: -1px; cursor: pointer; }
  .swatch:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .swatch[aria-checked='true'] { outline: none; border-color: var(--text); box-shadow: 0 0 0 2px var(--focus-ring) inset; }
  .big { display: flex; flex-direction: column; align-items: center; gap: 8px; min-width: 0; max-width: 100%; }
  .ring-stage { position: relative; flex: none; width: 96px; height: 96px; margin: calc(96px * .16); isolation: isolate; border-radius: 50%; }
  .caption { margin: 0; max-width: 100%; text-align: center; font-size: 12px; color: var(--text-3); }
</style>
