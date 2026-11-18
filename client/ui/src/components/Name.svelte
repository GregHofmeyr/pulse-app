<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { appearance } from '../lib/appearance.svelte'
  import { displayName, roleColorFor, userOf } from '../lib/selectors'
  import { nameCss, resolveNameStyle } from '../lib/nameStyle'
  import { letters, popColors } from '../lib/nameEffects'
  import { openCard } from '../lib/profileCard.svelte'
  import { nameOpts, type NameSurface } from '../lib/theme.svelte'

  import type { User } from '../lib/protocol/User'

  let { userId, serverId, text, interactive = true, size = 'normal', user, surface = 'chat', bg, play = false }: {
    userId: string
    serverId: string | null
    text?: string
    interactive?: boolean
    size?: 'normal' | 'card'
    /** What it sits on, so it's made readable there (#42): the chat and dialogs, the sidebar and member lists, or a menu (a profile card passes `bg`, its own colour). */
    surface?: NameSurface
    /** The exact colour it sits on, instead of `surface` (a profile card's own surface, #139). */
    bg?: string
    /** Style the name as this user instead of the stored one (the profile editor's preview). */
    user?: User
    /** Its name effect plays (#468): in the profile editor and while a full profile is open. Elsewhere it is still,
     *  and plays only while the name is hovered. */
    play?: boolean
  } = $props()

  const resolved = $derived(resolveNameStyle(user ?? userOf(app.state, userId), { roleColor: roleColorFor(app.state, serverId, userId), showStyles: appearance.showNameStyles, ...nameOpts(surface), ...(bg ? { bg } : {}) }))
  const css = $derived(nameCss(resolved, true))
  const fx = $derived(resolved.effect?.key ?? null)
  const label = $derived(text ?? displayName(app.state, serverId, userId))
  // Pop bounces letter by letter; a gradient name gives each letter its own colour along the gradient
  const popLetters = $derived(fx === 'pop' ? letters(label) : [])
  const popInk = $derived(fx === 'pop' ? popColors(resolved.paint, popLetters.length) : [])
</script>

{#snippet content()}
  {#if fx === 'pop'}
    <span class="sr-only">{label}</span><span class="fx-letters" aria-hidden="true">{#each popLetters as l, i}<span class="fx-l" style:--i={i} style:color={popInk[i]}>{l}</span>{/each}</span>
  {:else}{label}{/if}
{/snippet}

{#if interactive}
  <button type="button" class="name" class:card={size === 'card'} class:fx-play={play} data-fx={fx} style={css} onclick={(e) => openCard(userId, serverId, e.currentTarget)}>{@render content()}</button>
{:else}
  <span class="name" class:card={size === 'card'} class:fx-play={play} data-fx={fx} style={css}>{@render content()}</span>
{/if}

<style>
  /* The font scale evens out the fonts; the line height stays at the parent's, so rows keep their height.
     fit-content: a stretching parent (a flex column) would widen the box, and a gradient spans the box, not the text. */
  .name { display: inline-block; width: fit-content; font-size: calc(1em * var(--name-scale, 1)); line-height: calc(1.4em / var(--name-scale, 1)); vertical-align: bottom; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* line height in the card's own em: still the parent's 1.4 */
  .name.card { font-size: calc(1.15em * var(--name-scale, 1)); line-height: calc(1.4em / (1.15 * var(--name-scale, 1))); }
  button.name { background: none; border: 0; padding: 0; margin: 0; font-family: inherit; font-weight: inherit; color: inherit; cursor: pointer; text-align: left; }
  button.name:hover { text-decoration: underline; }

  /* ---- Name effects (#468): a still look everywhere; the moving one plays on hover, or with .fx-play (the editor,
     a full profile). The colours come from lib/nameEffects.ts as --fx-*: the name's own colour where it has one,
     else the text colour it sits in. ---- */
  /* glows, strokes and bounces reach past the text: clipped a little further out than the box, never cut at it */
  .name[data-fx] { position: relative; overflow: clip; overflow-clip-margin: .35em; }
  button.name[data-fx]:hover { text-decoration: none; }

  .name[data-fx='neon'] { text-shadow: 0 0 1px var(--fx-hi), 0 0 5px var(--fx-a, currentColor), 0 0 12px var(--fx-glow, currentColor); }
  .name[data-fx='neon']:hover, .name[data-fx='neon'].fx-play { animation: fx-neon 1.6s ease-in-out infinite alternate; }
  @keyframes fx-neon {
    from { text-shadow: 0 0 1px var(--fx-hi), 0 0 3px var(--fx-a, currentColor), 0 0 7px var(--fx-glow, currentColor); }
    to { text-shadow: 0 0 2px var(--fx-hi), 0 0 8px var(--fx-a, currentColor), 0 0 18px var(--fx-glow, currentColor); }
  }

  .name[data-fx='toon'] { -webkit-text-stroke: .05em var(--fx-dark, var(--bg-0)); paint-order: stroke fill; text-shadow: .08em .08em 0 var(--fx-b, var(--bg-0)); }
  .name[data-fx='toon']:hover, .name[data-fx='toon'].fx-play { animation: fx-toon .9s steps(2) infinite; }
  @keyframes fx-toon {
    from { text-shadow: .08em .08em 0 var(--fx-b, var(--bg-0)); transform: rotate(-1.5deg); }
    to { text-shadow: -.08em .08em 0 var(--fx-b, var(--bg-0)); transform: rotate(1.5deg); }
  }

  .fx-l { display: inline-block; }
  .name[data-fx='pop'] { text-shadow: 0 .1em 0 var(--fx-dark, var(--bg-0)); }
  .name[data-fx='pop']:hover .fx-l, .name[data-fx='pop'].fx-play .fx-l { animation: fx-pop .9s cubic-bezier(.3, 1.6, .5, 1) infinite; animation-delay: calc(var(--i) * 60ms); }
  @keyframes fx-pop { 0%, 60%, 100% { transform: translateY(0); } 30% { transform: translateY(-.22em); } }

  .name[data-fx='gummy'] { filter: drop-shadow(0 .07em 0 var(--fx-dark, var(--bg-0))); }
  .name[data-fx='gummy']:hover, .name[data-fx='gummy'].fx-play { animation: fx-gummy 1.1s ease-in-out infinite; }
  @keyframes fx-gummy { 0%, 100% { transform: scale(1, 1); } 30% { transform: scale(1.06, .92); } 55% { transform: scale(.96, 1.05); } 75% { transform: scale(1.02, .98); } }

  .name[data-fx='prism'] { background-position: 0% 50%; }
  .name[data-fx='prism']:hover, .name[data-fx='prism'].fx-play { animation: fx-prism 2.4s linear infinite; }
  @keyframes fx-prism { to { background-position: 300% 50%; } }

  .name[data-fx='shimmer'] { background-position: 100% 50%; }
  .name[data-fx='shimmer']:hover, .name[data-fx='shimmer'].fx-play { animation: fx-shimmer 1.8s ease-in-out infinite; }
  @keyframes fx-shimmer { to { background-position: -150% 50%; } }

  .name[data-fx='glitch'] { text-shadow: .06em 0 var(--fx-g1, var(--accent)), -.06em 0 var(--fx-g2, var(--danger-text)); }
  .name[data-fx='glitch']:hover, .name[data-fx='glitch'].fx-play { animation: fx-glitch .5s steps(3) infinite; }
  @keyframes fx-glitch {
    0% { text-shadow: .06em 0 var(--fx-g1, var(--accent)), -.06em 0 var(--fx-g2, var(--danger-text)); transform: translate(0); }
    33% { text-shadow: -.08em .04em var(--fx-g1, var(--accent)), .08em -.04em var(--fx-g2, var(--danger-text)); transform: translate(1px, -1px); }
    66% { text-shadow: .08em -.04em var(--fx-g1, var(--accent)), -.08em .04em var(--fx-g2, var(--danger-text)); transform: translate(-1px, 1px); }
    100% { transform: translate(0); }
  }

  /* Ember: fire letters with a warm glow, and small charcoal specks of ash that drift up while it plays (so it never
     reads as an orange Neon) */
  .name[data-fx='ember'] { filter: drop-shadow(0 0 4px var(--fx-fire)); }
  .name[data-fx='ember']::after { content: ''; position: absolute; inset: 0; pointer-events: none; opacity: .55;
    background-image: radial-gradient(circle, var(--fx-ash) 0 1px, transparent 1.6px), radial-gradient(circle, var(--fx-ash-2) 0 .8px, transparent 1.4px), radial-gradient(circle, var(--fx-ash) 0 .7px, transparent 1.2px);
    background-size: 1.9em 1.3em, 2.7em 1.6em, 1.3em 1.1em; background-position: .2em .4em, 1.1em .1em, .6em .9em; }
  .name[data-fx='ember']:hover, .name[data-fx='ember'].fx-play { animation: fx-ember 1.3s ease-in-out infinite alternate; }
  .name[data-fx='ember']:hover::after, .name[data-fx='ember'].fx-play::after { animation: fx-ash 2.2s linear infinite; }
  @keyframes fx-ember { from { filter: drop-shadow(0 0 2px var(--fx-fire)) brightness(.95); } to { filter: drop-shadow(0 -.1em 7px var(--fx-fire)) brightness(1.12); } }
  @keyframes fx-ash { from { transform: translateY(.5em); opacity: 0; } 30% { opacity: .7; } to { transform: translateY(-.9em); opacity: 0; } }

  @media (prefers-reduced-motion: reduce) {
    .name[data-fx], .name[data-fx]::after, .name[data-fx] .fx-l { animation: none !important; }
  }
</style>
