<script lang="ts">
  // The Looks section's right column (#727): picture tiles in four short groups. A tile's face is the current choice
  // as a picture, not a label: your avatar and banner, your name in its style, the theme as a miniature card, the glow
  // on or off, your ring on a sample avatar, the ring colour. Avatar and banner open the card's own Change / Remove menu
  // (ImageMenu); Glow toggles; Theme opens its popover. The Name style and Speaking ring tiles open theirs once those
  // popovers exist (`open` holds which); until then they only highlight.
  import Avatar from '../Avatar.svelte'
  import Name from '../Name.svelte'
  import SpeakingRing from '../SpeakingRing.svelte'
  import CardThemePopover from './CardThemePopover.svelte'
  import ImageMenu from './ImageMenu.svelte'
  import MiniCard from './MiniCard.svelte'
  import { cardVars } from '../../lib/cardTheme'
  import { mediaUrl } from '../../lib/emoji'
  import { previewUser } from '../../lib/profileDraft'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { RING_COLORS, speakCss } from '../../lib/speakingRing'
  import { nameBg } from '../../lib/theme.svelte'
  import { app } from '../../lib/store.svelte'

  let { state: st }: { state: ProfileEditorState } = $props()

  type Popover = 'name' | 'theme' | 'ring'
  let open = $state<Popover | null>(null)
  const me = $derived(app.state.me)
  const pu = $derived(me ? previewUser(me, st.draft) : null)
  const colour = $derived(pu ? speakCss(pu, st.draft.ringColor, nameBg('side')) : 'var(--speak-green)')
  const colourLabel = $derived(RING_COLORS.find((c) => c.key === st.draft.ringColor)?.label ?? 'Colour')
  const RING_D = 36
  const glowVars = $derived(Object.entries(cardVars(st.draft.themeTop, st.draft.themeBottom)).map(([k, v]) => `${k}: ${v}`).join('; '))
</script>

{#snippet avatarTrigger({ toggle, open: on, busy }: { toggle: () => void; open: boolean; busy: boolean })}
  <button type="button" class="tile square" aria-label="Avatar" aria-haspopup="menu" aria-expanded={on} aria-busy={busy} data-menu-toggle onclick={toggle}>
    {#if pu && me}<Avatar id={me.id} name={pu.display_name} user={pu} size={56} />{/if}
  </button>
{/snippet}
{#snippet bannerTrigger({ toggle, open: on, busy }: { toggle: () => void; open: boolean; busy: boolean })}
  <button type="button" class="tile banner" aria-label="Banner" aria-haspopup="menu" aria-expanded={on} aria-busy={busy} data-menu-toggle onclick={toggle}>
    {#if st.draft.bannerHash}<img src={mediaUrl(st.draft.bannerStill ?? st.draft.bannerHash)} alt="" draggable="false" />{:else}<span class="none"></span>{/if}
  </button>
{/snippet}

<div class="tiles">
  <section aria-labelledby="lt-pictures">
    <h3 id="lt-pictures">Avatar &amp; banner</h3>
    <div class="row">
      <div class="cell"><ImageMenu kind="avatar" state={st} trigger={avatarTrigger} /></div>
      <div class="cell wide"><ImageMenu kind="banner" state={st} trigger={bannerTrigger} /></div>
    </div>
  </section>

  <section aria-labelledby="lt-name">
    <h3 id="lt-name">Name</h3>
    <div class="row">
      <button type="button" class="tile wide" aria-label="Name style" aria-haspopup="dialog" onclick={() => (open = 'name')}>
        {#if me && pu}<span class="face-name"><Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} /></span>{/if}
      </button>
    </div>
  </section>

  <section aria-labelledby="lt-card">
    <h3 id="lt-card">Card</h3>
    <div class="row">
      <button type="button" class="tile mini" aria-label="Card theme" aria-haspopup="dialog" onclick={() => (open = 'theme')}>
        <MiniCard top={st.draft.themeTop} bottom={st.draft.themeBottom} />
      </button>
      <button type="button" role="switch" class="tile mini" aria-label="Glow" aria-checked={st.draft.glow} onclick={() => (st.draft.glow = !st.draft.glow)}>
        <span class="glow-demo" class:on={st.draft.glow} style={glowVars}><MiniCard top={st.draft.themeTop} bottom={st.draft.themeBottom} /></span>
      </button>
    </div>
  </section>

  <section aria-labelledby="lt-ring">
    <h3 id="lt-ring">Speaking ring</h3>
    <div class="row">
      <button type="button" class="tile square" aria-label="Ring style" aria-haspopup="dialog" onclick={() => (open = 'ring')}>
        {#if me && pu}
          <span class="ring-stage" style:--d="{RING_D}px">
            <Avatar id={me.id} name={pu.display_name} user={pu} size={RING_D} />
            <SpeakingRing userId={me.id} d={RING_D} speaking place="list" style={st.draft.ringStyle} color={st.draft.ringColor} user={pu} />
          </span>
        {/if}
      </button>
      <button type="button" class="tile square" aria-label="Ring colour" aria-haspopup="dialog" onclick={() => (open = 'ring')}>
        <span class="swatch" style:--swatch={colour} title={colourLabel}></span>
      </button>
    </div>
  </section>
</div>

{#if open === 'theme'}<CardThemePopover state={st} onClose={() => (open = null)} />{/if}

<style>
  .tiles { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  section { display: flex; flex-direction: column; gap: 8px; min-width: 0; padding-bottom: 16px; border-bottom: 1px solid var(--line-soft); }
  section:last-child { border-bottom: 0; padding-bottom: 0; }
  h3 { margin: 0; font-size: 12px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .row { display: flex; gap: 8px; min-width: 0; }
  .cell { position: relative; flex: 0 0 auto; min-width: 0; }
  .cell.wide { flex: 1 1 0; }
  .tile { position: relative; display: grid; place-items: center; min-width: 0; height: 76px; padding: 0; overflow: hidden; border: 2px solid transparent; border-radius: var(--radius-lg); background: var(--bg-3); color: var(--text); font: inherit; cursor: pointer; }
  .tile:hover { background: var(--bg-4); }
  .tile:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .tile.square { flex: 0 0 76px; aspect-ratio: 1; }
  .row > .tile { flex: 1 1 0; }
  .row > .tile.square { flex: 0 0 76px; }
  .cell > .tile { width: 100%; }
  .cell > .tile.square { width: 76px; }
  .tile.wide, .tile.banner { width: 100%; }
  .tile.mini { padding: 8px; overflow: visible; }
  .tile.mini :global(.mc) { border-radius: var(--radius-md); }
  .tile[aria-expanded='true'] { border-color: var(--accent); }
  .tile img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .none { width: 100%; height: 100%; background: repeating-linear-gradient(135deg, var(--bg-4) 0 6px, var(--bg-3) 6px 12px); }
  .face-name { max-width: 100%; padding: 0 12px; box-sizing: border-box; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; font-size: 16px; }
  /* the glow, drawn from the card's own colours, when on; a flat card when off */
  .glow-demo { display: block; width: 100%; height: 100%; border-radius: var(--radius-md); }
  .glow-demo.on { box-shadow: 0 0 8px 1px var(--glow-top-a), 0 2px 12px 2px var(--glow-bottom-a); }
  .tile.mini[aria-checked='false'] .glow-demo { filter: saturate(.4); opacity: .7; }
  .ring-stage { position: relative; display: block; width: var(--d); height: var(--d); }
  .swatch { width: 36px; height: 36px; border-radius: 50%; background: var(--swatch); box-shadow: inset 0 0 0 1px var(--line-strong); }
</style>
