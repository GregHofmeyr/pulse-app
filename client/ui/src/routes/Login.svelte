<script lang="ts">
  import { api, errorText } from '../lib/tauri'
  import type { User } from '../lib/protocol/User'

  let { notice = '', onAuthed }: { notice?: string; onAuthed: (u: User) => void } = $props()

  let mode = $state<'login' | 'register'>('login')
  let serverUrl = $state('http://localhost:7890')
  let inviteCode = $state('')
  let username = $state('')
  let password = $state('')
  let busy = $state(false)
  let error = $state('')

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    busy = true
    error = ''
    try {
      const u =
        mode === 'login'
          ? await api.login(serverUrl, username.trim(), password)
          : await api.register(serverUrl, inviteCode.trim(), username.trim(), password)
      onAuthed(u)
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }
</script>

<main data-tauri-drag-region>
  <form class="card" onsubmit={submit}>
    <div class="brand">
      <span class="logo" aria-hidden="true">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
      </span>
      <h1>{mode === 'login' ? 'Welcome back' : 'Join Pulse'}</h1>
    </div>
    {#if notice}<p class="notice">{notice}</p>{/if}

    <label>Server <input bind:value={serverUrl} required autocomplete="url" /></label>
    {#if mode === 'register'}
      <label>Invite code <input bind:value={inviteCode} required autocomplete="off" /></label>
    {/if}
    <label>Username <input bind:value={username} required autocomplete="username" /></label>
    <label>Password
      <input type="password" bind:value={password} required minlength={mode === 'register' ? 8 : undefined}
        autocomplete={mode === 'login' ? 'current-password' : 'new-password'} />
    </label>

    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button class="primary" disabled={busy}>{busy ? '…' : mode === 'login' ? 'Log in' : 'Create account'}</button>
    <button type="button" class="link" onclick={() => { mode = mode === 'login' ? 'register' : 'login'; error = '' }}>
      {mode === 'login' ? 'Got an invite? Create an account' : 'Already have an account? Log in'}
    </button>
  </form>
</main>

<style>
  main { height: 100%; display: grid; place-items: center; background: var(--bg-0); }
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
</style>
