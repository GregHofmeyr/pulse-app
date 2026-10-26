<script lang="ts">
  import { diagUi, setDiagOverlay } from '../lib/stream/diag.svelte'
  import { api, errorText } from '../lib/tauri'

  let confirming = $state(false)
  let busy = $state(false)
  let result = $state('')
  let error = $state('')

  const signedOut = (n: number) =>
    n === 0 ? 'No other devices were signed in.' : `Signed out ${n} other device${n === 1 ? '' : 's'}.`

  async function signOutOthers() {
    busy = true
    error = ''
    result = ''
    try {
      result = signedOut(await api.logoutOthers())
      confirming = false
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }
</script>

<h2>Advanced</h2>
<label class="row">
  <input type="checkbox" checked={diagUi.on} onchange={(e) => setDiagOverlay(e.currentTarget.checked)} />
  <span>Stream diagnostics</span>
</label>
<p class="hint">Shows live numbers on streams you share or watch. They are always written to the log for bug reports.</p>

<h3>Sessions</h3>
{#if confirming}
  <p class="q">Are you sure? Every other device signed in to this account goes back to the login screen. This one stays signed in.</p>
  <div class="actions">
    <button class="danger" disabled={busy} onclick={signOutOthers}>Sign out</button>
    <button disabled={busy} onclick={() => (confirming = false)}>Cancel</button>
  </div>
{:else}
  <button class="danger" onclick={() => { confirming = true; result = ''; error = '' }}>Sign out all other devices</button>
{/if}
{#if result}<p class="ok" role="status">{result}</p>{/if}
{#if error}<p class="err" role="alert">{error}</p>{/if}

<style>
  h2 { margin: 0 0 16px; font-size: 20px; }
  h3 { margin: 28px 0 10px; font-size: 15px; }
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  .hint { margin: 6px 0 0 26px; font-size: 13px; color: var(--text-3); }
  .q { margin: 0 0 10px; font-size: 13px; color: var(--text-2); }
  .actions { display: flex; gap: 8px; }
  button { height: 32px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: 8px; background: transparent; color: var(--text); font-size: 13px; }
  button:hover { background: var(--bg-3); }
  button:disabled { opacity: .6; }
  button.danger { color: #f2616b; }
  .ok { margin: 10px 0 0; font-size: 13px; color: var(--text-2); }
  .err { margin: 10px 0 0; font-size: 13px; color: #f2616b; }
</style>
