<script lang="ts" module>
  /** Bars round the ring (§4.11): 44 holders, every other one below 48px. */
  const BARS = 44
  const barPhase = (i: number) => Math.sin(i * 0.9) + 1
  const BAR_LIST = Array.from({ length: BARS }, (_, i) => ({ i, rotate: (i * 360) / BARS, delay: -(barPhase(i) * 0.25), rest: 0.35 + (0.4 * barPhase(i)) / 2 }))
  /** How long a ring takes to fade when it goes off; its level is cleared after (so a soundboard-lit ring starts at .5). */
  const FADE_MS = 250
</script>

<script lang="ts">
  // One component draws every speaking ring (#651 §3.2): an absolutely positioned sibling of the Avatar, exactly the
  // avatar's box, with every size a fraction of --d. Starting or stopping speaking changes only data-ring; the style's
  // layers stay mounted. Levels (--lvl) are written onto the root by lib/ringLevels, never through state.
  import { untrack } from 'svelte'
  import { motion, observe } from '../lib/animate.svelte'
  import { register, setWatching } from '../lib/ringLevels.svelte'
  import { userOf } from '../lib/selectors'
  import { effectiveStyle, ringColorOf, ringState, ringStyleOf, speakCss, watchesLevels, type RingColor, type RingPlace, type RingStyle } from '../lib/speakingRing'
  import { isAnimating, ringPrefs } from '../lib/speakingRing.svelte'
  import { app } from '../lib/store.svelte'
  import { nameBg } from '../lib/theme.svelte'
  import type { User } from '../lib/protocol/User'

  let {
    userId,
    d,
    speaking,
    place,
    style,
    color,
    user: draftUser,
  }: {
    userId: string
    /** The avatar's diameter in px. */
    d: number
    /** Lit: speaking, or their soundboard sound is playing. */
    speaking: boolean
    place: RingPlace
    /** The preview's draft style and colour (they override the person's own). */
    style?: RingStyle
    color?: RingColor
    /** The preview's draft you, so "Profile colour" follows a name colour not saved yet. */
    user?: User
  } = $props()

  const user = $derived(draftUser ?? userOf(app.state, userId))
  const preview = $derived(place === 'preview')
  const eff = $derived(preview ? (style ?? ringStyleOf(user)) : effectiveStyle(style ?? ringStyleOf(user), { classicForAll: ringPrefs.classic }))
  const css = $derived(speakCss(user, color ?? ringColorOf(user), nameBg('side')))
  const ringNow = $derived(ringState({ lit: speaking, place, style: eff, reducedMotion: motion.reducedMotion, streaming: motion.streaming, animating: preview || isAnimating(userId) }))
  const watches = $derived(watchesLevels(eff, { reducedMotion: motion.reducedMotion, streaming: motion.streaming, place }))
  const small = $derived(d < 48)
  const dots = $derived(small ? 12 : 24)
  const bars = $derived(small ? BAR_LIST.filter((b) => b.i % 2 === 0) : BAR_LIST)

  let box = $state<HTMLElement>()
  let onScreen = $state(false)

  function attach(el: HTMLElement) {
    return untrack(() => {
      box = el
      const unregister = preview ? () => {} : register(userId, el)
      const unobserve = observe(el, (v) => (onScreen = v))
      return () => {
        unobserve()
        unregister()
        setWatching(el, false)
      }
    })
  }

  // ask the core for levels only while this ring is on screen and its effective style follows the voice
  $effect(() => {
    if (box) setWatching(box, onScreen && watches)
  })

  // Going off: the layers fade (CSS), the level eases down with them, then --lvl is cleared.
  let fade: ReturnType<typeof setTimeout> | undefined
  let wasLit = false
  $effect(() => {
    const lit = ringNow !== 'off'
    const el = box
    if (!el) return
    untrack(() => {
      if (lit) {
        clearTimeout(fade)
        fade = undefined
      } else if (wasLit) {
        clearTimeout(fade)
        fade = setTimeout(() => {
          fade = undefined
          el.style.removeProperty('--lvl')
        }, FADE_MS)
      }
      wasLit = lit
    })
  })
  $effect(() => () => clearTimeout(fade))
</script>

<span class="ring-box" data-style={eff} data-ring={ringNow} data-place={place} style:--d="{d}px" style:--speak={css} aria-hidden="true" {@attach attach}>
  {#if eff === 'classic'}
    <span class="layer front classic"></span>
  {:else if eff === 'glow'}
    <span class="layer front glow"></span>
  {:else if eff === 'pulse'}
    <span class="layer front pulse"></span>
  {:else if eff === 'ripple'}
    <span class="layer front ripple r1"></span>
    <span class="layer front ripple r2"></span>
  {:else if eff === 'sweep'}
    <span class="layer front sweep"></span>
  {:else if eff === 'orbit'}
    <span class="layer front orbit">
      {#each [0, 72, 144, 216, 288] as deg (deg)}<span class="holder" style:transform="rotate({deg}deg)"><span class="dot"></span></span>{/each}
    </span>
  {:else if eff === 'halo'}
    <span class="layer back halo"></span>
  {:else if eff === 'viewfinder'}
    <span class="layer front vf">
      <span class="corner tl"></span><span class="corner tr"></span><span class="corner bl"></span><span class="corner br"></span>
    </span>
  {:else if eff === 'liquid'}
    <span class="layer back liquid">
      <span class="blob b1"></span><span class="blob b2"></span><span class="blob b3"></span>
    </span>
  {:else if eff === 'dots'}
    <span class="layer front dots">
      {#each { length: dots }, i (i)}<span class="holder" style:transform="rotate({(i * 360) / dots}deg)"><span class="dot" style:--delay="{-(i / dots)}s"></span></span>{/each}
    </span>
  {:else if eff === 'bars'}
    <span class="layer front bars">
      {#each bars as b (b.i)}<span class="holder" style:transform="rotate({b.rotate}deg)"><span class="bar" style:--delay="{b.delay}s" style:--rest={b.rest}></span></span>{/each}
    </span>
  {/if}
</span>

<style>
  /* Exactly the avatar's box: the §4 insets are relative to it. No z-index, opacity, transform or overflow here, so
     the root neither forms a stacking context nor clips; the host's avatar button is the stacking context. */
  .ring-box { position: absolute; inset: 0; pointer-events: none; }
  .layer { position: absolute; inset: 0; border-radius: 50%; pointer-events: none; box-sizing: border-box; }
  .front { z-index: 1; }
  .back { z-index: -1; }
  .holder { position: absolute; inset: 0; }

  /* Off: hidden and still. Fading out (not Classic, which snaps like today) so a ring never vanishes mid-word. */
  .ring-box[data-ring='off'] .layer { opacity: 0; }
  .ring-box[data-ring='off'] .layer:not(.classic) { transition: opacity .2s; }

  /* Classic (§4.1): today's double ring; 4/7 on the stage, 2/4 on the strip, band and list. Static, no transition. */
  .classic { box-shadow: 0 0 0 2px var(--ring), 0 0 0 4px var(--speak); }
  .ring-box[data-place='stage'] .classic, .ring-box[data-place='preview'] .classic { box-shadow: 0 0 0 4px var(--ring), 0 0 0 7px var(--speak); }

  /* Soft glow (§4.2): painted once, only opacity changes. */
  .glow { inset: calc(var(--d) * -.104); background: radial-gradient(circle closest-side, transparent 73%, var(--speak) 82%, transparent 100%); opacity: .9; }
  .ring-box[data-ring='on'] .glow { transition: opacity .15s; }

  /* Pulse (§4.3) */
  .pulse { inset: calc(var(--d) * -.073); border: max(2px, calc(var(--d) * .031)) solid var(--speak); opacity: .95; }
  .ring-box[data-ring='on'] .pulse { animation: ring-pulse 1.1s ease-in-out infinite; }
  @keyframes ring-pulse { 0%, 100% { transform: scale(1); opacity: .95; } 50% { transform: scale(1.07); opacity: .55; } }

  /* Ripple (§4.4): two sonar rings, the second half a cycle on. */
  .ripple { inset: calc(var(--d) * -.042); border: max(1.5px, calc(var(--d) * .021)) solid var(--speak); opacity: .9; }
  .ring-box[data-ring='still'] .r2 { opacity: 0; }
  .ring-box[data-ring='on'] .ripple { animation: ring-ripple 1.6s ease-out infinite; }
  .ring-box[data-ring='on'] .r2 { animation-delay: -.8s; }
  @keyframes ring-ripple { 0% { transform: scale(1); opacity: .9; } 100% { transform: scale(1.18); opacity: 0; } }

  /* Sweep (§4.5): a static, radially symmetric mask leaves a thin band, and rotates with the layer for free. */
  .sweep {
    inset: calc(var(--d) * -.0625);
    background: conic-gradient(from 0deg, transparent 0 55%, var(--speak) 85%, transparent 100%);
    -webkit-mask: radial-gradient(circle closest-side, transparent 92%, var(--speak) 93%);
    mask: radial-gradient(circle closest-side, transparent 92%, var(--speak) 93%);
  }
  .ring-box[data-ring='still'] .sweep, .ring-box[data-ring='off'] .sweep { background: var(--speak); }
  .ring-box[data-ring='still'] .sweep { opacity: .9; }
  .ring-box[data-ring='on'] .sweep { animation: ring-spin 1.8s linear infinite; }
  @keyframes ring-spin { to { transform: rotate(360deg); } }

  /* Orbit (§4.6): one rotating layer carries five static dots. */
  .orbit { inset: calc(var(--d) * -.125); }
  .orbit .dot { position: absolute; top: 0; left: 50%; width: max(4px, calc(var(--d) * .094)); aspect-ratio: 1; translate: -50% 0; border-radius: 50%; background: var(--speak); }
  .ring-box[data-ring='on'] .orbit { animation: ring-spin 2.6s linear infinite; }

  /* Halo (§4.7): behind the avatar; the soft edge is the gradient, never a blur. */
  .halo { inset: calc(var(--d) * -.15); background: radial-gradient(circle closest-side, var(--speak) 70%, transparent 100%); transform: scale(.96); opacity: .5; }
  .ring-box[data-ring='on'] .halo { animation: ring-halo 1.4s ease-in-out infinite; }
  @keyframes ring-halo { 0%, 100% { transform: scale(.92); opacity: .35; } 50% { transform: scale(1); opacity: .7; } }

  /* Viewfinder (§4.8): four corner brackets, snapping in once each time they start. */
  .vf { inset: calc(var(--d) * -.115); border-radius: 0; }
  .corner { position: absolute; width: calc(var(--d) * .167); height: calc(var(--d) * .167); border: 0 solid var(--speak); }
  .tl { top: 0; left: 0; border-top-width: max(2px, calc(var(--d) * .031)); border-left-width: max(2px, calc(var(--d) * .031)); }
  .tr { top: 0; right: 0; border-top-width: max(2px, calc(var(--d) * .031)); border-right-width: max(2px, calc(var(--d) * .031)); }
  .bl { bottom: 0; left: 0; border-bottom-width: max(2px, calc(var(--d) * .031)); border-left-width: max(2px, calc(var(--d) * .031)); }
  .br { bottom: 0; right: 0; border-bottom-width: max(2px, calc(var(--d) * .031)); border-right-width: max(2px, calc(var(--d) * .031)); }
  .ring-box[data-ring='on'] .vf { animation: ring-snap .18s ease-out; }
  @keyframes ring-snap { from { transform: scale(1.06); opacity: 0; } to { transform: scale(1); opacity: 1; } }

  /* Liquid (§4.9): three soft blobs behind the avatar, swelling with the level and drifting. */
  .liquid { inset: calc(var(--d) * -.05); opacity: .85; scale: calc(.88 + .12 * var(--lvl, .5)); transition: scale .1s linear; }
  .ring-box[data-ring='still'] .liquid { scale: 1; }
  .ring-box[data-ring='off'] .layer.liquid { transition: opacity .2s, scale .1s linear; }
  .blob { position: absolute; inset: 0; border-radius: 50%; background: radial-gradient(circle closest-side, var(--speak) 55%, transparent 100%); }
  .ring-box[data-ring='on'] .b1 { animation: ring-b1 1.3s ease-in-out infinite; }
  .ring-box[data-ring='on'] .b2 { animation: ring-b2 1.7s ease-in-out infinite; }
  .ring-box[data-ring='on'] .b3 { animation: ring-b3 1.1s ease-in-out infinite; }
  @keyframes ring-b1 { 0%, 100% { transform: none; } 50% { transform: translate(calc(var(--d) * .0625), calc(var(--d) * -.052)) scale(1.04); } }
  @keyframes ring-b2 { 0%, 100% { transform: none; } 50% { transform: translate(calc(var(--d) * -.0625), calc(var(--d) * .042)) scale(1.05); } }
  @keyframes ring-b3 { 0%, 100% { transform: none; } 50% { transform: translate(calc(var(--d) * .021), calc(var(--d) * .073)) scale(1.03); } }

  /* Wave of dots (§4.10): a wave travels round; taller with the level (and faster: lib/ringLevels' playback rate). */
  .dots { inset: calc(var(--d) * -.16); }
  .dots .dot {
    position: absolute; top: calc(var(--d) * .056); left: 50%; width: max(3px, calc(var(--d) * .052)); aspect-ratio: 1; translate: -50% 0; border-radius: 50%;
    background: var(--speak); scale: calc(.8 + .6 * var(--lvl, .5)); transition: scale .1s linear;
  }
  .ring-box[data-ring='still'] .dots .dot { scale: 1; }
  .ring-box[data-ring='on'] .dots .dot { animation: ring-dot 1s ease-in-out var(--delay, 0s) infinite; }
  @keyframes ring-dot { 0%, 100% { transform: scale(.6); } 50% { transform: scale(1); } }

  /* Bars round the ring (§4.11): neighbours in near phase read as a radial equaliser; they grow from their inner end. */
  .bars { inset: calc(var(--d) * -.16); }
  .bars .bar {
    position: absolute; top: 0; left: 50%; width: max(1px, calc(var(--d) * .031)); height: calc(var(--d) * .13); translate: -50% 0; border-radius: 999px;
    background: var(--speak); transform-origin: 50% 100%; transform: scaleY(var(--rest, .55)); scale: 1 calc(.5 + .5 * var(--lvl, .5)); transition: scale .1s linear;
  }
  .ring-box[data-ring='still'] .bars .bar { scale: 1; }
  .ring-box[data-ring='on'] .bars .bar { animation: ring-bar .5s ease-in-out var(--delay, 0s) infinite alternate; }
  @keyframes ring-bar { from { transform: scaleY(.35); } to { transform: scaleY(1); } }
</style>
