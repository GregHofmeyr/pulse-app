<script lang="ts">
  // Asks for a nickname (#48): someone's server nickname (anyone in the server may set or reset it;
  // everyone there sees it), or the private one only you see. Blank and Save, or Reset, is no
  // nickname. The rules are the server's, checked here as you type (in characters, as it counts).
  import { untrack } from 'svelte'
  import { modal } from '../lib/actions'
  import { app } from '../lib/store.svelte'
  import { baseName, serverNick, userOf } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'
  import { nameProblem } from '../lib/profileDraft'
  import type { RenameTarget } from '../lib/profileCard.svelte'

  let { target, onClose }: { target: RenameTarget; onClose: () => void } = $props()

  const server = $derived(target.kind === 'server' ? app.state.servers.find((s) => s.id === target.serverId) : undefined)
  const current = $derived(target.kind === 'server' ? serverNick(app.state, target.serverId, target.userId) : (app.state.privateNicks[target.userId] ?? null))
  // the name they'd have without this nickname: in a server, your private one then their display
  // name (the precedence, one layer down); a private one sits on their display name
  const base = $derived.by(() => {
    const u = userOf(app.state, target.userId)
    return u ? baseName(u) : 'them'
  })
  const without = $derived(target.kind === 'server' ? (app.state.privateNicks[target.userId] ?? base) : base)
  const self = $derived(target.userId === app.state.me?.id)
  const title = $derived(target.kind === 'private' ? 'Set private nickname' : self ? 'Change your nickname' : 'Change nickname')
  const hint = $derived(
    target.kind === 'private'
      ? `Only you see this name for ${without}, everywhere.`
      : `Everyone in ${server?.name ?? 'this server'} sees it there. Every nickname goes in the Name Archive.`,
  )

  let value = $state(untrack(() => current ?? ''))
  let busy = $state(false)
  let error = $state('')
  // blank is a reset, so only a name typed can be wrong
  const problem = $derived(value.trim() ? nameProblem(value) : null)

  async function save(nickname: string | null) {
    if (busy || (nickname !== null && problem)) return
    busy = true
    error = ''
    const n = nickname?.trim() || null
    try {
      if (target.kind === 'server') await api.setNickname(target.serverId, target.userId, n)
      else await api.setPrivateNickname(target.userId, n)
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  const cancel = () => {
    if (!busy) onClose()
  }
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={cancel}>
    <h1 class="modal-title">{title}</h1>
    <p>{hint}</p>
    <form onsubmit={(e) => { e.preventDefault(); void save(value) }}>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input" aria-label="Nickname" aria-invalid={!!problem} bind:value placeholder={without} autofocus disabled={busy} />
      {#if problem}<p class="err" role="alert">{problem}</p>{:else if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="actions">
        {#if current}<button type="button" class="btn btn-lg btn-secondary reset" disabled={busy} onclick={() => save(null)}>{target.kind === 'private' ? 'Clear' : 'Reset'}</button>{/if}
        <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
        <button type="submit" class="btn btn-lg btn-primary" disabled={busy || !!problem}>Save</button>
      </div>
    </form>
  </div>
</div>

<style>
  /* above the profile card and full profile it opens from (stacking.test.ts) */
  .overlay { position: fixed; inset: 0; z-index: 70; }
  .modal { width: min(400px, 100%); }
  form { display: flex; flex-direction: column; gap: 12px; }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .reset { margin-right: auto; }
</style>
