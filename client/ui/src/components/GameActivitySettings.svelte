<script lang="ts">
  // Settings → Privacy → Game activity (#380): the master switch, the finer ones under it (each its own row, so Join,
  // achievements and clip drop can join them later), and the games added by hand. All on by default. Windows only:
  // elsewhere Pulse never looks.
  import Icon from './Icon.svelte'
  import { api, errorText } from '../lib/tauri'
  import { addGame, gameActivity, gamePrefs, removeGame, setGameSwitch, type GameSwitch } from '../lib/gamePrefs.svelte'
  const uid = $props.id()

  /** The finer switches under the master one; a new feature adds its row here and its key in GameSettings. */
  const finer: { key: Exclude<GameSwitch, 'enabled'>; label: string; help: string }[] = [
    { key: 'art', label: 'Show game art', help: "Steam games show their art, which your Pulse server fetches. Off, the game's Steam id isn't sent." },
    { key: 'time', label: 'Show play time', help: "Others see how long you've been playing. Off, it isn't sent." },
  ]

  // "Add a game": the picked exe, waiting for its name
  let picked = $state<string | null>(null)
  let name = $state('')
  let error = $state('')
  async function pick() {
    error = ''
    try {
      const got = await api.pickGameExe()
      if (got) {
        name = got.name
        picked = got.exe
      }
    } catch (e) {
      error = errorText(e)
    }
  }
  function save(e: SubmitEvent) {
    e.preventDefault()
    if (!picked || !name.trim()) return
    addGame(name, picked)
    picked = null
  }
  const fileName = (exe: string) => exe.split(/[\\/]/).pop() ?? exe
</script>

<div class="settings-field game-activity">
  <h3 class="settings-section-title">Game activity</h3>
  <label class="row">
    <input type="checkbox" role="switch" class="switch" data-switch="enabled" checked={gamePrefs.enabled} aria-describedby="{uid}-on-help"
      onchange={(e) => setGameSwitch('enabled', e.currentTarget.checked)} />
    <span>Game activity</span>
  </label>
  <small id="{uid}-on-help">Everyone who can see you online sees "Playing" with the game you're in. Works on Windows. Off, Pulse doesn't look at all and sends nothing.</small>
  {#each finer as f (f.key)}
    <label class="row sub">
      <input type="checkbox" role="switch" class="switch" data-switch={f.key} checked={gamePrefs[f.key]} disabled={!gamePrefs.enabled} aria-describedby="{uid}-{f.key}-help"
        onchange={(e) => setGameSwitch(f.key, e.currentTarget.checked)} />
      <span>{f.label}</span>
    </label>
    <small class="sub" id="{uid}-{f.key}-help">{f.help}</small>
  {/each}
  {#if gamePrefs.enabled && gameActivity.detecting}
    <p class="seen" aria-live="polite">{gameActivity.playing ? `Pulse sees you playing ${gameActivity.playing}` : 'No game running right now.'}</p>
  {/if}

  <h4 class="games-title">Games added by hand</h4>
  <p class="hint">Steam games are found on their own. For anything else, pick its .exe: Pulse spots it running and shows the name you give it. The file's location stays on this computer.</p>
  {#if gamePrefs.games.length}
    <ul class="settings-list">
      {#each gamePrefs.games as g (g.exe)}
        <li class="settings-list-item">
          <span class="who"><span class="nm">{g.name}</span><small data-tip={g.exe}>{fileName(g.exe)}</small></span>
          <button type="button" class="btn btn-secondary" aria-label="Remove {g.name}" onclick={() => removeGame(g.exe)}>Remove</button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if picked}
    <form class="add-form" onsubmit={save}>
      <label class="name-field">Name it <input class="input input-sm" aria-label="Game name" bind:value={name} maxlength="100" required /></label>
      <small data-tip={picked}>{fileName(picked)}</small>
      <div class="actions">
        <button type="submit" class="btn btn-primary" disabled={!name.trim()}>Add</button>
        <button type="button" class="btn btn-secondary" onclick={() => (picked = null)}>Cancel</button>
      </div>
    </form>
  {:else}
    <button type="button" class="btn btn-secondary add" onclick={() => void pick()}><Icon name="plus" size={14} /> Add a game</button>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  .sub { margin-left: 24px; }
  .seen { color: var(--text-3); }
  .games-title { margin: 8px 0 0; font-size: 13px; font-weight: 600; }
  .add { align-self: flex-start; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .nm, .who small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .nm { font-weight: 600; }
  .who small, .add-form small { color: var(--text-3); }
  .add-form { display: flex; flex-direction: column; gap: 6px; }
  .name-field { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .actions { display: flex; gap: 6px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
