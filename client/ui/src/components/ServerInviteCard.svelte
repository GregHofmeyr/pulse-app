<script lang="ts">
  // An invite to a server (#80), as a card pack (#511): a direct invite's message, or an invite link pasted in a
  // message. Someone it's for gets it sealed: a foil pack in the server's colours, crimped and shaded, with its icon in
  // a window, "PULSE · SERVER PACK", its name and who it's from. Holding and dragging across the top tears it (a
  // glowing zigzag follows, the strip lifts; let go early and it springs back); at TEAR_AT of the width it rips, the
  // strip flies off and a holo card rises out with a gold burst. Enter or Space opens it too. Its sender sees the card
  // already open, and a dead invite is the card greyed with why. With reduced motion it arrives sealed too, held still,
  // and a click, Enter or a short drag opens it at once: no flight, burst or rise, the card is just there (#538). Once
  // opened it stays open on this device (lib/invitePack). The space is reserved up front, so nothing shifts.
  //
  // The card: "✦ MYTHICAL PULL ✦" (always: there are no tiers), the banner (or the server's colours), the ringed icon,
  // its name, who it's from with "N online · M members", and Join (the invite's own words), Open once you're in. What
  // it shows is per viewer, asked of the server once the card is on screen and kept for the session
  // (lib/invitePreviews.svelte): lookups are rate limited, so it never asks in a loop.
  import { onDestroy } from 'svelte'
  import { isMember } from '../lib/selectors'
  import { app, ui } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { invitePreviews, loadInvitePreview } from '../lib/invitePreviews.svelte'
  import { acceptInvite } from '../lib/inviteActions.svelte'
  import { cardView, countsLine, type CardState } from '../lib/serverInvites'
  import { avatarColors } from '../lib/avatar'
  import { NO_TEAR, RIP_MS, markOpened, openingStage, press, prefersReducedMotion, progressOf, ripAt, startStage, stretch, wasOpened, type PackStage, type Tear } from '../lib/invitePack'
  import { PACK } from '../lib/packColors'
  import ServerChip from './ServerChip.svelte'
  import ServerBanner from './ServerBanner.svelte'

  let { code }: { code: string } = $props()

  let root = $state<HTMLElement>()
  // ask once it's on screen (or at once where there's no IntersectionObserver)
  $effect(() => {
    const el = root
    const c = code
    if (!el) return
    if (typeof IntersectionObserver !== 'function') return loadInvitePreview(c)
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        loadInvitePreview(c)
        io.disconnect()
      }
    })
    io.observe(el)
    return () => io.disconnect()
  })

  const entry = $derived(invitePreviews.byCode[code])
  const preview = $derived(entry?.status === 'ok' ? entry.preview : null)
  /** As you see it now: joined since it was asked (here or on another device), the server's members say so. */
  const shown = $derived.by((): CardState | null => {
    if (!entry || entry.status === 'loading') return null
    if (entry.status === 'gone') return 'not_found'
    if (entry.status === 'error') return 'unavailable'
    const p = entry.preview
    return isMember(app.state, p.server_id) ? 'already_member' : p.state
  })
  const view = $derived(shown ? cardView(shown, preview ?? undefined) : null)
  const mine = $derived(!!preview && preview.inviter_id === app.state.me?.id)
  const colors = $derived(avatarColors(preview?.server_id ?? code))
  const from = $derived(preview ? (mine ? 'you' : preview.inviter_name) : '')

  // --- the pack ---
  // Opened on this device before (read once: opening it here plays the rip, then it stays open), and less motion asked.
  // svelte-ignore state_referenced_locally
  const openedBefore = wasOpened(code)
  const reduced = prefersReducedMotion()
  /** How this card starts, once its preview is in. */
  const start = $derived(preview && shown ? startStage({ mine, valid: shown === 'valid', opened: openedBefore }) : null)
  /** Opening it here: tearing done, then open. */
  let phase = $state<'idle' | 'ripping' | 'done'>('idle')
  const stage = $derived<PackStage | null>(start === 'sealed' ? (phase === 'ripping' ? 'ripping' : phase === 'done' ? 'open' : 'sealed') : start)
  let tear = $state<Tear>(NO_TEAR)
  const progress = $derived(progressOf(tear))
  /** Shown as it opens (the burst, the rise): not for a card that was already open. */
  let opening = $state(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  onDestroy(() => clearTimeout(timer))

  function open() {
    if (stage !== 'sealed') return
    tear = NO_TEAR
    markOpened(code)
    if (openingStage(reduced) === 'open') {
      // reduced motion (#538): the card is simply there, no rip, burst or rise
      phase = 'done'
      return
    }
    phase = 'ripping'
    opening = true
    clearTimeout(timer)
    timer = setTimeout(() => (phase = 'done'), RIP_MS)
  }
  const rel = (e: PointerEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    return { x: (e.clientX - r.left) / (r.width || 1), y: ((e.clientY - r.top) * 400) / (r.height || 400) }
  }
  function down(e: PointerEvent) {
    if (stage !== 'sealed') return
    const { x, y } = rel(e)
    const t = press(x, y)
    if (!t) return
    ;(e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId)
    tear = t
  }
  function move(e: PointerEvent) {
    if (!tear.dragging || stage !== 'sealed') return
    const r = stretch(tear, rel(e).x, ripAt(reduced))
    tear = r.tear
    if (r.rip) open()
  }
  function up() {
    if (stage === 'sealed') tear = NO_TEAR
  }
  // a click with no pointer is Enter or Space on the pack; with reduced motion any click opens it (#538)
  function clickOpen(e: MouseEvent) {
    if (e.detail === 0 || reduced) open()
  }

  // the zigzag the perforation and the tear follow: the seam's teeth (y 0↔8), 3px thick
  const XS = [0.8, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75, 80, 85, 90, 95, 99.2]
  const ZIGZAG = `polygon(${[...XS.map((x, i) => `${x}% ${i % 2 ? 0 : 8}px`), ...XS.map((x, i) => `${x}% ${(i % 2 ? 0 : 8) + 3}px`).reverse()].join(', ')})`
  const SEAM_TEETH = `polygon(0 0, 100% 0, ${XS.map((x, i) => `${x}% ${i % 2 ? 50 : 58}px`).reverse().join(', ')})`
  const SEAM_FLAT = 'polygon(0 0, 100% 0, 99.2% 58px, 0.8% 58px)'
  // with reduced motion the strip stays put while you drag (#538)
  const lift = $derived(stage === 'sealed' && !reduced ? progress : 0)

  let joining = $state(false)
  let error = $state('')
  async function act() {
    if (!view?.button?.action || !preview) return
    if (view.button.action === 'open') {
      ui.goServer = preview.server_id
      return
    }
    if (joining) return
    joining = true
    error = ''
    try {
      await acceptInvite(code)
    } catch (e) {
      error = errorText(e)
    } finally {
      joining = false
    }
  }
</script>

<section bind:this={root} class="zone" class:sealed={stage === 'sealed'} class:ripping={stage === 'ripping'} class:open={stage === 'open'} class:opening class:dead={view?.dead}
  aria-label={preview ? `Invite to ${preview.name}` : 'Server invite'} data-server-invite={code} data-state={shown ?? 'loading'} data-stage={stage ?? 'loading'}
  style:--foil-a={colors[0]} style:--foil-b={colors[1]} style:--gold={PACK.gold} style:--gold-glow={PACK.goldGlow}
  style:--gold-halo={PACK.goldHalo} style:--gold-burst={PACK.goldBurst} style:--rose={PACK.rose} style:--periwinkle={PACK.periwinkle}
  style:--mint={PACK.mint} style:--on-gold={PACK.onGold} style:--print={PACK.print} style:--print-soft={PACK.printSoft} style:--print-faint={PACK.printFaint}
  style:--light={PACK.light} style:--light-soft={PACK.lightSoft} style:--light-clear={PACK.lightClear} style:--sheen={PACK.sheen} style:--shine={PACK.shine}
  style:--crimp={PACK.crimp} style:--crimp-shade={PACK.crimpShade} style:--crimp-light={PACK.crimpLight} style:--crimp-dark={PACK.crimpDark} style:--perf={PACK.perf}
  style:--edge-dark={PACK.edgeDark} style:--edge-darker={PACK.edgeDarker} style:--window-hi={PACK.windowHi} style:--window-lo={PACK.windowLo}
  style:--window-rim={PACK.windowRim} style:--window-inset={PACK.windowInset} style:--window-lip={PACK.windowLip} style:--icon-shadow={PACK.iconShadow}
  style:--print-shadow={PACK.printShadow} style:--print-glow={PACK.printGlow} style:--drop-far={PACK.dropFar} style:--drop-near={PACK.dropNear}
  style:--card-drop={PACK.cardDrop} style:--tear-glow={PACK.tearGlow} style:--spark={PACK.spark}>
  {#if !view}
    <p class="loading">Loading invite…</p>
  {:else}
    {#if opening}<div class="burst" aria-hidden="true"></div>{/if}

    <!-- the card: risen out of the pack, or simply there -->
    <div class="card" aria-hidden={stage === 'sealed' ? 'true' : undefined}>
      <div class="holo">
        <div class="face">
          {#if preview && !view.dead}<div class="rarity">✦ MYTHICAL PULL ✦</div>{/if}
          <div class="banner"><ServerBanner id={preview?.server_id ?? code} hash={preview?.banner_hash} still={preview?.banner_still} />
            <span class="ring">
              {#if preview}<ServerChip id={preview.server_id} name={preview.name} iconHash={preview.icon_hash} size={94} radius="27px" />{:else}<span class="unknown">?</span>{/if}
            </span>
          </div>
          <div class="text">
            {#if preview}
              <strong class="name">{preview.name}</strong>
              <span class="meta">from {from} · {countsLine(preview.online_count, preview.member_count)}</span>
            {/if}
            {#if view.note}<span class="note" data-note>{view.note}</span>{/if}
          </div>
          {#if view.button}
            <button type="button" class="join" class:open-btn={view.button.action === 'open'} disabled={!view.button.action || joining || stage === 'sealed'}
              tabindex={stage === 'sealed' ? -1 : undefined} onclick={() => void act()}>{view.button.label}</button>
          {/if}
          {#if error}<p class="err" role="alert">{error}</p>{/if}
          <span class="sweep" aria-hidden="true"></span>
        </div>
      </div>
    </div>

    {#if preview && (stage === 'sealed' || stage === 'ripping')}
      <div class="float">
        <button type="button" class="pack" class:dragging={tear.dragging} aria-label="Server pack from {from}: {preview.name}. {reduced ? 'Click or press Enter' : 'Drag across the top, or press Enter,'} to open it."
          onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} onclick={clickOpen}>
          <span class="body">
            <span class="foil" style:background-position="0 -50px"></span>
            <span class="edges"></span>
            <span class="crimp bottom"></span>
            <span class="sweep slow"></span>
            <span class="print">
              <span class="brand">PULSE</span>
              <span class="kind">SERVER PACK</span>
              <span class="window"><ServerChip id={preview.server_id} name={preview.name} iconHash={preview.icon_hash} size={76} radius="22px" /></span>
              <span class="pname">{preview.name}</span>
              <span class="inside">1 invite inside · from {from}</span>
            </span>
          </span>
          <span class="strip" style:clip-path={stage !== 'sealed' || progress > 0.02 ? SEAM_TEETH : SEAM_FLAT}
            style:transform={stage !== 'sealed' ? undefined : `rotate(${(-lift * 5).toFixed(2)}deg) translateY(${(-lift * 4).toFixed(1)}px)`}>
            <span class="foil"></span>
            <span class="edges"></span>
            <span class="crimp top"></span>
            <span class="tear-label">{progress > 0.02 ? 'KEEP GOING' : 'TEAR HERE'}</span>
          </span>
          <span class="zig perf" style:clip-path={ZIGZAG}></span>
          <span class="zig zip" class:idle={!tear.dragging && progress < 0.02} style:clip-path={ZIGZAG}></span>
          <span class="tear" class:on={progress > 0.01 && stage === 'sealed'}
            style:clip-path="inset(0 {((1 - tear.lo - (stage === 'sealed' ? progress : 0)) * 100).toFixed(1)}% 0 {(tear.lo * 100).toFixed(1)}%)">
            <span class="tear-line" style:clip-path={ZIGZAG}></span>
          </span>
          <span class="spark" class:on={tear.dragging && stage === 'sealed'} style:left="{(tear.cursor * 100).toFixed(1)}%"></span>
        </button>
        <span class="hint">{reduced ? 'Click to open it' : 'Drag across the top to tear it open'}</span>
      </div>
    {/if}
  {/if}
</section>

<style>
  /* the space a pack (and the card it holds) takes, reserved whatever it shows: nothing shifts as it loads or opens */
  .zone { --ease: cubic-bezier(.2, .8, .2, 1); position: relative; width: min(100%, 300px); height: 440px; margin-top: 4px; }
  .loading { margin: 0; padding-top: 180px; text-align: center; font-size: 13px; color: var(--text-3); }

  /* --- the card --- */
  /* Laid out at the size it's seen at once open (#517), so it rests untransformed: no upscaled bitmap, no compositing
     layer left on after the rise (the text and icon stay crisp). In the pack and as it rips it's scaled down instead. */
  .card { position: absolute; left: calc(50% - 123px); top: 6px; width: 246px; height: 370px; border-radius: 20px; z-index: 5;
    transform: none; box-shadow: 0 20px 50px var(--card-drop), 0 0 40px var(--gold-halo); }
  .sealed .card { z-index: 1; opacity: 0; transform: translateY(54px) scale(.805); }
  .opening .card { transition: transform 700ms var(--ease), opacity 200ms ease; }
  .ripping .card { z-index: 1; opacity: 1; transform: translateY(-76px) scale(.805); }
  .holo { position: absolute; inset: 0; border-radius: 20px; padding: 3px; background: conic-gradient(from 200deg, var(--gold), var(--rose), var(--periwinkle), var(--mint), var(--gold)); }
  .face { position: relative; width: 100%; height: 100%; box-sizing: border-box; border-radius: 17px; background: var(--bg-1); padding: 14px; display: flex; flex-direction: column; gap: 11px; overflow: hidden; }
  /* a deeper gold per theme, heavier, with a quiet glow: it reads at 4.5:1 on the card (#517) */
  .rarity { font-family: var(--font-mono, monospace); font-size: 12px; font-weight: 800; letter-spacing: .18em; text-align: center; color: var(--pack-gold-text); animation: pack-glowpulse 2s infinite; }
  .banner { position: relative; height: 126px; flex-shrink: 0; border-radius: var(--radius-md); overflow: hidden; }
  /* the icon in the banner's middle, ringed in the card's surface and glowing gold: on top of the banner, not in it */
  .ring { position: absolute; inset: 0; margin: auto; width: fit-content; height: fit-content; display: grid; border-radius: 27px; box-shadow: 0 0 0 4px var(--bg-1), 0 0 24px var(--gold-glow); }
  .unknown { width: 94px; height: 94px; border-radius: 27px; background: var(--bg-3); color: var(--text-3); display: grid; place-items: center; font-size: 34px; font-weight: 800; }
  .text { text-align: center; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: 20px; font-weight: 800; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: 13px; color: var(--text-2); overflow-wrap: anywhere; }
  .note { font-size: 13px; font-weight: 600; color: var(--text-2); }
  .join { margin-top: auto; font: inherit; font-size: 16px; font-weight: 800; padding: 12px; border-radius: var(--radius-md); border: 0; background: var(--gold); color: var(--on-gold); cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .join:disabled { opacity: .6; cursor: default; }
  .join.open-btn { background: var(--bg-3); color: var(--text); }
  .err { margin: 0; color: var(--danger-text); font-size: 13px; text-align: center; }
  .sweep { position: absolute; top: 0; bottom: 0; left: 0; width: 40%; background: linear-gradient(90deg, transparent, var(--shine), transparent); animation: pack-sweep 2.6s ease-in-out infinite 1s; pointer-events: none; }
  /* a dead invite: the card greyed, with why; no pull, no shine */
  .dead .card { box-shadow: none; }
  .dead .holo { background: var(--line); }
  .dead .banner, .dead .ring { filter: grayscale(1); opacity: .5; }
  .dead .sweep { display: none; }
  .dead .join { background: var(--bg-3); color: var(--text-3); opacity: 1; }

  /* --- the sealed pack --- */
  .float { position: absolute; left: calc(50% - 130px); top: 10px; z-index: 2; display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .sealed .float { animation: pack-bob 3s ease-in-out infinite; }
  .ripping .float { transform: translateY(150px); opacity: 0; transition: transform 600ms var(--ease) 150ms, opacity 450ms ease 300ms; }
  .pack { position: relative; display: block; width: 260px; height: 400px; padding: 0; border: 0; background: transparent; cursor: grab; font: inherit; color: inherit; touch-action: none;
    filter: drop-shadow(0 22px 26px var(--drop-far)) drop-shadow(0 2px 2px var(--drop-near)); }
  .pack.dragging { cursor: grabbing; }
  .sealed .float:has(.pack.dragging) { animation: none; }
  .pack:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 4px; border-radius: var(--radius-lg); }
  /* the body: crimped at the bottom, pinched at the waist so the crimps flare; the seam's teeth on top */
  .body { position: absolute; left: 0; right: 0; top: 50px; height: 350px; z-index: 2;
    clip-path: polygon(0.8% 8px, 5% 0, 10% 8px, 15% 0, 20% 8px, 25% 0, 30% 8px, 35% 0, 40% 8px, 45% 0, 50% 8px, 55% 0, 60% 8px, 65% 0, 70% 8px, 75% 0, 80% 8px, 85% 0, 90% 8px, 95% 0, 99.2% 8px, 98.2% 30%, 98% 55%, 98.6% 92%, 100% 100%, 0 100%, 1.4% 92%, 2% 55%, 1.8% 30%); }
  /* one gradient across the strip and the body (each shows its part of the same 260×400), so it reads as one piece */
  .foil { position: absolute; inset: 0; background: linear-gradient(115deg, var(--foil-a) 0%, var(--foil-b) 26%, var(--foil-a) 44%, var(--foil-b) 66%, var(--foil-a) 100%); background-size: 260px 400px; background-position: 0 0; }
  .edges { position: absolute; inset: 0; background: linear-gradient(90deg, var(--edge-dark) 0%, var(--light) 14%, var(--light-clear) 34%, var(--light-clear) 64%, var(--light-soft) 84%, var(--edge-darker) 100%); }
  .crimp { position: absolute; left: 0; right: 0; height: 20px; }
  .crimp.bottom { bottom: 0; background: repeating-linear-gradient(90deg, var(--crimp) 0 2px, var(--crimp-shade) 2px 3px, transparent 3px 6px), linear-gradient(180deg, var(--crimp-dark), var(--crimp-light)); }
  .crimp.top { top: 0; background: repeating-linear-gradient(90deg, var(--crimp) 0 2px, var(--crimp-shade) 2px 3px, transparent 3px 6px), linear-gradient(0deg, var(--crimp-dark), var(--crimp-light)); }
  .sweep.slow { width: 30%; background: linear-gradient(90deg, transparent, var(--sheen), transparent); animation: pack-sweep 3.6s ease-in-out infinite; }
  .print { position: absolute; left: 0; right: 0; top: 34px; display: flex; flex-direction: column; align-items: center; gap: 14px; min-width: 0; }
  .brand { font-size: 30px; font-weight: 900; font-style: italic; letter-spacing: .04em; color: var(--print); text-shadow: 0 3px 0 var(--print-shadow), 0 0 18px var(--print-glow); }
  .kind { font-family: var(--font-mono, monospace); font-size: 10px; letter-spacing: .3em; color: var(--print-soft); }
  .window { width: 112px; height: 112px; border-radius: 50%; display: grid; place-items: center; background: radial-gradient(circle at 40% 35%, var(--window-hi), var(--window-lo) 70%);
    border: 3px solid var(--window-rim); box-shadow: inset 0 4px 10px var(--window-inset), 0 2px 0 var(--window-lip); }
  .window :global(.chip) { box-shadow: 0 6px 14px var(--icon-shadow); }
  .pname { max-width: 220px; font-size: 16px; font-weight: 800; color: var(--print); text-shadow: 0 1px 2px var(--print-shadow); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .inside { font-size: 11px; color: var(--print-soft); }
  /* the strip above the perforation: it lifts as you tear, springs back if you let go, and flies off on the rip */
  .strip { position: absolute; left: 0; right: 0; top: 0; height: 58px; z-index: 4; overflow: hidden; transform-origin: 100% 100%; transition: transform 350ms var(--ease); }
  .pack.dragging .strip { transition: transform 60ms linear; }
  .ripping .strip { transform: translate(70px, -170px) rotate(32deg); opacity: 0; transition: transform 650ms var(--ease), opacity 450ms ease 250ms; }
  .tear-label { position: absolute; left: 0; right: 0; top: 30px; text-align: center; font-size: 9px; font-weight: 800; letter-spacing: .34em; color: var(--print-faint); }
  /* the dashed zigzag perforation, and a light that zips along it now and then while it waits */
  .zig { position: absolute; left: 0; top: 50px; width: 260px; height: 12px; pointer-events: none; z-index: 5; }
  .perf { background: repeating-linear-gradient(90deg, var(--perf) 0 5px, transparent 5px 9px); transition: opacity 200ms ease; }
  .ripping .perf { opacity: 0; }
  .zip { background: linear-gradient(90deg, transparent, var(--print), transparent) no-repeat; background-size: 30% 100%; background-position: -40% 0; }
  .sealed .zip.idle { animation: pack-zipbg 3.2s ease-in-out infinite .6s; }
  /* the glowing tear following the pointer, and the spark at its tip */
  .tear { position: absolute; left: 0; top: 46px; width: 260px; height: 20px; z-index: 6; pointer-events: none; opacity: 0;
    filter: drop-shadow(0 0 3px var(--print)) drop-shadow(0 0 6px var(--tear-glow)); transition: clip-path 300ms var(--ease), opacity 300ms ease; }
  .tear.on { opacity: 1; }
  .pack.dragging .tear { transition: none; }
  .tear-line { position: absolute; left: 0; top: 4px; width: 260px; height: 12px; background: var(--print); }
  .spark { position: absolute; top: 50px; width: 12px; height: 12px; margin-left: -6px; border-radius: 50%; z-index: 6; pointer-events: none; opacity: 0;
    background: radial-gradient(circle, var(--print), var(--light-clear) 70%); box-shadow: 0 0 12px 4px var(--spark); transition: opacity 150ms ease; }
  .spark.on { opacity: 1; }
  .hint { font-size: 13px; color: var(--text-2); animation: pack-hintfade 2s ease-in-out infinite; }
  .ripping .hint { opacity: 0; transition: opacity 200ms ease; }
  /* the gold burst as the card comes out */
  .burst { position: absolute; left: 50%; top: 205px; width: 0; height: 0; border-radius: 50%; z-index: 0; outline: 150px solid var(--gold-burst); outline-offset: -150px; animation: pack-burst 900ms ease-out forwards; pointer-events: none; }

  @keyframes pack-sweep { 0% { transform: translateX(-120%) skewX(-20deg); } 100% { transform: translateX(320%) skewX(-20deg); } }
  @keyframes pack-zipbg { 0% { background-position: -40% 0; } 60%, 100% { background-position: 140% 0; } }
  @keyframes pack-glowpulse { 0%, 100% { text-shadow: 0 0 4px var(--gold-halo); } 50% { text-shadow: 0 0 10px var(--gold-glow); } }
  @keyframes pack-burst { 0% { transform: scale(.2); opacity: 0; } 30% { opacity: 1; } 100% { transform: scale(1.7); opacity: 0; } }
  @keyframes pack-bob { 0%, 100% { transform: translateY(0) rotate(0deg); } 50% { transform: translateY(-4px) rotate(-.6deg); } }
  @keyframes pack-hintfade { 0%, 100% { opacity: .55; } 50% { opacity: 1; } }
  /* a paused window or a stream: it holds still */
  :global(.streaming) .zone *, :global(.unfocused) .zone * { animation-play-state: paused; }
  /* less motion (#538): the pack is held still (no bob, zip or sweep) and a click opens it, so it reads as one */
  @media (prefers-reduced-motion: reduce) { .zone *, .zone { animation: none !important; transition: none !important; } .pack, .pack.dragging { cursor: pointer; } }
</style>
