<script lang="ts">
  import { onMount } from 'svelte'
  import { twoFactor, errorText, type TwoFactorSetup } from '../lib/tauri'
  import type { TwoFactorStatus } from '../lib/protocol/TwoFactorStatus'

  type View = 'status' | 'password' | 'scan' | 'codes' | 'regen' | 'disable'

  let status = $state<TwoFactorStatus | null>(null)
  let view = $state<View>('status')
  let busy = $state(false)
  let error = $state('')
  // Typed values, the pending secret and shown recovery codes live only here, and are dropped on cancel/done.
  let password = $state('')
  let code = $state('')
  let setup = $state<TwoFactorSetup | null>(null)
  let codes = $state<string[]>([])
  let signedOutNote = $state('')
  let copyNote = $state<'' | 'ok' | 'fail'>('')

  // The matrix plus a 4-module white quiet zone around it, drawn at a fixed size.
  const QUIET = 4
  const PX_TARGET = 200
  // whole pixels per module, so every module is the same size
  const scale = $derived(setup ? Math.max(1, Math.floor(PX_TARGET / (setup.qr.size + 2 * QUIET))) : 1)
  const px = $derived(setup ? (setup.qr.size + 2 * QUIET) * scale : 0)
  const dark = $derived.by(() => {
    const out: { x: number; y: number }[] = []
    if (!setup) return out
    const n = setup.qr.size
    setup.qr.modules.forEach((on, i) => { if (on) out.push({ x: (i % n) + QUIET, y: Math.floor(i / n) + QUIET }) })
    return out
  })

  const signedOut = (n: number) =>
    n === 0 ? 'No other devices were signed in.' : `Signed out ${n} other device${n === 1 ? '' : 's'}.`

  async function refresh() {
    status = null
    error = ''
    try {
      status = (await twoFactor.status()) ?? null
    } catch (err) {
      error = errorText(err)
    }
  }
  onMount(refresh)

  function reset() {
    view = 'status'
    password = ''
    code = ''
    setup = null
    codes = []
    signedOutNote = ''
    copyNote = ''
    error = ''
  }
  function open(v: View) {
    reset()
    view = v
  }

  async function run(f: () => Promise<void>) {
    busy = true
    error = ''
    try {
      await f()
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }

  const start = (e: SubmitEvent) => {
    e.preventDefault()
    return run(async () => {
      setup = await twoFactor.setup(password)
      password = ''
      view = 'scan'
    })
  }
  const enable = (e: SubmitEvent) => {
    e.preventDefault()
    return run(async () => {
      const r = await twoFactor.enable(code.trim())
      codes = r.recovery_codes
      signedOutNote = signedOut(r.signed_out)
      setup = null
      code = ''
      view = 'codes'
    })
  }
  const regenerate = (e: SubmitEvent) => {
    e.preventDefault()
    return run(async () => {
      codes = (await twoFactor.regenerate(code.trim())).recovery_codes
      signedOutNote = ''
      code = ''
      view = 'codes'
    })
  }
  const disable = (e: SubmitEvent) => {
    e.preventDefault()
    return run(async () => {
      await twoFactor.disable(code.trim())
      reset()
      await refresh()
    })
  }
  async function saved() {
    reset()
    await refresh()
  }
  async function copy() {
    try {
      if (!navigator.clipboard) throw new Error('no clipboard')
      await navigator.clipboard.writeText(codes.join('\n'))
      copyNote = 'ok'
    } catch {
      copyNote = 'fail'
    }
  }
</script>

<h3>Two-factor authentication</h3>

{#if view === 'status' && !status && error}
  <p class="err" role="alert">{error}</p>
  <button class="btn btn-secondary" onclick={refresh}>Try again</button>
{:else if view === 'status' && !status}
  <p class="hint" role="status">Checking…</p>
{:else if view === 'status' && status}
  {#if !status.available}
    <p class="hint">Two-factor authentication isn't set up on this server yet.</p>
  {:else if status.enabled}
    <p class="q">Two-factor authentication is on. {status.recovery_codes_left} recovery code{status.recovery_codes_left === 1 ? '' : 's'} left.</p>
    <div class="actions">
      <button class="btn btn-secondary" onclick={() => open('regen')}>New recovery codes</button>
      <button class="btn btn-secondary btn-danger-text" onclick={() => open('disable')}>Turn off</button>
    </div>
  {:else}
    <p class="q">Ask for a code from an authenticator app when you log in, on top of your password.</p>
    <button class="btn btn-secondary" onclick={() => open('password')}>Set up</button>
  {/if}
{:else if view === 'password'}
  <form onsubmit={start}>
    <p class="q">Enter your password to start.</p>
    <label>Password <input type="password" bind:value={password} required autocomplete="current-password" /></label>
    <div class="actions">
      <button class="btn btn-primary" disabled={busy}>Continue</button>
      <button type="button" class="btn btn-secondary" disabled={busy} onclick={reset}>Cancel</button>
    </div>
  </form>
{:else if view === 'scan' && setup}
  <form onsubmit={enable}>
    <p class="q">Scan this with an authenticator app, or type the key in. Then enter the 6-digit code it shows.</p>
    <svg class="qr" role="img" aria-label="QR code for your authenticator app" width={px} height={px}
      viewBox="0 0 {setup.qr.size + 2 * QUIET} {setup.qr.size + 2 * QUIET}" shape-rendering="crispEdges">
      <rect width={setup.qr.size + 2 * QUIET} height={setup.qr.size + 2 * QUIET} fill="#fff" />
      {#each dark as m (m.x + ',' + m.y)}
        <rect x={m.x} y={m.y} width="1" height="1" fill="#000" />
      {/each}
    </svg>
    <p class="key"><code>{setup.secret}</code></p>
    <label>Code <input bind:value={code} required inputmode="numeric" autocomplete="one-time-code" placeholder="6-digit code" /></label>
    <div class="actions">
      <button class="btn btn-primary" disabled={busy}>Turn on</button>
      <button type="button" class="btn btn-secondary" disabled={busy} onclick={reset}>Cancel</button>
    </div>
  </form>
{:else if view === 'codes'}
  <p class="q">Save these recovery codes somewhere safe. Each works once, and they won't be shown again.</p>
  <ul class="codes">
    {#each codes as c (c)}<li><code>{c}</code></li>{/each}
  </ul>
  {#if signedOutNote}<p class="ok" role="status">{signedOutNote}</p>{/if}
  <div class="actions">
    <button class="btn btn-secondary" onclick={copy}>{copyNote === 'ok' ? 'Copied' : 'Copy'}</button>
    <button class="btn btn-primary" onclick={saved}>I've saved these</button>
  </div>
  {#if copyNote === 'fail'}<p class="hint" role="status">Couldn't copy: select and copy them by hand.</p>{/if}
{:else if view === 'regen' || view === 'disable'}
  <form onsubmit={view === 'regen' ? regenerate : disable}>
    <p class="q">
      {view === 'regen'
        ? 'Enter a code from your authenticator app. Your old recovery codes stop working.'
        : 'Enter a code from your authenticator app, or a recovery code.'}
    </p>
    <label>Code
      <input bind:value={code} required autocomplete="one-time-code" autocapitalize="off" spellcheck="false"
        placeholder={view === 'regen' ? '6-digit code' : '6-digit code or recovery code'} />
    </label>
    <div class="actions">
      <button class="btn {view === 'disable' ? 'btn-secondary btn-danger-text' : 'btn-primary'}" disabled={busy}>{view === 'regen' ? 'Get new codes' : 'Turn off'}</button>
      <button type="button" class="btn btn-secondary" disabled={busy} onclick={reset}>Cancel</button>
    </div>
  </form>
{/if}
{#if view !== 'status' && error}<p class="err" role="alert">{error}</p>{/if}

<style>
  h3 { margin: 28px 0 10px; font-size: 15px; }
  form { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; font-weight: 600; color: var(--text-2); }
  input { height: 32px; width: 240px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); color: var(--text); font-size: 13px; }
  .q { margin: 0 0 10px; font-size: 13px; color: var(--text-2); }
  form .q { margin: 0; }
  .hint { margin: 6px 0 0; font-size: 13px; color: var(--text-3); }
  .actions { display: flex; gap: 8px; }
  .qr { display: block; border-radius: 6px; }
  .key { margin: 0; font-size: 13px; }
  .key code, .codes code { font-family: ui-monospace, monospace; letter-spacing: .04em; }
  .codes { list-style: none; margin: 0 0 10px; padding: 0; display: grid; grid-template-columns: repeat(2, max-content); gap: 4px 24px; font-size: 13px; }
  .ok { margin: 10px 0 10px; font-size: 13px; color: var(--text-2); }
  .err { margin: 10px 0 0; font-size: 13px; color: var(--danger-text); }
</style>
