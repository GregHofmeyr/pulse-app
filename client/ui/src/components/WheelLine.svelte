<script lang="ts">
  // A picker wheel spin in the chat (#158, #271). When the spin reaches this client as it happens, it mounts as a
  // card with the wheel spinning to the server's angle (the same for everyone watching); it lands with a chime and
  // the result view over the wheel. The card stays until you've seen it (Pulse focused, this chat on screen, the
  // card in view), then CARD_SEEN_MS more, then folds into the line. ✕ folds it at once; it folds anyway an hour
  // after the spin (lib/wheelCards.ts, per device). Opened later (or mid-spin) it's the card with the result view
  // while that holds, else the line; it never replays the spin. The line is a mini result card (#273): a thumbnail of
  // the landed wheel, "▮ Valheim" large, "alex spun Games tonight · 8 options · 16:50" under it, and icon buttons
  // that show on hover or focus. Spin again, Remove winner & spin and, for a quick wheel, Save as wheel are on both.
  // Reduced motion, being live and an unfocused window spin short without ticks (the chime still plays).
  import { onMount, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import Wheel from './Wheel.svelte'
  import SaveWheelForm from './SaveWheelForm.svelte'
  import { clickOutside } from '../lib/actions'
  import { popoverStyle } from '../lib/reactions'
  import { errorText } from '../lib/tauri'
  import { playSound } from '../lib/voiceui'
  import { WHEEL_LIMITS } from '../lib/wheel'
  import { cardState, dismissCard, markCardSeen } from '../lib/wheelCards'
  import { sliceColour } from '../lib/wheelColours'
  import { quietNow, runSpin } from '../lib/wheelSpin'
  import { chatShown, spinAgain, spunLive } from '../lib/wheels.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import type { Message } from '../lib/protocol/Message'
  import type { WheelSpin } from '../lib/protocol/WheelSpin'

  let { message, serverId, time }: { message: Message; serverId: string | null; time: string } = $props()

  const spin = $derived(message.wheel as WheelSpin)
  const n = $derived(spin.options.length)
  const winner = $derived(spin.options[spin.winner] ?? '')
  const label = $derived(spin.name ?? 'a quick wheel')
  const shown = chatShown()

  // decided once, as it mounts: a spin seen later never replays
  const id = untrack(() => message.id)
  const spunAt = untrack(() => Date.parse(message.created_at) || Date.now())
  const live = untrack(() => spunLive(id))
  const quiet = untrack(quietNow)
  let phase = $state<'spinning' | 'card' | 'line'>(untrack(() => (live ? 'spinning' : cardState(id, spunAt).show ? 'card' : 'line')))
  let rotation = $state(untrack(() => (live ? 0 : (message.wheel?.angle ?? 0))))

  onMount(() => {
    if (!live) return
    const { angle, options } = untrack(() => spin)
    return runSpin({
      angle,
      count: options.length,
      quiet,
      onRotate: (deg) => (rotation = deg),
      onLanded: () => {
        // with this chat open, even with Pulse in the background: that's when the chime helps (#271)
        if (shown()) playSound('wheel_result')
        phase = cardState(id, spunAt).show ? 'card' : 'line'
      },
    })
  })

  // seen: the card in view (no IntersectionObserver: it counts as in view) with Pulse focused and the chat shown
  let card: HTMLElement | undefined = $state()
  let inView = $state(typeof IntersectionObserver !== 'function')
  $effect(() => {
    if (!card || typeof IntersectionObserver !== 'function') return
    const io = new IntersectionObserver((es) => {
      for (const e of es) inView = e.isIntersecting
    })
    io.observe(card)
    return () => io.disconnect()
  })
  let seen = $state(untrack(() => cardState(id, spunAt).seen))
  $effect(() => {
    if (phase === 'card' && seen === null && inView && windowFocus.focused && shown()) {
      markCardSeen(id, spunAt)
      seen = cardState(id, spunAt).seen
    }
  })
  // folds when its time's up (seen + CARD_SEEN_MS, or the hour)
  $effect(() => {
    if (phase !== 'card') return
    void seen
    const { show, foldAt } = cardState(id, spunAt)
    if (!show) {
      phase = 'line'
      return
    }
    const t = setTimeout(() => (phase = 'line'), Math.min(foldAt - Date.now(), 2 ** 31 - 1))
    return () => clearTimeout(t)
  })
  function dismiss() {
    dismissCard(id, spunAt)
    phase = 'line'
  }

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

{#snippet acts()}
  <span class="acts">
    <button type="button" class="link-btn" disabled={busy} onclick={() => again(false)}>Spin again</button>
    {#if n > WHEEL_LIMITS.minOptions}· <button type="button" class="link-btn" disabled={busy} onclick={() => again(true)}>Remove winner &amp; spin</button>{/if}
    {#if !spin.name}
      {#if savedAs}· <span class="saved">Saved as {savedAs}</span>
      {:else}· <button bind:this={saveBtn} type="button" class="link-btn" data-menu-toggle aria-haspopup="dialog" aria-expanded={saving}
        onclick={() => (saving = !saving)}>Save as wheel <Icon name="chevronRight" size={14} /></button>{/if}
    {/if}
  </span>
{/snippet}

{#if phase !== 'line'}
  <div bind:this={card} class="wheel-card" data-message-id={message.id}>
    <Wheel options={spin.options} {rotation} size={180} landed={phase === 'card' ? spin.winner : null} result={phase === 'card' ? winner : null} />
    <div class="body">
      <p class="caption" aria-live="polite">
        {#if phase === 'spinning'}{@render who()} is spinning {label}…
        {:else}{@render who()} spun {label} <span class="sr-only">→ {winner}</span>{/if}
      </p>
      {#if phase === 'card'}
        {@render acts()}
        <span class="time">{time}</span>
      {/if}
      {#if error}<span class="err" role="alert">{error}</span>{/if}
    </div>
    {#if phase === 'card'}
      <button type="button" class="icon-btn icon-btn-xs dismiss" aria-label="Dismiss" data-tip="Dismiss" onclick={dismiss}><Icon name="x" size={14} /></button>
    {/if}
  </div>
{:else}
  <!-- the 🎡 line as a mini result card (#273): the landed wheel, the winner, who spun what; actions on hover or focus -->
  <div class="wheel-line" data-message-id={message.id}>
    <Wheel options={spin.options} rotation={spin.angle} size={40} landed={spin.winner} mini />
    <div class="txt">
      <p class="top"><span class="sr-only">🎡 </span><span class="swatch" style:background={sliceColour(spin.winner, n)}></span><strong class="won">{winner}</strong></p>
      <p class="sub">{@render who()} spun {label} · {n} options · {time}{#if savedAs} · <span class="saved">Saved as {savedAs}</span>{/if}</p>
      {#if error}<p class="err" role="alert">{error}</p>{/if}
    </div>
    <span class="acts icons" class:held={saving || busy}>
      <button type="button" class="icon-btn icon-btn-sm" aria-label="Spin again" data-tip="Spin again" disabled={busy} onclick={() => again(false)}><Icon name="spinAgain" size={16} /></button>
      {#if n > WHEEL_LIMITS.minOptions}
        <button type="button" class="icon-btn icon-btn-sm" aria-label="Remove winner & spin" data-tip="Remove winner & spin" disabled={busy} onclick={() => again(true)}><Icon name="spinRemove" size={16} /></button>
      {/if}
      {#if !spin.name && !savedAs}
        <button bind:this={saveBtn} type="button" class="icon-btn icon-btn-sm" aria-label="Save as wheel" data-tip="Save as wheel" data-menu-toggle aria-haspopup="dialog"
          aria-expanded={saving} onclick={() => (saving = !saving)}><Icon name="bookmark" size={16} /></button>
      {/if}
    </span>
  </div>
{/if}
{#if saving}
  <div class="save-pop menu-surface pop-in" role="dialog" aria-label="Save as wheel" use:clickOutside={() => (saving = false)} style={saveStyle}>
    <SaveWheelForm {serverId} options={spin.options} onCancel={() => (saving = false)} onSaved={(w) => { saving = false; savedAs = w.name }} />
  </div>
{/if}

<style>
  /* the spin card sits where the line will be, indented like the system lines */
  .wheel-card { position: relative; display: flex; align-items: center; gap: 16px; margin: 6px 18px 6px 30px; padding: 12px 36px 12px 16px; align-self: flex-start; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-lg); }
  .body { display: flex; flex-direction: column; gap: 6px; min-width: 0; max-width: 280px; }
  .caption { margin: 0; font-size: 14px; color: var(--text-2); }
  .dismiss { position: absolute; top: 6px; right: 6px; }
  /* the line (#273): a small card, indented like the system lines */
  .wheel-line { display: flex; align-items: center; gap: 12px; box-sizing: border-box; max-width: 460px; margin: 4px 18px 4px 30px; padding: 8px 6px 8px 8px; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-md); }
  .txt { flex: 1; min-width: 0; }
  .top, .sub { margin: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .top { display: flex; align-items: center; gap: 6px; font-size: 15px; line-height: 1.3; }
  .won { min-width: 0; overflow: hidden; text-overflow: ellipsis; color: var(--text); font-weight: 700; }
  .sub { font-size: 12px; color: var(--text-3); }
  .txt .err { margin: 2px 0 0; }
  /* the actions show on hover or focus (and while one is in use); still there for Tab, so focus reveals them */
  .icons { display: inline-flex; align-items: center; gap: 2px; flex-shrink: 0; }
  .icons .icon-btn { opacity: 0; }
  .wheel-line:hover .icon-btn, .wheel-line:focus-within .icon-btn, .icons.held .icon-btn { opacity: 1; }
  .icons .icon-btn:disabled { opacity: .5; }
  .swatch { display: inline-block; flex-shrink: 0; width: 10px; height: 10px; border-radius: var(--radius-xs); vertical-align: -1px; }
  .acts { display: inline-flex; align-items: center; gap: 6px; flex-wrap: wrap; font-size: 13px; color: var(--text-3); }
  .acts .link-btn { display: inline-flex; align-items: center; gap: 2px; font-size: 13px; }
  .acts .link-btn:disabled { opacity: .5; cursor: default; }
  .saved { color: var(--text-3); }
  .time { font-size: 11px; color: var(--text-3); }
  .err { flex-basis: 100%; color: var(--danger-text); font-size: 12px; }
  .save-pop { width: 280px; padding: 12px; z-index: 50; --pop-origin: top left; }
</style>
