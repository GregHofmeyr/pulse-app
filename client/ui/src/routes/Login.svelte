<script lang="ts">
  import { api, errorText } from '../lib/tauri'
  import { defaultServer } from '../lib/config'
  import type { User } from '../lib/protocol/User'
  import ReportBug from '../components/ReportBug.svelte'

  let { notice = '', onAuthed }: { notice?: string; onAuthed: (u: User) => void } = $props()

  let mode = $state<'login' | 'register'>('login')
  let serverUrl = $state(defaultServer(import.meta.env))
  let inviteCode = $state('')
  let username = $state('')
  let password = $state('')
  let busy = $state(false)
  let error = $state('')
  let reportOpen = $state(false)
  // second step: the password is already gone from state; the core holds the ticket
  let step = $state<'password' | 'code'>('password')
  let code = $state('')
  const EXPIRED = 'That sign-in expired. Log in again.'

  function back(msg = '') {
    // the core drops the ticket too; nothing to wait for (or show) if that fails
    api.cancelLogin().catch(() => {})
    step = 'password'
    code = ''
    password = ''
    error = msg
  }

  async function verify(e: SubmitEvent) {
    e.preventDefault()
    busy = true
    error = ''
    try {
      const r = await api.loginMfa(code.trim())
      if (r.status === 'ready') onAuthed(r.user)
      else back(EXPIRED)
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    busy = true
    error = ''
    try {
      if (mode === 'login') {
        const r = await api.login(serverUrl, username.trim(), password)
        if (r.status === 'ready') onAuthed(r.user)
        else if (r.status === 'mfa_required') {
          password = ''
          code = ''
          step = 'code'
        } else back(EXPIRED)
      } else {
        onAuthed(await api.register(serverUrl, inviteCode.trim(), username.trim(), password))
      }
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }
</script>

<main data-tauri-drag-region>
  <form class="card" onsubmit={step === 'code' ? verify : submit}>
    <div class="brand">
      <span class="logo" aria-hidden="true">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
      </span>
      <h1>{step === 'code' ? 'Two-factor authentication' : mode === 'login' ? 'Welcome back' : 'Join Pulse'}</h1>
    </div>
    {#if notice}<p class="notice">{notice}</p>{/if}

    {#if step === 'code'}
      <label>Code
        <input bind:value={code} required placeholder="6-digit code or recovery code" autocomplete="one-time-code"
          inputmode="text" autocapitalize="off" spellcheck="false" />
      </label>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <button class="primary" disabled={busy}>{busy ? '…' : 'Verify'}</button>
      <button type="button" class="link" disabled={busy} onclick={() => back()}>Back</button>
    {:else}
      <label>Server <input bind:value={serverUrl} required autocomplete="url" /></label>
      {#if mode === 'register'}
        <label>Invite code <input bind:value={inviteCode} required autocomplete="off" /></label>
      {/if}
      <label>Username <input bind:value={username} required autocomplete="username" autocapitalize="off" spellcheck="false" /></label>
      <label>Password
        <input type="password" bind:value={password} required minlength={mode === 'register' ? 8 : undefined}
          autocomplete={mode === 'login' ? 'current-password' : 'new-password'} />
      </label>

      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <button class="primary" disabled={busy}>{busy ? '…' : mode === 'login' ? 'Log in' : 'Create account'}</button>
      <button type="button" class="link" onclick={() => { mode = mode === 'login' ? 'register' : 'login'; error = '' }}>
        {mode === 'login' ? 'Got an invite? Create an account' : 'Already have an account? Log in'}
      </button>
    {/if}
  </form>
  <button type="button" class="report" onclick={() => (reportOpen = true)}>Report a bug</button>
  {#if reportOpen}<ReportBug saveOnly {serverUrl} onClose={() => (reportOpen = false)} />{/if}
</main>

<style>
  main { height: 100%; display: grid; place-items: center; align-content: center; gap: 14px; background: var(--bg-0); }
  .card {
    width: 360px; padding: 28px; border-radius: 18px; background: var(--bg-2);
    border: 1px solid var(--line); display: flex; flex-direction: column; gap: 14px;
  }
  .brand { display: flex; align-items: center; gap: 12px; margin-bottom: 6px; }
  .logo { width: 34px; height: 34px; border-radius: 10px; background: var(--accent); color: var(--on-accent); display: grid; place-items: center; }
  h1 { margin: 0; font-size: 20px; font-weight: 600; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  input { height: 40px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-1); font-size: 14px; font-weight: 400; letter-spacing: 0; }
  .primary { height: 42px; border: 0; border-radius: 10px; background: var(--accent); color: var(--on-accent); font-weight: 600; margin-top: 4px; }
  .primary:disabled { opacity: .6; }
  .link { background: none; border: 0; color: var(--text-3); font-size: 13px; }
  .link:hover { color: var(--text); }
  .error { margin: 0; color: #f2616b; font-size: 13px; }
  .notice { margin: 0; color: #e8b04a; font-size: 13px; }
  .report { background: none; border: 0; color: var(--warn); font-size: 13px; opacity: .8; }
  .report:hover { opacity: 1; }
</style>
