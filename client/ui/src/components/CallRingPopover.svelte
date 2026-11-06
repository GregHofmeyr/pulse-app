<script lang="ts">
  // An incoming call while you're somewhere else in the app (#50): who's calling (their avatar; a group's name with
  // the caller's), Join and Decline. It sits over the top of the content without blocking it, and stays until
  // answered, declined or timed out. Inside the conversation the call band shows the ring instead (CallBand).
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import { app } from '../lib/store.svelte'
  import { declineCall, joinCall, popoverRings } from '../lib/calls.svelte'
  import { conversationName } from '../lib/conversations'
  import { displayName } from '../lib/selectors'

  const rings = $derived(popoverRings())
</script>

<div class="stack" aria-live="polite">
  {#each rings as r (r.call_id)}
    {@const caller = displayName(app.state, null, r.caller_id)}
    {@const group = app.state.channels[r.channel_id]?.kind === 'group'}
    <div class="ring pop-in" role="alertdialog" aria-label="Incoming call from {caller}">
      <span class="av"><Avatar id={r.caller_id} name={caller} size={40} /></span>
      <span class="text">
        {#if group}
          <strong>{conversationName(app.state, r.channel_id)}</strong>
          <small><span class="name-clip">{caller}</span> is calling</small>
        {:else}
          <strong class="name-clip">{caller}</strong>
          <small>is calling you</small>
        {/if}
      </span>
      <button type="button" class="join" onclick={() => joinCall(r.channel_id)}><Icon name="phone" size={14} /> Join</button>
      <button type="button" class="decline" onclick={() => declineCall(r.channel_id)}>Decline</button>
    </div>
  {/each}
</div>

<style>
  /* top centre, over the content; only the cards take clicks */
  .stack { position: fixed; top: 14px; left: 50%; transform: translateX(-50%); z-index: 45; display: flex; flex-direction: column; gap: 8px; pointer-events: none; }
  .ring { pointer-events: auto; display: flex; align-items: center; gap: 12px; padding: 10px 12px; min-width: 300px; background: var(--surface-menu); border: 1px solid var(--line-menu); border-radius: 14px; box-shadow: var(--shadow-2); }
  .av { border-radius: 50%; box-shadow: 0 0 0 2px var(--ring), 0 0 0 4px var(--ok); display: grid; }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .text strong { font-size: 14px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .text small { font-size: 12px; color: var(--text-3); }
  .join { height: 32px; padding: 0 12px; border: 0; border-radius: 9px; background: var(--accent); color: var(--on-accent); font-weight: 600; display: flex; align-items: center; gap: 6px; }
  .decline { height: 32px; padding: 0 12px; border: 0; border-radius: 9px; background: var(--danger); color: var(--on-danger); font-weight: 600; }
  .join:hover, .decline:hover { filter: brightness(1.1); }
</style>
