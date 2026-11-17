<script lang="ts">
  // An invite to play (#436), in its message's body: who's inviting, the game (Steam art behind it, the logo or else
  // its name) and Join game while there's a lobby or game server to join. Where to join is never in the message: Join
  // asks the server for it (only the chat's members get it) and opens Steam. Once its sender leaves the game or the
  // lobby the button greys out with "No longer in this game" (GameInviteChanged, and on reload). The sender sees the
  // card without Join.
  import { gameArt } from '../lib/game'
  import { joinGameInvite } from '../lib/gameInvites.svelte'
  import { app } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import type { GameInvite } from '../lib/protocol/GameInvite'
  import type { Message } from '../lib/protocol/Message'
  import Icon from './Icon.svelte'

  let { message, nameOf }: { message: Message; nameOf: (userId: string | null) => string } = $props()

  const invite = $derived(message.game_invite as GameInvite)
  const mine = $derived(message.author_id === app.state.me?.id)
  const art = $derived(gameArt(invite))
  const who = $derived(mine ? 'You' : nameOf(message.author_id))
  let joining = $state(false)
  let error = $state('')
  let logoFailed = $state(false)
  $effect(() => {
    void art.logo
    logoFailed = false
  })
  async function join() {
    if (joining || !invite.joinable) return
    joining = true
    error = ''
    try {
      await joinGameInvite(message.id)
    } catch (e) {
      error = errorText(e)
    } finally {
      joining = false
    }
  }
</script>

<section class="invite" class:has-art={!!art.wide} class:ended={invite.ended} aria-label="Invite to play {invite.name}" data-game-invite>
  {#if art.wide}<img class="wide" src={art.wide} alt="" />{/if}
  <div class="strip">
    <span class="head"><Icon name="gamepad" size={14} /> {who} invited {mine ? 'them' : 'you'} to play</span>
    {#if art.logo && !logoFailed}
      <img class="logo" src={art.logo} alt={invite.name} data-tip={invite.name} onerror={() => (logoFailed = true)} />
    {:else}
      <span class="name" data-tip={invite.name}>{invite.name}</span>
    {/if}
    {#if invite.ended}
      <button type="button" class="join" disabled>No longer in this game</button>
    {:else if invite.joinable && !mine}
      <button type="button" class="join" disabled={joining} onclick={() => void join()}><Icon name="play" size={14} /> Join game</button>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </div>
</section>

<style>
  /* a card like the plan card: bg-2 on the chat, a line round it, the art faded behind the words */
  .invite { position: relative; overflow: hidden; max-width: 440px; margin-top: 4px; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-lg); }
  .wide { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: .45; display: block; }
  .has-art::after { content: ''; position: absolute; inset: 0; background: linear-gradient(to right, var(--bg-2) 35%, transparent); pointer-events: none; }
  .ended .wide { filter: grayscale(1); opacity: .25; }
  .strip { position: relative; z-index: 1; display: flex; flex-direction: column; align-items: flex-start; gap: 6px; padding: 12px; min-height: 72px; justify-content: center; }
  .head { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 600; color: var(--text-2); }
  .logo { display: block; max-width: 60%; max-height: 44px; object-fit: contain; object-position: left center; }
  .name { font-weight: 700; font-size: 15px; color: var(--text); max-width: 100%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  /* a live action: the accent, like Join on the Playing card */
  .join { height: 30px; padding: 0 12px; display: inline-flex; align-items: center; gap: 6px; border: 0; border-radius: var(--radius-sm); background: var(--accent); color: var(--on-accent); font-family: inherit; font-size: 13px; font-weight: 700; cursor: pointer; transition: filter var(--dur-fast); }
  .join:hover:not(:disabled) { filter: brightness(1.08); }
  .join:disabled { opacity: .6; cursor: default; }
  .ended .join { background: var(--bg-3); color: var(--text-3); opacity: 1; }
  .err { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
