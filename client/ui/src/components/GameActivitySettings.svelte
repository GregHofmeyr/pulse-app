<script lang="ts">
  // Settings → Privacy → Game activity (#380): the master switch, the finer ones under it (each its own row, so Join
  // can join them later), clip drop's watched folders (#382), the Steam account and its achievements (#379), and the
  // games added by hand. All on by default. Windows only: elsewhere Pulse never looks.
  import Icon from './Icon.svelte'
  import { api, errorText } from '../lib/tauri'
  import { addGame, gameActivity, gamePrefs, removeGame, setClipFolders, setGameSwitch, type GameSwitch } from '../lib/gamePrefs.svelte'
  import { isWindows } from '../lib/stream/platform'
  import { linkSteam, loadSteam, setSteamHours, setSteamJoin, setSteamShare, steam, steamNote, unlinkSteam } from '../lib/steam.svelte'
  import { completionLine, hoursText } from '../lib/achievements'
  const uid = $props.id()

  // Steam (#379): the link and "Share Steam achievements", which the server keeps (it does the reading)
  $effect(() => {
    if (!steam.status) void loadSteam()
  })
  const steamLink = $derived(steam.status?.link ?? null)
  const steamHint = $derived(steamNote(steam.status))
  const steamRate = $derived(steamLink?.share_achievements ? completionLine(steamLink.completion_rate, steamLink.perfect_games) : null)
  // Steam hours (#383): on unless the server says off (an older server never says)
  const shareHours = $derived(!!steamLink && steamLink.share_hours !== false)
  const steamHours = $derived(shareHours && steamLink?.recent_minutes != null ? `${hoursText(steamLink.recent_minutes)} in Steam games over the last two weeks` : null)
  // Join (#381): the same rule
  const shareJoin = $derived(!!steamLink && steamLink.share_join !== false)
  const steamOff = $derived(!gamePrefs.enabled || !steamLink || !steam.status?.achievements_available || steam.busy)

  /** The finer switches under the master one; a new feature adds its row here and its key in GameSettings. */
  const finer: { key: Exclude<GameSwitch, 'enabled'>; label: string; help: string }[] = [
    { key: 'art', label: 'Show game art', help: "Steam games show their art, which your Pulse server fetches. Off, the game's Steam id isn't sent." },
    { key: 'time', label: 'Show play time', help: "Others see how long you've been playing. Off, it isn't sent." },
    { key: 'clips', label: 'Offer new recordings as clips', help: 'When NVIDIA or Game Bar saves a recording in a watched folder, Pulse offers to share it to a clips channel. Nothing is sent unless you share it.' },
  ]

  // Clip drop's folders (#382): the chosen ones, else the core's defaults under Videos (asked once, on Windows).
  const windows = isWindows()
  let defaultFolders = $state<string[]>([])
  $effect(() => {
    if (!windows) return
    let gone = false
    api.clipDropDefaults().then((d) => !gone && (defaultFolders = d), () => {})
    return () => (gone = true)
  })
  const folders = $derived(gamePrefs.clip_folders ?? defaultFolders)
  async function addFolder() {
    error = ''
    try {
      const got = await api.pickClipFolder()
      if (got && !folders.some((f) => f.toLowerCase() === got.toLowerCase())) setClipFolders([...folders, got])
    } catch (e) {
      error = errorText(e)
    }
  }
  const removeFolder = (f: string) => setClipFolders(folders.filter((x) => x !== f))

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
  {#if windows && gamePrefs.enabled && gamePrefs.clips}
    <div class="sub folders">
      <h4 class="games-title">Watched folders</h4>
      <p class="hint">New videos here and in their folders one level down (NVIDIA saves each game in its own) are offered.</p>
      {#if folders.length}
        <ul class="settings-list">
          {#each folders as f (f)}
            <li class="settings-list-item">
              <span class="who"><small class="path" data-tip={f}>{f}</small></span>
              <button type="button" class="btn btn-secondary" aria-label="Stop watching {f}" onclick={() => removeFolder(f)}>Remove</button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="hint">No folders: nothing is offered.</p>
      {/if}
      <div class="actions">
        <button type="button" class="btn btn-secondary" onclick={() => void addFolder()}><Icon name="plus" size={14} /> Add a folder</button>
        {#if gamePrefs.clip_folders}
          <button type="button" class="btn btn-secondary" onclick={() => setClipFolders(null)}>Use the defaults</button>
        {/if}
      </div>
    </div>
  {/if}
  {#if steam.status}
    <div class="steam" data-steam>
      <h4 class="games-title">Steam account</h4>
      {#if steamLink}
        <div class="settings-list-item">
          <span class="who"><span class="nm">{steamLink.persona ?? 'Steam account linked'}</span>{#if steamLink.persona}<small>Linked Steam account</small>{/if}</span>
          <button type="button" class="btn btn-secondary" disabled={steam.busy} onclick={() => void unlinkSteam()}>Unlink</button>
        </div>
      {:else}
        <p class="hint">Opens Steam's sign-in in your browser. Pulse never sees your Steam password.</p>
        <button type="button" class="btn btn-secondary add" disabled={!steam.status.linking_available || steam.busy} onclick={() => void linkSteam()}>Link Steam account</button>
      {/if}
      <label class="row sub">
        <input type="checkbox" role="switch" class="switch" data-switch="steam" checked={!!steamLink?.share_achievements} aria-describedby="{uid}-steam-help"
          disabled={steamOff}
          onchange={(e) => void setSteamShare(e.currentTarget.checked)} />
        <span>Share Steam achievements</span>
      </label>
      <small class="sub" id="{uid}-steam-help">While you play, new unlocks and 100 % completions post in the Achievements channel of servers that have one, and your completion rate shows on your profile. Off, your Pulse server stops reading them.</small>
      {#if steamHint}<p class="note sub" role="status">{steamHint}</p>{/if}
      {#if steamRate}<p class="seen sub">{steamRate}</p>{/if}
      <label class="row sub">
        <input type="checkbox" role="switch" class="switch" data-switch="steam-hours" checked={shareHours} aria-describedby="{uid}-steam-hours-help"
          disabled={steamOff} onchange={(e) => void setSteamHours(e.currentTarget.checked)} />
        <span>Share Steam hours</span>
      </label>
      <small class="sub" id="{uid}-steam-hours-help">Your hours in Steam games over the last two weeks show on your profile and your servers' leaderboards, and count toward Touch Grass. Off, your Pulse server stops reading them.</small>
      {#if steamHours}<p class="seen sub" data-steam-hours>{steamHours}</p>{/if}
      <label class="row sub">
        <input type="checkbox" role="switch" class="switch" data-switch="steam-join" checked={shareJoin} aria-describedby="{uid}-steam-join-help"
          disabled={steamOff} onchange={(e) => void setSteamJoin(e.currentTarget.checked)} />
        <span>Let friends join your game</span>
      </label>
      <small class="sub" id="{uid}-steam-join-help">When Steam shows your lobby or game server publicly, people who can see you playing get Join on your profile card and in your right-click menu. That shares the lobby or the server's address with them. Off, your Pulse server stops looking.</small>
      {#if steam.error}<p class="error" role="alert">{steam.error}</p>{/if}
    </div>
  {/if}
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
  .folders, .steam { display: flex; flex-direction: column; gap: 6px; }
  .note { color: var(--warn-text); }
  .who .path { color: var(--text-2); }
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
