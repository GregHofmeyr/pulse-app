<script lang="ts" module>
  /** How long the button text waits for a pause in typing before it makes a new link. */
  export const JOIN_TEXT_WAIT_MS = 700
</script>

<script lang="ts">
  // Invite people to a server (#80), from its tab's menu or the member list's "+ Invite". The server's chip and name
  // on a soft glow of its colours (or its banner, #475); people you know (another server or a DM) who aren't in it, each with Invite (a direct
  // invite: a card in your DM with them, then "Sent"); and a ready link with Copy, how long it lasts and how many can
  // use it (a day and one, as the server's defaults; "Never" and "No limit" are the owner's), and the card's button
  // text. Changing any of them makes a fresh link; the one it replaces is revoked if it never left the dialog (never
  // copied), and so is the last one on close: nobody else can have it. Links are rate limited, so the button text
  // waits for a pause in typing.
  import { onDestroy, onMount } from 'svelte'
  import { modal } from '../lib/actions'
  import { avatarColor } from '../lib/avatar'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName, namesOf } from '../lib/selectors'
  import { presenceOf } from '../lib/presence'
  import { loadServerOrigin, serverOrigin } from '../lib/invitePreviews.svelte'
  import { sendDirectInvite } from '../lib/inviteActions.svelte'
  import { DEFAULT_EXPIRY, DEFAULT_USES, JOIN_TEXT_MAX, expiryChoices, inviteLink, usesChoices } from '../lib/serverInvites'
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import Segmented from './Segmented.svelte'
  import ServerChip from './ServerChip.svelte'
  import ServerBanner from './ServerBanner.svelte'

  let { serverId, onClose }: { serverId: string; onClose: () => void } = $props()

  const server = $derived(app.state.servers.find((s) => s.id === serverId))
  const name = $derived(server?.name ?? 'this server')
  const owner = $derived(!!server?.owner_id && server.owner_id === app.state.me?.id)
  onMount(loadServerOrigin)

  // --- people ---
  let q = $state('')
  let hovered = $state<string | null>(null)
  /** Per person: on its way, sent, or why not. */
  let sent = $state<Record<string, 'sending' | 'sent' | { error: string }>>({})
  const people = $derived.by(() => {
    const me = app.state.me?.id
    const inIt = new Set((app.state.members[serverId] ?? []).map((m) => m.user.id))
    const needle = q.trim().toLowerCase()
    return Object.values(app.state.people)
      .filter((p) => p.user.id !== me && !inIt.has(p.user.id))
      .filter((p) => !needle || [p.user.username, ...namesOf(app.state, null, p.user.id)].some((n) => n.toLowerCase().includes(needle)))
      .map((p) => ({ id: p.user.id, name: displayName(app.state, null, p.user.id) }))
      .sort((a, b) => a.name.localeCompare(b.name))
  })
  async function invite(userId: string) {
    if (sent[userId] === 'sending' || sent[userId] === 'sent') return
    sent[userId] = 'sending'
    try {
      await sendDirectInvite(serverId, userId)
      sent[userId] = 'sent'
    } catch (e) {
      sent[userId] = { error: errorText(e) }
    }
  }

  // --- the link ---
  let expiry = $state(DEFAULT_EXPIRY)
  let uses = $state(DEFAULT_USES)
  let joinText = $state('')
  /** The link shown, and whether it ever left the dialog (Copy, or a copy of the field). */
  let current = $state<{ code: string; copied: boolean } | null>(null)
  let making = $state(false)
  let linkError = $state('')
  let copiedFlash = $state(false)
  let closed = false
  let seq = 0
  const link = $derived(current && serverOrigin.origin ? inviteLink(serverOrigin.origin, current.code) : '')
  const revoke = (code: string) => void api.revokeInvite(code).catch(() => {})

  async function mint() {
    const mine = ++seq
    making = true
    linkError = ''
    try {
      const made = await api.createServerInvite(serverId, { max_age_secs: expiry, max_uses: uses, join_text: joinText.trim() || undefined })
      // a newer choice was made meanwhile, or the dialog closed: nobody saw this one
      if (mine !== seq || closed) return revoke(made.code)
      const was = current
      current = { code: made.code, copied: false }
      if (was && !was.copied) revoke(was.code)
    } catch (e) {
      if (mine === seq) linkError = errorText(e)
    } finally {
      if (mine === seq) making = false
    }
  }
  onMount(() => void mint())

  let typing: ReturnType<typeof setTimeout> | undefined
  function textChanged() {
    clearTimeout(typing)
    typing = setTimeout(() => void mint(), JOIN_TEXT_WAIT_MS)
  }
  function textDone() {
    if (typing === undefined) return
    clearTimeout(typing)
    typing = undefined
    void mint()
  }
  const setExpiry = (v: number) => { if (v !== expiry) { expiry = v; void mint() } }
  const setUses = (v: number) => { if (v !== uses) { uses = v; void mint() } }

  let flashTimer: ReturnType<typeof setTimeout> | undefined
  async function copy() {
    if (!current || !link) return
    current.copied = true
    try {
      await navigator.clipboard.writeText(link)
      copiedFlash = true
      clearTimeout(flashTimer)
      flashTimer = setTimeout(() => (copiedFlash = false), 1500)
    } catch (e) {
      linkError = errorText(e)
    }
  }
  const noteCopied = () => { if (current) current.copied = true }

  onDestroy(() => {
    closed = true
    clearTimeout(typing)
    clearTimeout(flashTimer)
    if (current && !current.copied) revoke(current.code)
  })
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Invite to {name}" tabindex="-1" use:modal={onClose} data-invite-dialog>
    {#if server?.banner_hash}
      <!-- its banner (#475), fading into the dialog, in place of the glow -->
      <div class="glow pic" aria-hidden="true"><ServerBanner id={serverId} hash={server.banner_hash} still={server.banner_still} /></div>
    {:else}
      <div class="glow" aria-hidden="true" style:--glow={avatarColor(serverId)}></div>
    {/if}
    <header>
      <span class="chip-in"><ServerChip id={serverId} {name} iconHash={server?.icon_hash} still={server?.icon_still} size={48} /></span>
      <h1 class="modal-title">Invite to {name}</h1>
      <button class="icon-btn round modal-close" aria-label="Close" onclick={onClose}><Icon name="x" size={16} /></button>
    </header>

    <input class="input search" placeholder="Search people" aria-label="Search people" bind:value={q} />
    <ul class="people" aria-label="People to invite">
      {#each people as p (p.id)}
        {@const st = sent[p.id]}
        <li class="person hover-row" onpointerenter={() => (hovered = p.id)} onpointerleave={() => { if (hovered === p.id) hovered = null }} data-person={p.id}>
          <Avatar id={p.id} name={p.name} size={30} presence={presenceOf(app.state, p.id, clock.nowIso)} hovered={() => hovered === p.id} />
          <span class="nm"><Name userId={p.id} serverId={null} text={p.name} interactive={false} /></span>
          {#if st && typeof st === 'object'}<span class="why" data-tip={st.error}><Icon name="alert" size={14} /></span>{/if}
          <button type="button" class="btn invite" class:btn-secondary={st !== 'sent'} class:sent={st === 'sent'} disabled={st === 'sending' || st === 'sent'}
            onclick={() => void invite(p.id)}>{st === 'sent' ? 'Sent' : 'Invite'}</button>
        </li>
      {:else}
        <li class="empty">{q.trim() ? `No one matches “${q.trim()}”.` : 'Everyone you know is in already.'}</li>
      {/each}
    </ul>

    <section class="link" aria-label="Invite link">
      <span class="lbl">Or send a link</span>
      <div class="link-row">
        <input class="input" readonly value={link || (making ? 'Making a link…' : '')} aria-label="Invite link" oncopy={noteCopied} onfocus={(e) => e.currentTarget.select()} />
        <button type="button" class="btn btn-primary" disabled={!link || making} onclick={() => void copy()}>{#if copiedFlash}<Icon name="check" size={16} /> Copied{:else}<Icon name="copy" size={16} /> Copy{/if}</button>
      </div>
      {#if linkError}<p class="err" role="alert">{linkError}</p>{/if}
      <div class="opt"><span class="lbl">Expires</span><Segmented label="Expires" size="sm" options={expiryChoices(owner)} value={expiry} onchange={setExpiry} /></div>
      <div class="opt"><span class="lbl">Uses</span><Segmented label="Uses" size="sm" options={usesChoices(owner)} value={uses} onchange={setUses} /></div>
      <label class="opt"><span class="lbl">Button text</span>
        <input class="input input-sm" maxlength={JOIN_TEXT_MAX} placeholder="Join {name}" bind:value={joinText} oninput={textChanged} onblur={textDone}
          onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); textDone() } }} />
      </label>
    </section>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; padding: 32px 16px; }
  .modal { position: relative; overflow: hidden; width: min(440px, calc(100vw - 32px)); max-height: calc(100vh - 64px); gap: 10px; }
  /* the artsy bit: a soft glow of the server's colour and the accent behind the header, drifting in as the chip pops */
  .glow { position: absolute; inset: 0 0 auto 0; height: 150px; pointer-events: none; opacity: .32;
    background: radial-gradient(55% 100% at 22% 0%, var(--glow), transparent 72%), radial-gradient(50% 90% at 82% 0%, var(--accent-strong), transparent 70%);
    animation: glow-in var(--dur-slow) var(--ease) both; }
  .glow.pic { opacity: .5; background: none; -webkit-mask-image: linear-gradient(to bottom, var(--bg-1) 30%, transparent); mask-image: linear-gradient(to bottom, var(--bg-1) 30%, transparent); }
  @keyframes glow-in { from { opacity: 0; transform: translateY(-12px); } }
  .chip-in { display: inline-grid; animation: chip-pop var(--dur-slow) var(--ease) var(--dur) both; }
  @keyframes chip-pop { from { opacity: 0; transform: scale(.7) rotate(-6deg); } 70% { transform: scale(1.06); } }
  :global(.streaming) .glow, :global(.streaming) .chip-in, :global(.unfocused) .glow, :global(.unfocused) .chip-in { animation: none; }
  @media (prefers-reduced-motion: reduce) {
    .glow, .chip-in { animation: none; }
  }
  header { position: relative; display: flex; align-items: center; gap: 12px; padding-right: 36px; }
  header h1 { margin: 0; min-width: 0; overflow-wrap: anywhere; }
  .modal-close { position: absolute; top: 0; right: 0; }
  .search { position: relative; }
  .people { position: relative; margin: 0; padding: 0; list-style: none; overflow-y: auto; min-height: 60px; max-height: 240px; display: flex; flex-direction: column; gap: 2px; }
  .person { display: flex; align-items: center; gap: 10px; padding: 5px 8px; border-radius: var(--radius-md); }
  .person:hover { background: var(--bg-3); --ring: var(--bg-3); }
  .nm { flex: 1; min-width: 0; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .why { color: var(--danger-text); display: inline-grid; }
  .invite { height: 28px; }
  .invite.sent, .invite.sent:disabled { opacity: 1; color: var(--ok-text); }
  .empty { padding: 8px; font-size: 13px; color: var(--text-3); }
  .link { position: relative; display: flex; flex-direction: column; gap: 8px; padding-top: 10px; border-top: 1px solid var(--line); }
  .lbl { font-size: 12px; font-weight: 600; color: var(--text-3); }
  .link-row { display: flex; gap: 8px; }
  .link-row .input { flex: 1; min-width: 0; font-size: 13px; }
  .opt { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .opt .lbl { flex-shrink: 0; width: 84px; }
  .opt .input { flex: 1; min-width: 0; }
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
</style>
