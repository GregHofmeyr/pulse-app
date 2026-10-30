<script lang="ts">
  import { diagUi, setDiagOverlay } from '../lib/stream/diag.svelte'
  import { onMount } from 'svelte'
  import { api, errorText } from '../lib/tauri'
  import TwoFactorSettings from './TwoFactorSettings.svelte'
  import ChangePassword from './ChangePassword.svelte'

  // null until the saved value is known: the box can't be changed before then
  let autoSend = $state<boolean | null>(null)
  let reportError = $state('')
  onMount(async () => {
    try {
      autoSend = (await api.getReportSettings()).auto_send_crashes
    } catch (err) {
      reportError = errorText(err)
    }
  })
  async function setAutoSend(on: boolean) {
    const before = autoSend
    autoSend = on
    reportError = ''
    try {
      autoSend = (await api.setReportSettings({ auto_send_crashes: on })).auto_send_crashes
    } catch (err) {
      autoSend = before
      reportError = errorText(err)
    }
  }

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

<section class="settings-pane">
  <h2>Advanced</h2>
  <div class="settings-field">
    <label class="row">
      <input type="checkbox" checked={diagUi.on} onchange={(e) => setDiagOverlay(e.currentTarget.checked)} />
      <span>Stream diagnostics</span>
    </label>
    <p class="hint">Shows live numbers on streams you share or watch. They are always written to the log for bug reports.</p>
  </div>

  <div class="settings-field">
    <label class="row">
      <input type="checkbox" checked={autoSend ?? false} disabled={autoSend === null}
        onchange={(e) => setAutoSend(e.currentTarget.checked)} />
      <span>Send crash reports automatically</span>
    </label>
    <p class="hint">If Pulse crashes or closes unexpectedly, a report with the app log goes to the server the next time you sign in. Off: it is saved on this computer and you're asked whether to send it.</p>
    {#if reportError}<p class="err" role="alert">{reportError}</p>{/if}
  </div>

  <div>
    <h3>Sessions</h3>
    {#if confirming}
      <p class="q">Are you sure? Every other device signed in to this account goes back to the login screen. This one stays signed in.</p>
      <div class="actions">
        <button class="btn btn-secondary btn-danger-text" disabled={busy} onclick={signOutOthers}>Sign out</button>
        <button class="btn btn-secondary" disabled={busy} onclick={() => (confirming = false)}>Cancel</button>
      </div>
    {:else}
      <button class="btn btn-secondary btn-danger-text" onclick={() => { confirming = true; result = ''; error = '' }}>Sign out all other devices</button>
    {/if}
    {#if result}<p class="ok" role="status">{result}</p>{/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </div>

  <div><ChangePassword /></div>

  <div><TwoFactorSettings /></div>
</section>

<style>
  /* the pane's 20px gap plus this: the 28px between sections it had */
  h3 { margin: 8px 0 10px; font-size: 15px; }
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  /* under the checkbox's label, not the box */
  .hint { margin: -2px 0 0 26px; font-size: 12px; color: var(--text-3); }
  .q { margin: 0 0 10px; font-size: 13px; color: var(--text-2); }
  .actions { display: flex; gap: 8px; }
  .ok { margin: 10px 0 0; font-size: 13px; color: var(--text-2); }
  .err { margin: 2px 0 0; font-size: 13px; color: var(--danger-text); }
  .settings-field .err { margin: 0 0 0 26px; }
</style>
