<script lang="ts">
  // An invite to a server (#80), as a card: a direct invite's message, or an invite link pasted in a message. A banner
  // strip of its own (the server's, #475, else a solid gradient of its two colours), its chip over the banner's bottom
  // edge as on a profile card, then, all on the card's own surface (never over the picture), who invited you, its
  // name, "N online · M members", up to three faces you know, and the Join button on the right (the invite's own words, else "Join
  // <server>"). Once you're in: "You're in" and Open. Expired, used, revoked or unknown, someone else's, or banned:
  // greyed with why. What it shows is per viewer, asked of the server once the card is on screen and kept for the
  // session (lib/invitePreviews.svelte): lookups are rate limited, so it never asks in a loop.
  import { isMember } from '../lib/selectors'
  import { app, ui } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { invitePreviews, loadInvitePreview } from '../lib/invitePreviews.svelte'
  import { acceptInvite } from '../lib/inviteActions.svelte'
  import { cardView, countsLine, type CardState } from '../lib/serverInvites'
  import Avatar from './Avatar.svelte'
  import ServerChip from './ServerChip.svelte'
  import ServerBanner from './ServerBanner.svelte'

  let { code }: { code: string } = $props()

  let root = $state<HTMLElement>()
  // ask once it's on screen (or at once where there's no IntersectionObserver)
  $effect(() => {
    const el = root
    const c = code
    if (!el) return
    if (typeof IntersectionObserver !== 'function') return loadInvitePreview(c)
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        loadInvitePreview(c)
        io.disconnect()
      }
    })
    io.observe(el)
    return () => io.disconnect()
  })

  const entry = $derived(invitePreviews.byCode[code])
  const preview = $derived(entry?.status === 'ok' ? entry.preview : null)
  /** As you see it now: joined since it was asked (here or on another device), the server's members say so. */
  const shown = $derived.by((): CardState | null => {
    if (!entry || entry.status === 'loading') return null
    if (entry.status === 'gone') return 'not_found'
    if (entry.status === 'error') return 'unavailable'
    const p = entry.preview
    return isMember(app.state, p.server_id) ? 'already_member' : p.state
  })
  const view = $derived(shown ? cardView(shown, preview ?? undefined) : null)
  const mine = $derived(!!preview && preview.inviter_id === app.state.me?.id)

  let joining = $state(false)
  let error = $state('')
  async function act() {
    if (!view?.button?.action || !preview) return
    if (view.button.action === 'open') {
      ui.goServer = preview.server_id
      return
    }
    if (joining) return
    joining = true
    error = ''
    try {
      await acceptInvite(code)
    } catch (e) {
      error = errorText(e)
    } finally {
      joining = false
    }
  }
</script>

<section bind:this={root} class="card" class:dead={view?.dead} class:loading={!view} aria-label={preview ? `Invite to ${preview.name}` : 'Server invite'} data-server-invite={code} data-state={shown ?? 'loading'}>
  <div class="banner" class:pic={!!preview?.banner_hash}><ServerBanner id={preview?.server_id ?? code} hash={preview?.banner_hash} still={preview?.banner_still} /></div>
  <!-- the chip over the banner's bottom edge; nothing else touches the banner -->
  <span class="icon">
    {#if preview}
      <ServerChip id={preview.server_id} name={preview.name} iconHash={preview.icon_hash} size={56} />
    {:else}
      <span class="unknown" aria-hidden="true">?</span>
    {/if}
  </span>
  <div class="body">
    <div class="info">
      {#if preview}
        <span class="head">{mine ? 'You sent an invite' : `${preview.inviter_name} invited you`}</span>
        <strong class="name">{preview.name}</strong>
        {#if view?.note}<span class="note" data-note>{view.note}</span>{:else}<span class="counts">{countsLine(preview.online_count, preview.member_count)}</span>{/if}
      {:else if view}
        <strong class="name">{view.note}</strong>
      {:else}
        <strong class="name muted">Loading invite…</strong>
      {/if}
    </div>
    {#if preview && preview.faces.length && !view?.dead}
      <span class="faces" aria-label="People you know there">
        {#each preview.faces.slice(0, 3) as f (f.id)}<span class="face"><Avatar id={f.id} name={f.display_name ?? f.username} size={22} /></span>{/each}
      </span>
    {/if}
    {#if view?.button}
      <button type="button" class="join" class:open={view.button.action === 'open'} disabled={!view.button.action || joining} onclick={() => void act()}>{view.button.label}</button>
    {/if}
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</section>

<style>
  /* the game invite card's frame (#436): bg-2 on the chat, a line round it */
  .card { position: relative; overflow: hidden; width: min(100%, 520px); margin-top: 4px; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-lg); }
  /* the banner's own strip: no text ever sits on it */
  .banner { height: 96px; }
  .dead .banner, .loading .banner { filter: grayscale(1); opacity: .35; }
  /* the chip over the banner's bottom edge, ringed in the card's colour (as a profile card's avatar) */
  .icon { position: relative; display: inline-grid; margin: -32px 0 0 16px; padding: 4px; border-radius: 30%; background: var(--bg-2); }
  .dead .icon :global(.chip) { filter: grayscale(1); opacity: .6; }
  .unknown { width: 56px; height: 56px; border-radius: 28%; background: var(--bg-3); color: var(--text-3); display: grid; place-items: center; font-size: 24px; font-weight: 700; }
  /* everything else on the card's own surface: the theme's text colours, whatever the banner is */
  .body { display: flex; align-items: center; gap: 12px; padding: 6px 16px 16px; }
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .head { font-size: 12px; font-weight: 600; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name { font-size: 16px; font-weight: 700; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dead .name, .muted { color: var(--text-2); }
  .counts, .note { font-size: 12px; color: var(--text-2); }
  .dead .note { color: var(--text-3); }
  .faces { display: flex; flex-shrink: 0; }
  .face { display: inline-grid; border-radius: 50%; box-shadow: 0 0 0 2px var(--bg-2); --ring: var(--bg-2); }
  .face + .face { margin-left: -6px; }
  /* a live action: the accent, like Join game */
  .join { flex-shrink: 0; height: 32px; max-width: 180px; padding: 0 14px; border: 0; border-radius: var(--radius-sm); background: var(--accent); color: var(--on-accent); font-family: inherit; font-size: 13px; font-weight: 700; cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; transition: filter var(--dur-fast); }
  .join:hover:not(:disabled) { filter: brightness(1.08); }
  .join:disabled { opacity: .6; cursor: default; }
  .join.open { background: var(--bg-3); color: var(--text); }
  .dead .join { background: var(--bg-3); color: var(--text-3); opacity: 1; }
  .err { margin: 0; padding: 0 16px 12px; color: var(--danger-text); font-size: 13px; }
</style>
