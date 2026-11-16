<script lang="ts">
  // "Playing" on a profile card (#380): the game's wide art (Steam's library hero, else its store header) faded behind
  // a strip with its logo (else its name) and how long they've played. A game with no art (not from Steam, or art
  // switched off) is the strip alone; no play time when they switched it off. Not the banner (owner): that stays theirs.
  // Join (#381): their public Steam lobby or game server, as the panel's own link-out (like Listen on Spotify); the
  // same action is "Join game" in their right-click menu. Never on your own.
  import { app, clock, ui } from '../lib/store.svelte'
  import { elapsedLabel, gameArt } from '../lib/game'
  import { api, errorText } from '../lib/tauri'
  import { joinUrl } from '../lib/steamJoin'
  import type { Playing } from '../lib/protocol/Playing'

  let { playing, userId = null }: { playing: Playing; userId?: string | null } = $props()
  const join = $derived(userId && userId !== app.state.me?.id && joinUrl(playing.join) ? playing.join! : null)
  let joining = $state(false)
  async function joinGame() {
    if (!join || joining) return
    joining = true
    try {
      await api.joinSteamGame(join)
    } catch (e) {
      ui.notice = errorText(e)
    } finally {
      joining = false
    }
  }
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
    {#if join}
      <button type="button" class="join" disabled={joining} data-tip={join.kind === 'lobby' ? "Join their Steam lobby" : "Join their game server"} onclick={() => void joinGame()}>Join</button>
    {/if}
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
  /* a live action: the accent (owner pick 5), the panel's Listen-on-Spotify size */
  .join { margin-top: 2px; height: 28px; padding: 0 12px; display: inline-flex; align-items: center; border: 0; border-radius: var(--radius-sm); background: var(--accent); color: var(--on-accent); font-family: inherit; font-size: 12px; font-weight: 700; cursor: pointer; transition: filter var(--dur-fast); }
  .join:hover:not(:disabled) { filter: brightness(1.08); }
  .join:disabled { opacity: .6; cursor: default; }
</style>
