<script lang="ts">
  import { onMount } from 'svelte'
  import { api, twoFactor, errorText } from '../lib/tauri'

  // Unknown (null) when the 2FA state couldn't be read: the code field is offered, optional.
  let twoFactorOn = $state<boolean | null>(null)
  let open = $state(false)
  let busy = $state(false)
  let error = $state('')
  let done = $state(false)
  // Typed passwords live only here, and are dropped on cancel, success and close.
  let current = $state('')
  let next = $state('')
  let confirm = $state('')
  let code = $state('')

  onMount(async () => {
    try {
      twoFactorOn = (await twoFactor.status())?.enabled ?? null
    } catch {
      twoFactorOn = null
    }
  })

  function clear() {
    current = ''
    next = ''
    confirm = ''
    code = ''
    error = ''
  }
  function start() {
    clear()
    done = false
    open = true
  }
  function cancel() {
    clear()
    open = false
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    error = ''
    if (next !== confirm) {
      error = "The new passwords don't match."
      return
    }
    busy = true
    try {
      await api.changePassword(current, next, twoFactorOn === false ? null : code.trim() || null)
      clear()
      open = false
      done = true
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }
</script>

<h3>Password</h3>
{#if open}
  <form onsubmit={submit}>
    <label>Current password <input type="password" bind:value={current} required autocomplete="current-password" /></label>
    <label>New password <input type="password" bind:value={next} required minlength="8" autocomplete="new-password" /></label>
    <label>Confirm new password <input type="password" bind:value={confirm} required autocomplete="new-password" /></label>
    {#if twoFactorOn !== false}
      <label>Code
        <input bind:value={code} required={twoFactorOn === true} autocomplete="one-time-code" autocapitalize="off" spellcheck="false"
          placeholder={twoFactorOn ? '6-digit code or recovery code' : 'only if two-factor is on'} />
      </label>
    {/if}
    <p class="q">Every other device signed in to this account goes back to the login screen. This one stays signed in.</p>
    <div class="actions">
      <button disabled={busy}>Change</button>
      <button type="button" disabled={busy} onclick={cancel}>Cancel</button>
    </div>
  </form>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
{:else}
  <button onclick={start}>Change password</button>
  {#if done}<p class="ok" role="status">Password changed. Other devices were signed out.</p>{/if}
{/if}

<style>
  h3 { margin: 28px 0 10px; font-size: 15px; }
  form { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; font-weight: 600; color: var(--text-2); }
  input { height: 32px; width: 240px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); color: var(--text); font-size: 13px; }
  .q { margin: 0; font-size: 13px; color: var(--text-2); }
  .actions { display: flex; gap: 8px; }
  button { height: 32px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: 8px; background: transparent; color: var(--text); font-size: 13px; }
  button:hover { background: var(--bg-3); }
  button:disabled { opacity: .6; }
  .ok { margin: 10px 0 0; font-size: 13px; color: var(--text-2); }
  .err { margin: 10px 0 0; font-size: 13px; color: #f2616b; }
</style>
