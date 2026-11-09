<script lang="ts">
  // A picker wheel spin in the chat (#158). When the spin reaches this client as it happens, the line mounts as a
  // card with the wheel spinning to the server's angle (the same for everyone watching), holds the result a moment,
  // then folds into the line: "🎡 alex spun Games tonight → ▮ Valheim" with Spin again, Remove winner & spin and,
  // for a quick wheel, Save as wheel. Opened later (or mid-spin) it is the settled line straight away. Reduced
  // motion, being live and an unfocused window spin short and silent.
  import { onMount, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import Wheel from './Wheel.svelte'
  import SaveWheelForm from './SaveWheelForm.svelte'
  import { clickOutside } from '../lib/actions'
  import { motion } from '../lib/animate.svelte'
  import { twemojiUrl } from '../lib/emoji'
  import { popoverStyle } from '../lib/reactions'
  import { errorText } from '../lib/tauri'
  import { playSound } from '../lib/voiceui'
  import { HOLD_MS, QUIET_HOLD_MS, rotationAt, spinDuration, TICK_GAP_MS, winnerAt } from '../lib/wheel'
  import { sliceColour } from '../lib/wheelColours'
  import { spinAgain, spunLive } from '../lib/wheels.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import type { Message } from '../lib/protocol/Message'
  import type { WheelSpin } from '../lib/protocol/WheelSpin'

  let { message, serverId, time }: { message: Message; serverId: string | null; time: string } = $props()

  const spin = $derived(message.wheel as WheelSpin)
  const n = $derived(spin.options.length)
  const winner = $derived(spin.options[spin.winner] ?? '')
  const label = $derived(spin.name ?? 'a quick wheel')

  // decided once, as it mounts: a line seen later never replays its spin
  const live = untrack(() => spunLive(message.id))
  const quiet = untrack(() => motion.reducedMotion || motion.streaming || !windowFocus.focused)
  let phase = $state<'spinning' | 'landed' | 'line'>(untrack(() => (live ? 'spinning' : 'line')))
  let rotation = $state(0)

  onMount(() => {
    if (!live) return
    const { angle, options } = untrack(() => spin)
    const count = options.length
    const duration = spinDuration(quiet)
    let start: number | null = null
    let raf = 0
    let hold: ReturnType<typeof setTimeout> | undefined
    let slice = winnerAt(0, count)
    let lastTick = -Infinity
    const frame = (t: number) => {
      start ??= t
      const elapsed = t - start
      rotation = rotationAt(elapsed, angle, quiet)
      // a quiet tick each time a new slice passes the pointer (never under reduced motion)
      const now = winnerAt(rotation, count)
      if (now !== slice) {
        slice = now
        if (!quiet && t - lastTick >= TICK_GAP_MS) {
          lastTick = t
          playSound('wheel_tick')
        }
      }
      if (elapsed < duration) raf = requestAnimationFrame(frame)
      else {
        phase = 'landed'
        hold = setTimeout(() => (phase = 'line'), quiet ? QUIET_HOLD_MS : HOLD_MS)
      }
    }
    raf = requestAnimationFrame(frame)
    return () => {
      cancelAnimationFrame(raf)
      clearTimeout(hold)
    }
  })

  let busy = $state(false)
  let error = $state('')
  async function again(removeWinner: boolean) {
    if (busy) return
    busy = true
    error = ''
    try {
      await spinAgain(message.id, removeWinner)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  let saveBtn: HTMLButtonElement | undefined = $state()
  let saving = $state(false)
  let savedAs = $state('')
  const saveStyle = $derived(saving && saveBtn ? popoverStyle(saveBtn.getBoundingClientRect(), { width: 280, height: 170 }, { width: window.innerWidth, height: window.innerHeight }) : '')
</script>

{#snippet who()}{#if spin.spinner_id}<Name userId={spin.spinner_id} {serverId} />{:else}Someone{/if}{/snippet}

{#if phase !== 'line'}
  <div class="wheel-card" data-message-id={message.id}>
    <Wheel options={spin.options} {rotation} size={180} landed={phase === 'landed' ? spin.winner : null} />
    <p class="caption" aria-live="polite">
      {#if phase === 'spinning'}{@render who()} is spinning {label}…
      {:else}<span class="swatch" style:background={sliceColour(spin.winner, n)}></span> <strong>{winner}</strong>{/if}
    </p>
  </div>
{:else}
  <div class="system wheel-line" data-message-id={message.id}>
    <img class="emoji" src={twemojiUrl('🎡')} alt="🎡" draggable="false" />
    <span class="what">{@render who()} spun {label} → <span class="swatch" style:background={sliceColour(spin.winner, n)}></span> <strong class="won">{winner}</strong></span>
    <span class="acts">
      · <button type="button" class="link-btn" disabled={busy} onclick={() => again(false)}>Spin again</button>
      {#if n > 2}· <button type="button" class="link-btn" disabled={busy} onclick={() => again(true)}>Remove winner &amp; spin</button>{/if}
      {#if !spin.name}
        {#if savedAs}· <span class="saved">Saved as {savedAs}</span>
        {:else}· <button bind:this={saveBtn} type="button" class="link-btn" data-menu-toggle aria-haspopup="dialog" aria-expanded={saving}
          onclick={() => (saving = !saving)}>Save as wheel <Icon name="chevronRight" size={14} /></button>{/if}
      {/if}
    </span>
    <span class="time">{time}</span>
    {#if error}<span class="err" role="alert">{error}</span>{/if}
  </div>
  {#if saving}
    <div class="save-pop menu-surface pop-in" role="dialog" aria-label="Save as wheel" use:clickOutside={() => (saving = false)} style={saveStyle}>
      <SaveWheelForm {serverId} options={spin.options} onCancel={() => (saving = false)} onSaved={(w) => { saving = false; savedAs = w.name }} />
    </div>
  {/if}
{/if}

<style>
  /* the spin card sits where the line will be, indented like the system lines */
  .wheel-card { display: flex; align-items: center; gap: 16px; margin: 6px 18px 6px 30px; padding: 12px 16px; align-self: flex-start; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-lg); }
  .caption { margin: 0; max-width: 260px; font-size: 14px; color: var(--text-2); display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .caption strong { color: var(--text); font-size: 18px; }
  .system { padding: 6px 18px 6px 30px; font-size: 13px; color: var(--text-3); }
  .wheel-line { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 6px; }
  .emoji { width: 16px; height: 16px; }
  .what { color: var(--text-2); }
  .won { color: var(--text); }
  .swatch { display: inline-block; width: 10px; height: 10px; border-radius: var(--radius-xs); vertical-align: -1px; }
  .acts { display: inline-flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .acts .link-btn { display: inline-flex; align-items: center; gap: 2px; font-size: 13px; }
  .acts .link-btn:disabled { opacity: .5; cursor: default; }
  .saved { color: var(--text-3); }
  .time { font-size: 11px; color: var(--text-3); }
  .err { flex-basis: 100%; color: var(--danger-text); font-size: 12px; }
  .save-pop { width: 280px; padding: 12px; z-index: 50; --pop-origin: top left; }
</style>
