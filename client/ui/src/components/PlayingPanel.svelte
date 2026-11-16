<script lang="ts">
  // "Playing" on a profile card (#380): the game's wide art (Steam's library hero, else its store header) faded behind
  // a strip with its logo (else its name) and how long they've played. A game with no art (not from Steam, or art
  // switched off) is the strip alone; no play time when they switched it off. Not the banner (owner): that stays theirs.
  import { clock } from '../lib/store.svelte'
  import { elapsedLabel, gameArt } from '../lib/game'
  import type { Playing } from '../lib/protocol/Playing'

  let { playing }: { playing: Playing } = $props()
  const art = $derived(gameArt(playing))
  const elapsed = $derived(elapsedLabel(playing.started_at, Date.parse(clock.nowIso)))
  // a logo that won't load gives way to the name
  let logoFailed = $state(false)
  $effect(() => {
    void art.logo
    logoFailed = false
  })
</script>

<section class="playing" class:has-art={!!art.wide} aria-label="Playing {playing.name}">
  {#if art.wide}<img class="wide" src={art.wide} alt="" />{/if}
  <div class="strip">
    <span class="head">Playing</span>
    {#if art.logo && !logoFailed}
      <img class="logo" src={art.logo} alt={playing.name} data-tip={playing.name} onerror={() => (logoFailed = true)} />
    {:else}
      <span class="name" data-tip={playing.name}>{playing.name}</span>
    {/if}
    {#if elapsed}<span class="time">for {elapsed}</span>{/if}
  </div>
</section>

<style>
  /* the card's own translucent panel (ProfileCard .panel), sized with the card (--k, #156) */
  .playing { position: relative; overflow: hidden; border-radius: var(--radius-md); background: var(--card-panel); color: var(--card-text); font-size: calc(13px * var(--k, 1)); }
  .wide { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: .5; display: block; }
  /* the art fades out under the strip's side, so the words always sit on the card's own panel */
  .has-art::after { content: ''; position: absolute; inset: 0; background: linear-gradient(to right, var(--card-panel) 30%, transparent); pointer-events: none; }
  .strip { position: relative; z-index: 1; display: flex; flex-direction: column; align-items: flex-start; gap: 4px; padding: calc(10px * var(--k, 1)) calc(12px * var(--k, 1)); min-height: calc(56px * var(--k, 1)); justify-content: center; }
  .head { font-size: calc(12px * var(--k, 1)); font-weight: 600; color: var(--card-text-2); }
  .logo { display: block; max-width: 60%; max-height: calc(40px * var(--k, 1)); object-fit: contain; object-position: left center; }
  .name { font-weight: 700; max-width: 100%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .time { font-size: calc(12px * var(--k, 1)); color: var(--card-text-2); }
</style>
