<script lang="ts">
  // Server settings → Invites (#80): the server's invites, newest first: its code, who made it (and whom a direct
  // invite is for), uses, when it expires, and Revoke while it still lets someone in. The owner sees everyone's, a
  // member their own (the server decides). Asked when the pane opens and again on InvitesChanged (made, used, revoked
  // anywhere). "+ Invite people" opens the Invite dialog.
  import { untrack } from 'svelte'
  import Name from './Name.svelte'
  import Icon from './Icon.svelte'
  import { app, clock, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName, isMember } from '../lib/selectors'
  import { expiresText, usesText } from '../lib/serverInvites'
  import type { Server } from '../lib/protocol/Server'
  import type { ServerInvite } from '../lib/protocol/ServerInvite'

  let { server }: { server: Server } = $props()

  const owner = $derived(server.owner_id === app.state.me?.id)
  let invites = $state<ServerInvite[] | null>(null)
  let error = $state('')
  let asked = 0
  async function load() {
    const ask = ++asked
    try {
      const got = await api.listServerInvites(server.id)
      if (ask === asked) { invites = got ?? []; error = '' }
    } catch (e) {
      if (ask === asked) error = errorText(e)
    }
  }
  const rev = $derived(app.state.invitesRev[server.id] ?? 0)
  const member = $derived(isMember(app.state, server.id))
  $effect(() => {
    void rev
    if (member) untrack(() => void load())
  })
  const STATE: Record<string, string> = { expired: 'Expired', used_up: 'Used', revoked: 'Revoked' }
  async function revoke(code: string) {
    error = ''
    try {
      await api.revokeInvite(code)
      invites = (invites ?? []).map((i) => (i.code === code ? { ...i, state: 'revoked' } : i))
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<section class="settings-pane" data-server-invites>
  <h2>Invites</h2>
  <p class="hint">{owner ? 'Every invite to this server.' : 'The invites you made.'}</p>
  {#if member}
    <button type="button" class="btn btn-secondary add" onclick={() => (ui.inviteTo = server.id)}><Icon name="plus" size={16} /> Invite people</button>
  {/if}
  {#if invites && invites.length}
    <ul class="list" aria-label="Invites to {server.name}">
      {#each invites as i (i.code)}
        {@const live = i.state === 'valid'}
        <li class="row" class:dead={!live} data-invite={i.code}>
          <span class="code">{i.code}</span>
          <span class="who">
            <Name userId={i.created_by} serverId={server.id} text={displayName(app.state, server.id, i.created_by)} interactive={false} />
            {#if i.for_user}<small>for {displayName(app.state, server.id, i.for_user)}</small>{/if}
          </span>
          <span class="uses" data-tip="Uses">{usesText(i.uses, i.max_uses)}</span>
          <span class="exp">{live ? expiresText(i.expires_at, Date.parse(clock.nowIso)) : STATE[i.state] ?? ''}</span>
          {#if live}<button type="button" class="btn btn-secondary btn-danger-text" onclick={() => void revoke(i.code)}>Revoke</button>{:else}<span class="gap"></span>{/if}
        </li>
      {/each}
    </ul>
  {:else if invites}
    <p class="empty">No invites yet.</p>
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</section>

<style>
  .hint { margin: -12px 0 0; color: var(--text-3); font-size: 12px; }
  .add { align-self: flex-start; }
  .list { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; }
  .row { min-height: 44px; padding: 6px 10px; border-radius: var(--radius-md); display: grid; grid-template-columns: 110px minmax(0, 1fr) 56px 96px 80px; align-items: center; gap: 12px; font-size: 13px; }
  .row:hover { background: var(--bg-2); }
  .row.dead { color: var(--text-3); }
  .code { font-weight: 600; letter-spacing: .02em; }
  .who { min-width: 0; display: flex; flex-direction: column; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .uses, .exp { color: var(--text-2); }
  .dead .uses, .dead .exp { color: var(--text-3); }
  .empty { margin: 0; color: var(--text-3); font-size: 13px; }
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
</style>
