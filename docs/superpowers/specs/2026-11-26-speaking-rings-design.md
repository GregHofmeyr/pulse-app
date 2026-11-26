# Speaking rings: design

Date: 2026-11-26. Issue #651. The owner's decisions came from a design conversation and a canvas of mockups: the nine
looping styles on the voice stage, each style at three sizes, the Profile settings picker, and four voice-reactive
ideas. Two of those ideas were taken up and one was dropped. This doc describes every style exactly, so the builder
never needs the canvas. Where it says "the canvas", it only means where a decision came from.

## 1. Intent

Today everyone's speaking ring looks the same: a solid green double ring around the avatar (`box-shadow` in
`UserTile.svelte`, `CallBand.svelte` and `ChannelList.svelte`). Issue #651 asks to let people pick their own ring.
It's small and mostly cosmetic, and it adds personality. It must still be obvious who is speaking, and it must
respect reduced motion.

This design adds a **ring style** (11 styles, three of which follow your voice) and a **ring colour** (a curated
palette, or your profile colour) to your profile. Everyone in a call sees them. One viewer setting turns everyone's
ring back into the plain one. Performance is the owner's main worry, so §8 sets hard rules: animations are
compositor-only, rings run only while that person speaks, and there's a cap on how many animate at once.

### Goals
- You pick a style and a colour once, in Profile settings, with a live preview. Everyone sees your ring in voice
  channels, DM calls and group calls.
- Speaking stays obvious in every style and colour, on every theme.
- The voice-reactive styles follow how loud you actually are (what the listener hears), not a fake loop.
- No style costs a repaint per frame. Silent tiles cost nothing.
- Nothing changes for anyone until they opt in. The default is Classic green, identical to today.

### Non-goals (this design)
- A free colour picker or custom gradients. The palette is curated (owner decision).
- A true waveform or spectrum. It would need a per-frame repaint (owner decision: dropped).
- The old "bars under the avatar" equaliser (owner decision: dropped in favour of Bars round the ring).
- Rings anywhere outside calls: profile cards, member lists, message avatars.
- Animated rings on the 22px channel-list avatars. They always show the static ring.
- Per-frequency (spectrum) data from the core. Only one loudness number per person goes to the UI.

### Anti-goals (these would make it a failure even if it works)
- A call that stutters, or a fan that spins up, because several people with fancy rings talk at once.
- A ring that spills out of its tile, overlaps the name, or gets clipped by accident.
- A colour that vanishes on some theme (for example white on the light theme), so you can't tell who's talking.
- Level events flowing when nobody is speaking or nobody can see a ring.

## 2. Decisions made with the owner

| Topic | Decision |
|---|---|
| Looping styles | Classic (default), Soft glow, Pulse, Ripple, Sweep, Orbit (5 dots), Halo, Viewfinder |
| Voice-reactive styles | Liquid (3 soft blobs behind the avatar), Wave of dots (24 dots), Bars round the ring (~44 bars) |
| Dropped | Bars under the avatar; a true waveform (per-frame repaint) |
| Colour | About 8 curated colours that read on every theme, plus "use my profile colour". No free picker |
| Default | Classic green: nothing changes until someone opts in |
| Viewer setting | "Show everyone's rings as Classic" in Voice settings, off by default (slower PCs, motion sensitivity) |
| Where | Voice stage tiles, the call band, DM and group calls. The 22px channel-list avatars always show the static ring |
| Motion rules | Only `transform` and `opacity` animate, each on its own small layer. Never box-shadow, blur, width or colour per frame |
| Blur | Static: drawn once |
| When | Animation runs only while that person is speaking |
| Reduced motion | Every style shows its still version |
| Cap | With more than about 8 people speaking at once, the extras show their still version |
| Levels | Per-speaker loudness 12–15 times a second, only for people speaking, as one small event (user id → 0..1). Nothing at all when nobody speaks |
| Levels in the UI | A CSS custom property (`--lvl`) on each tile, read through `scale`, smoothed by a short transition. Only visible tiles subscribe |
| Storage | Profile fields on the server (migration, `PUT /me/profile`, validated keys), carried on `User` like other profile fields. Old clients ignore them |
| Settings | A "Speaking ring" section in Profile settings: a style grid, colour swatches and a live animated preview |
| Containers | Nothing spills out of its container. Rings reserve their space, or the tile clips them on purpose |

## 3. The ring, wherever it shows

### 3.1 Places and sizes

| Place | Component | Avatar (D) | Ring |
|---|---|---|---|
| Voice stage tile (voice channels, DM and group calls) | `UserTile.svelte` (stage) | `tileScale(h).avatar`: 48–160px | Full style |
| Full view's strip | `UserTile.svelte` (`compact`) | 36px | Full style |
| Call band (top of a DM or group) | `CallBand.svelte` | 34px | Full style |
| Channel list occupants | `ChannelList.svelte` | 22px | Static Classic ring, in the person's colour |
| Profile settings preview | new, §7 | 96px | Your draft style, animated |

### 3.2 One component draws every ring

Add a new `SpeakingRing.svelte` (doc's call). It sits inside the avatar button, as an absolutely positioned sibling
of `Avatar`, centred on the avatar, with `pointer-events: none`. It takes:
- `userId`
- `d`, the avatar diameter in px
- `speaking`
- `place`: `'stage' | 'strip' | 'band' | 'preview'`
- for the preview only, an optional `style`/`colour` override

It sets `--d: {d}px` and `--speak: <colour>` on its root. Every size below is a fraction of `--d`, so one stylesheet
works at every size.

`SpeakingRing` draws **Classic too**. Classic becomes a static layer the size of the avatar, carrying today's exact
`box-shadow`. These per-place rules get removed:
- `.tile.speaking :global(.av)` in `UserTile.svelte`
- `.compact.speaking :global(.av)` in `UserTile.svelte`
- `.who.speaking :global(.av)` in `CallBand.svelte`

`ChannelList.svelte`'s `.ring.speaking` stays its own static rule, with `var(--ok)` replaced by the person's colour.

### 3.3 The ring box: nothing spills

Every style stays inside a **ring box**: a square of **1.32 D** centred on the avatar, so 0.16 D of outset on each
side, at every frame of its animation, including the peak of a pulse or a level. The specs in §4 are sized to fit
it. Reserve the space like this:

- **Stage and strip tiles.**
  - The avatar button gets `margin: calc(var(--d) * .16)`. Use margin, not padding: the button's own box, its focus
    ring and the mute badge stay anchored to the avatar disc.
  - `tileScale` (`lib/stagelayout.ts`) caps the avatar so that `1.32 D + 10px gap + the name row` fits inside the
    tile's height less 8px. Keep the existing 48–160 clamp, then take the smaller of the two.
  - `.tile` gets `overflow: hidden`, so a tile too small even for that clips on purpose at its rounded border
    rather than spilling into its neighbour.
  - This reserves space for everyone, Classic included, so switching style never moves the layout. Avatars on the
    tightest tiles shrink by up to about 10% (doc's call).
- **Call band.** `.people` gap goes from 6px to 12px, which is at least 2 × 0.16 × 34. The band's 10px vertical
  padding already covers 5.4px.
- **Strip.** The strip slot must leave at least 6px of clear space around the 36px avatar. The tile's
  `overflow: hidden` covers the rest.
- **Preview.** The 200 × 200 box holds a 96px avatar (its ring box is 127px).

Layering inside the ring box, back to front:
1. Liquid and Halo layers, which sit behind the avatar
2. the avatar
3. ring layers (dots, bars, arcs)
4. the mute or deafen badge

The badge stays above the ring.

### 3.4 Colour

`--speak` resolves in this order (doc's call):
1. The palette key's per-theme token, `var(--speak-<key>)`.
2. For `profile`, the person's name colour: `User.style.colors[0]`, already lightened for names by the server. With
   no name colour set, it falls back to green.
3. Unknown, absent or blocked (§6.4): green.

**The palette** (8 colours). Each is a token declared in **every theme block** in `app.css`. Every theme block
already declares every colour token, and `colorTokens.test.ts` and `themes.test.ts` check that.

| Key | Label | Dark themes | Light theme |
|---|---|---|---|
| `green` (default) | Green | `var(--ok)` (`#4fd18b` on dark: exactly today's ring) | `var(--ok)` |
| `blue` | Pulse blue | `#8b9cff` | a deeper shade, ≥ 3:1 on `--bg-1` |
| `gold` | Gold | `#e8b04a` | a deeper shade, ≥ 3:1 |
| `pink` | Pink | `#f06fb0` | a deeper shade, ≥ 3:1 |
| `cyan` | Cyan | `#4fc8e8` | a deeper shade, ≥ 3:1 |
| `orange` | Orange | `#f0884f` | a deeper shade, ≥ 3:1 |
| `white` | White | `#e8e9ed` | the theme's text ink (`var(--text)`): white can't show on a light panel |
| `purple` | Purple | `#b07cf0` | a deeper shade, ≥ 3:1 |

The doc's calls on the palette:
- **Purple is the 8th colour.** It's distinct from Pulse blue.
- **There's no red.** Red is the mute badge's `--danger`, and a red ring would read as "muted".
- The builder picks the exact light-theme shades, and a test checks each one is ≥ 3:1 (WCAG non-text contrast)
  against that theme's `--bg-1`. The same test checks the dark shades against every dark theme's `--bg-1`.

The tile's speaking **border** (`.tile.speaking { border-color }`) also takes `var(--speak)`. It's static, so it
costs nothing.

### 3.5 When a ring moves, and when it's still

`SpeakingRing` decides one of three states per tile:
- `off`: not speaking. Every layer is at `opacity: 0` with `animation: none`. Silent tiles cost nothing.
- `on`: speaking, animated.
- `still`: speaking, shown as the style's still version (§4).

The ring is lit (`on` or `still`) when the person is in `voice.speaking` or in the soundboard's playing set (today's
rule, unioned at render time). It's `still` instead of `on` when any of these holds:
1. reduced motion (`motion.reducedMotion` in `lib/animate.svelte.ts`)
2. you are live, i.e. `motion.streaming`, the rule emoji already follow (doc's call: it keeps the GPU for the
   encoder)
3. the person isn't among the **first 8 people to start speaking** of those speaking now (§8.3)
4. `place` is the channel list, which is always still

"Show everyone's rings as Classic" (§7.2) forces the style to `classic` before any of this. Classic has no animation,
so its `on` and `still` look the same.

The layers for the chosen style are rendered for as long as the tile is mounted, so starting to speak doesn't create
any DOM. Only a class or data attribute changes (`data-ring="off|on|still"`). Don't set `will-change` permanently. The
browser promotes a layer for as long as it animates, and a permanent `will-change` would hold layers for silent
tiles.

## 4. Each style

Notation:
- `D` is the avatar diameter. `calc(var(--d) * k)` is written as `k·D`.
- A layer is `position: absolute; border-radius: 50%; pointer-events: none`, positioned with `inset` relative to the
  avatar box. A negative inset reaches outside the avatar.
- "Lit" means `on` or `still`.
- Every `@keyframes` in this section changes only `transform` and `opacity`.
- Levels reach a style only through the `scale` property (§5.4). `scale` multiplies with any `transform` keyframe,
  and both use the same `transform-origin`.

### 4.1 Classic (default)
- **Look.** Today's solid double ring, snapping on and off.
- **CSS.**
  - One layer at `inset: 0`.
  - Lit, it has `box-shadow: 0 0 0 Apx var(--ring), 0 0 0 Bpx var(--speak)`, where A/B is 4/7 on the stage and
    2/4 on the strip, band and channel list. Those are exactly today's values.
  - No transition, because a box-shadow transition repaints every frame. That's the same reason `ChannelList`
    snaps today.
- **Still.** Identical.

### 4.2 Soft glow
- **Look.** A gentle glow fades in around the avatar and stays while you talk.
- **CSS.**
  - One layer at `inset: -.104·D` with
    `background: radial-gradient(circle closest-side, transparent 73%, var(--speak) 82%, transparent 100%)`.
  - `opacity: 0` normally, `.9` when lit, with `transition: opacity .15s`.
  - The gradient is painted once. Only opacity changes, once per start and stop.
- **Still.** Opacity `.9`, no transition.

### 4.3 Pulse
- **Look.** One ring breathes in and out while you speak.
- **CSS.**
  - One layer at `inset: -.073·D`, `border: max(2px, .031·D) solid var(--speak)`.
  - On: `animation: ring-pulse 1.1s ease-in-out infinite`.
  - `@keyframes ring-pulse { 0%,100% { transform: scale(1); opacity: .95 } 50% { transform: scale(1.07); opacity: .55 } }`.
  - Largest extent: 0.573 × 1.07 = 0.613·D.
- **Still.** `scale(1)`, opacity `.95`.

### 4.4 Ripple
- **Look.** Sonar rings travel outward from the avatar.
- **CSS.**
  - Two layers at `inset: -.042·D`, `border: max(1.5px, .021·D) solid var(--speak)`.
  - On: `animation: ring-ripple 1.6s ease-out infinite`. The second layer has `animation-delay: -.8s`. The delay is
    negative so both rings are already travelling on the first frame.
  - `@keyframes ring-ripple { 0% { transform: scale(1); opacity: .9 } 100% { transform: scale(1.18); opacity: 0 } }`.
  - The canvas used 1.4. That's lowered to 1.18 (doc's call) so the ring stays in the box: 0.542 × 1.18 = 0.64·D.
- **Still.** The first layer at `scale(1)`, opacity `.9`. The second is hidden.

### 4.5 Sweep
- **Look.** A bright arc runs round the ring.
- **CSS.**
  - One layer at `inset: -.0625·D`.
  - `background: conic-gradient(from 0deg, transparent 0 55%, var(--speak) 85%, transparent 100%)`.
  - `mask: radial-gradient(circle closest-side, transparent 92%, #000 93%)`, which leaves a thin band just outside
    the avatar.
  - On: `animation: ring-spin 1.8s linear infinite`, with `@keyframes ring-spin { to { transform: rotate(360deg) } }`.
  - The canvas punched the centre with a solid "gap" disc in the tile's colour. That breaks over a tile's game
    backdrop art (#380). The mask is static and radially symmetric, so it rotates with the layer for free
    (doc's call).
- **Still.** No rotation. The background swaps to solid `var(--speak)` at opacity `.9`, so it reads as a thin full
  ring. The swap happens once, on the state change.

### 4.6 Orbit
- **Look.** Five little dots circle the avatar, evenly spaced.
- **CSS.**
  - One rotating layer at `inset: -.125·D`, holding 5 static holders (`position: absolute; inset: 0`) rotated
    `0deg, 72deg, 144deg, 216deg, 288deg`.
  - Each holder has one dot at its top centre: `top: 0; left: 50%; width: max(4px, .094·D); aspect-ratio: 1;
    translate: -50% 0; border-radius: 50%; background: var(--speak)`.
  - On: the layer runs `ring-spin 2.6s linear infinite`. One animated element in all.
  - Largest extent: 0.625·D.
- **Still.** No rotation. The five dots sit at their rest positions.

### 4.7 Halo
- **Look.** A soft aura swells and settles behind the avatar.
- **CSS.**
  - One layer behind the avatar at `inset: -.15·D`, with
    `background: radial-gradient(circle closest-side, var(--speak) 70%, transparent 100%)`.
  - The soft edge is the gradient. There's no `filter: blur` (doc's call, §9). That keeps the blur drawn once and
    the layer inside its box.
  - On: `animation: ring-halo 1.4s ease-in-out infinite`.
  - `@keyframes ring-halo { 0%,100% { transform: scale(.92); opacity: .35 } 50% { transform: scale(1); opacity: .7 } }`.
  - Largest extent: 0.65·D.
- **Still.** `scale(.96)`, opacity `.5`.

### 4.8 Viewfinder
- **Look.** Four corner brackets snap in around the avatar.
- **CSS.**
  - One square layer at `inset: -.115·D; border-radius: 0`, with 4 corner elements.
  - Each corner is `.167·D` square, with `border: 0 solid var(--speak)` and `max(2px, .031·D)` width on its two
    outer sides only: top+left, top+right, bottom+left, bottom+right.
  - On: one entry animation, `ring-snap .18s ease-out` (runs once each time they start speaking), then static.
  - `@keyframes ring-snap { from { transform: scale(1.06); opacity: 0 } to { transform: scale(1); opacity: 1 } }`.
  - The canvas started at 1.15. That's lowered to 1.06 (doc's call): 0.615 × 1.06 = 0.652·D stays inside the
    square box.
- **Still.** The corners show without the entry animation.

### 4.9 Liquid (voice-reactive)
- **Look.** Three soft blobs behind the avatar swell and drift with your loudness: a liquid edge without a waveform.
- **CSS.**
  - A wrapper layer behind the avatar at `inset: -.05·D` (1.1·D across).
  - Lit: opacity `.85`.
  - `scale: calc(.88 + .12 * var(--lvl, .5))`, with `transition: scale .1s linear`.
  - It holds three blob layers (`inset: 0`), each
    `background: radial-gradient(circle closest-side, var(--speak) 55%, transparent 100%)`. No `filter` (doc's
    call: the canvas blurred the wrapper, and a filtered parent with moving children is re-blurred every frame).
  - On, each blob drifts:
    - blob 1: `ring-b1 1.3s ease-in-out infinite`, with `50% { transform: translate(.0625·D, -.052·D) scale(1.04) }`
    - blob 2: `ring-b2 1.7s`, with `50% { transform: translate(-.0625·D, .042·D) scale(1.05) }`
    - blob 3: `ring-b3 1.1s`, with `50% { transform: translate(.021·D, .073·D) scale(1.03) }`
  - Every blob is at `transform: none` at 0% and 100%.
  - Largest extent: about 0.65·D.
- **Still.** Blobs at rest, wrapper at `scale(1)` and opacity `.85`. The level is ignored.

### 4.10 Wave of dots (voice-reactive)
- **Look.** 24 dots round the ring swell in a wave that travels around. It runs taller and faster when you're loud.
- **CSS.**
  - A layer the size of the ring box (`inset: -.16·D`), holding 24 static holders (`inset: 0`) rotated
    `i × 15deg`.
  - Each holder has one dot: `top: .056·D; left: 50%; width: max(3px, .052·D); aspect-ratio: 1; translate: -50% 0;
    border-radius: 50%`. Dot centres sit at radius 0.578·D.
  - On: each dot runs `ring-dot 1s ease-in-out infinite`, with `animation-delay: -(i/24)s`. The negative delay puts
    the wave in motion at once, travelling round.
  - `@keyframes ring-dot { 0%,100% { transform: scale(.6) } 50% { transform: scale(1) } }`.
  - **Taller with loudness.** Each dot has `scale: calc(.8 + .6 * var(--lvl, .5))`, with
    `transition: scale .1s linear`. Peak 1.4 gives a largest extent of 0.614·D.
  - **Faster with loudness.** Changing `animation-duration` mid-run jumps the phase, so it isn't used. Instead
    there are three speed buckets from the level: < .35 runs at 0.75×, < .7 at 1×, and above that at 1.5×. A
    bucket has to hold for 250ms before it switches. On a switch, the tile's ring code calls
    `anim.updatePlaybackRate(rate)` on each `ring.getAnimations({ subtree: true })`. `updatePlaybackRate` changes
    speed without a jump. It runs a few times a second at most (doc's call).
  - With D < 48, render 12 dots (every 30°) instead of 24, because 24 don't fit at band size.
- **Still.** All dots at `scale(1)`, no animation. The level is ignored.

### 4.11 Bars round the ring (voice-reactive)
- **Look.** About 44 thin bars packed round the ring, neighbours moving together like a radial equaliser, rising
  with loudness.
- **CSS.**
  - A layer the size of the ring box (`inset: -.16·D`), holding **44 static holders** (`inset: 0`) rotated
    `i × (360/44)deg`.
  - Each holder has **one bar**: `top: 0; left: 50%; width: max(1px, .031·D); height: .13·D; translate: -50% 0;
    border-radius: 999px; background: var(--speak)`.
  - The bar's outer end is at the ring box edge (0.66·D) and its inner end at 0.53·D, just outside the avatar.
    `transform-origin: 50% 100%`, so it **scales from its inner end**.
  - On: each bar runs `ring-bar .5s ease-in-out infinite alternate`, with
    `animation-delay: -((sin(i × .9) + 1) × .25)s`. Neighbours get near phases, which is what makes it read as an
    equaliser. Compute the delay at render time.
  - `@keyframes ring-bar { from { transform: scaleY(.35) } to { transform: scaleY(1) } }`.
  - **Rising with loudness.** Each bar has `scale: 1 calc(.5 + .5 * var(--lvl, .5))`, with
    `transition: scale .1s linear`.
  - The product never exceeds 1, so the bars never leave the box.
  - With D < 48, render 22 bars (every other holder).
- **Still.** No animation. Each bar sits at a fixed gentle profile,
  `transform: scaleY(calc(.35 + .4 * (sin(i × .9) + 1) / 2))`, computed at render time. The level is ignored.

### 4.12 Speaking stays obvious

Every lit style shows something clearly at the lowest level:
- **Bars** never fall below `.35 × .5` of their length. That's a ring of short stubs.
- **Dots** never fall below scale `.48`.
- **Liquid** never falls below `scale(.88)`, at opacity `.85`.

On the stage, the tile's border also lights in `--speak`. A soundboard or join sound lights the ring with no voice
level. `var(--lvl, .5)` then gives the reactive styles a calm mid-level loop.

## 5. The levels pipeline

### 5.1 Where the levels come from today

- **Received voices.**
  - `MixerInput::push_in` (`client/src-tauri/src/voice/mixer.rs`) feeds every received sample into that source's
    `meter: Meter` (`voice/meter.rs`, a read-and-reset RMS). It does this after the receive-side leveller
    (`voice/leveller.rs` runs in each voice track's receive task, before the push) and after the person's own
    volume (`p.gain`).
  - So the meter is what you actually hear.
  - `MixerInput::take_peer_levels()` drains every meter and returns `(user id, rms)` per person, voice only. Streams
    are drained but never reported, and the loudest track wins if someone has two.
- **You.** `Shared::take_sent_level()` (`voice/devices.rs`) drains `sent_meter`, which holds what your mic really
  sends.
- **Speaking.**
  - The session's 100ms task in `VoiceManager`'s join path (`voice/mod.rs`, the block commented "levels for the UI
    meter + speaking ring") calls both of these every tick.
  - It feeds them to `speaking::SpeakingTracker` (`voice/speaking.rs`: lit at RMS ≥ `SPEAKING_RMS` 0.006, held
    300ms).
  - It emits `VoiceEvent::Speaking { user_ids }` only on change. The UI keeps that as `voice.speaking`
    (`client/ui/src/lib/voice.svelte.ts`).
- **The leveller.** It also measures each voice (K-weighted loudness for its gain), but per track and inside the
  receive task. The ring uses the mixer meter instead (doc's call). It's already per person, already includes
  personal volume and local mutes (a muted person reads 0, as their ring does today), and it's the same number that
  lights the ring.

### 5.2 Core changes

1. **Move the ring work onto its own faster tick.**
   - Take the three lines from the 100ms task (`take_peer_levels`, `take_sent_level`, `SpeakingTracker::update`)
     and put them in a new task in the same session with `tokio::time::interval(70ms)` (about 14 Hz) and
     `MissedTickBehavior::Skip`. Everything else stays on the 100ms task: meters, stats, noise suppression, echo,
     mic silence, muted talk.
   - Both readers are read-and-reset, so they must have exactly one caller. `mixer_levels()` (a test helper) and
     the mixer's tests are the only other callers.
   - `SpeakingTracker`'s hold is time-based, so the faster tick doesn't change its behaviour. The existing
     `speaking.rs` tests stay as they are.
2. **Map RMS to 0..1** in a new pure function, `ring_level(rms: f32) -> f32`, in `speaking.rs` or a new
   `voice/ringlevels.rs`:
   - Take `db = 20·log10(rms)`, then `clamp((db + 44) / 30, 0, 1)`.
   - −44 dBFS is about `SPEAKING_RMS`, the speaking threshold. −14 dBFS is loud speech after the leveller (which
     steers to about −20 LUFS).
3. **Smooth and quantise it** with a small per-person state, `RingLevels { shown: HashMap<String, f32>, last_sent }`:
   - Attack is instant. Release is `shown = max(new, shown × 0.7)` per tick, a fall of about 0.6s from full.
   - Round to steps of 0.05. Bars then fall back smoothly instead of flickering, and unchanged frames can be
     skipped.
4. **Emit.** Each tick, with `speaking` being the tracker's current set (add a `current()` accessor):
   - **Nobody is watching** (§5.3): emit nothing.
   - **`speaking` is empty:** emit one final `{}` if the last frame sent wasn't empty, then nothing until someone
     speaks.
   - **Otherwise:** build `{ user id → level }` for the ids in `speaking` only, and emit it unless it equals the last
     frame sent.
5. **The event.**
   - A new variant, `VoiceEvent::SpeakingLevels { levels: BTreeMap<String, f32> }`, with levels rounded to 2
     decimals. It serialises as `{"kind":"speaking_levels","levels":{"<id>":0.45}}`.
   - It goes on the existing `voice://event` channel.
   - Add the TypeScript case to the `VoiceEvent` union in `client/ui/src/lib/voiceui.ts`.
   - At 8 speakers it's under 300 bytes, at most 14 a second.

### 5.3 Subscribing: only visible tiles

- **The core.**
  - A new `Controls` flag, `ring_view: bool`, with `watch_rings(on)` and `rings_shown()`. Reset it in
    `reset_page_views()` next to `reset_meters()`.
  - A new command, `watch_voice_rings(on: bool)` in `commands.rs`, registered in `lib.rs` beside
    `watch_voice_meters`.
  - It's a flag, not a counter (doc's call), because the UI does the counting.
- **The UI.** A new module, `client/ui/src/lib/ringLevels.svelte.ts`:
  - It keeps the set of tiles watching. On 0 → 1 it invokes `watch_voice_rings({ on: true })`, and on 1 → 0 it
    invokes it with `on: false`. One invoke per transition, not per tile.
  - A tile watches while all of these hold:
    - it's mounted
    - it's on screen (an IntersectionObserver: export `observe()` from `lib/animate.svelte.ts` and reuse it)
    - `document.visibilityState === 'visible'`
    - its person's **effective** style is reactive: Liquid, Wave of dots or Bars, after the Classic-for-everyone
      setting, reduced motion and `motion.streaming` are applied
  - The cap (§8.3) doesn't affect watching, because the levels are cheap.

### 5.4 Applying levels in the UI

- In `voice.svelte.ts`'s `voice://event` listener, handle `speaking_levels` **before**
  `voice.stage = reduceStage(...)`, and return early. It must not touch any `$state`: a reactive write 14 times a
  second would re-render every tile.
- The listener hands the map to `ringLevels.apply(levels)`. That keeps `Map<userId, Set<HTMLElement>>` (the
  `SpeakingRing` roots registered through an `{@attach}`), and for each listed user it calls
  `el.style.setProperty('--lvl', String(v))` on that user's elements.
- An empty map sets `--lvl` to `0` on every registered element.
- An element registered mid-call starts with no `--lvl`, so the CSS fallback `.5` applies until the next frame.
- The CSS reads `--lvl` only inside `scale:` declarations, which carry `transition: scale .1s linear`. That smooths
  the 70ms steps into continuous motion on the compositor.
- **Never transition or animate `--lvl` itself.** Even registered with `@property`, an animated custom property
  runs on the main thread every frame.
- The Wave of dots speed buckets (§4.10) are computed in the same `apply` call, per element.

## 6. Storage and API

This follows the custom typing text precedent (#652, commit `a8567d1a`) field for field.

### 6.1 Migration
`server/migrations/0085_speaking_ring.sql`. 0085 is the next free number on `main` today. Take the next free one at
build time.
```sql
-- #651: the speaking ring others see. NULL is the default (Classic, green).
ALTER TABLE users ADD COLUMN ring_style TEXT;
ALTER TABLE users ADD COLUMN ring_color TEXT;
```

### 6.2 Keys

In `server/src/profiles/mod.rs`, beside `NAME_EFFECTS`:
```rust
pub const RING_STYLES: [&str; 11] = ["classic", "glow", "pulse", "ripple", "sweep", "orbit", "halo", "viewfinder", "liquid", "dots", "bars"];
pub const RING_COLORS: [&str; 9] = ["green", "blue", "gold", "pink", "cyan", "orange", "white", "purple", "profile"];
```
Each list gets a literal-copy test on both sides: the server's `keys_match_the_ui_list`, and
`client/ui/src/lib/speakingRing.test.ts`. That's the same pattern as `NAME_EFFECTS` and `nameEffects.test.ts`.

The keys are strings, not enums, so a client reading a key it doesn't know still reads the user. It draws Classic or
green instead.

### 6.3 Protocol (`protocol/src/rest.rs`)

- **`User`** gets two fields, each
  `#[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)]`:
  - `ring_style: Option<String>`, with the doc comment "the speaking ring's style (#651); absent: Classic (and from
    older servers; older clients ignore it)"
  - `ring_color: Option<String>`, with the doc comment "absent: green"
  - Absent when default, so unchanged users carry nothing new on the wire.
- **`UpdateProfileRequest`** gets the same two fields, with the same attributes. Absent keeps the current value,
  because older clients don't send them. A key sets it.
- Regenerate the ts-rs bindings: `client/ui/src/lib/protocol/User.ts` and `UpdateProfileRequest.ts`.

### 6.4 Server
- **`validate`.**
  - A present key not in its list → `400 "Unknown speaking ring."` or `400 "Unknown ring colour."`.
  - `Validated` gets `ring_style: Option<Option<String>>` and `ring_color: Option<Option<String>>`:
    - `None` keeps the current value.
    - `Some(None)` stores NULL. That's what `"classic"` and `"green"` store, so "back to default" leaves the user
      with nothing extra.
    - `Some(Some(k))` stores `k`.
- **`save`** (`profiles/routes.rs`). The `UPDATE` gets
  `ring_style = CASE WHEN ? THEN ? ELSE ring_style END` (and the same for `ring_color`), as `typing_text` does.
  `ProfileUpdated` already publishes the new `User` to everyone who knows them.
- **`users.rs`.**
  - Add `u.ring_style, u.ring_color` to `USER_COLS` and to `UserRow`.
  - `into_user` keeps only known keys, like `name_effect`. A row from a newer server's key reads as default.
- **`blocks::emptied_user`.** Reset both to `None`. Someone blocked sees the plain green ring, as they see no name
  style (#474).
- **Compiler-guided.** Every `User { .. }` literal (protocol tests, `auth/routes.rs`, the client's `commands.rs` and
  `api.rs` tests) gets the two fields as `None`. Members, people, `/me`, profiles and `ProfileUpdated` all carry the
  fields through `User` with no other change, as typing text does.
- **Clients and servers out of step.**
  - An old client against a new server ignores the fields, and its saves keep them because they're absent.
  - A new client against an old server saves fine, but the ring doesn't stick: the returned profile has none, so
    the draft goes back to Classic. That's acceptable for a friends server that updates together.

## 7. Settings UI

### 7.1 Profile settings: "Speaking ring"

`ProfileSettings.svelte` has tabs (About you, Card, Name style, Badges). Add a **Speaking ring** tab after Name style
(doc's call: the tabs are this page's sections). Like every tab, it edits the draft, and the page's Save sends it.

- **Draft** (`lib/profileDraft.ts`).
  - Add `ringStyle: string` and `ringColor: string`.
  - `draftFrom` fills them from `user.ring_style ?? 'classic'` and `user.ring_color ?? 'green'`, falling back to
    those for an unknown key.
  - `toRequest` always sends both.
  - `isDirty` picks them up automatically.
  - `previewUser` carries them, so the preview and the rest of the page show the draft.
- **Layout** (as the canvas's "Profile settings: pick your ring"):
  - A heading, **Speaking ring**, with the subtitle "How your avatar lights up when you talk. Everyone in the call
    sees it."
  - Then a row: the **preview** on the left, the controls on the right. They stack on narrow widths.
- **Preview.**
  - A 200 × 200 panel (`--bg-1`, 1px `--bg-3` border, `--radius-xl`) holding your avatar at 96px with
    `SpeakingRing place="preview"`, in your draft style and colour, permanently "talking".
  - For reactive styles, a fake speech envelope sets `--lvl`: a `setInterval` of 70ms with a cheap pseudo-random
    walk that has pauses. It runs only while the tab is shown.
  - Below it, the caption "Preview · talking".
  - Under reduced motion it shows the still version, with the caption "Preview · still (reduced motion is on)".
- **Style grid.**
  - Two labelled groups:
    - **Still or looping**: Classic, Soft glow, Pulse, Ripple, Sweep, Orbit, Halo, Viewfinder
    - **Follows your voice**: Liquid, Wave of dots, Bars round the ring
  - Each group is a 3-column grid of text chips (`minmax(0, 1fr)`, ellipsis, never wider than its cell).
  - All of them make up one `role="radiogroup" aria-label="Ring style"` with roving tabindex. Reuse the page's
    `radioKeys`.
  - The chips themselves don't animate. Only the preview does.
- **Colour.**
  - One `role="radiogroup" aria-label="Ring colour"` of 28px round swatches, in the order of §3.4. Each is drawn in
    its **current theme's** token, so a light-theme user sees the ink "White". Each has an `aria-label` with its
    name.
  - A ninth swatch, **Profile colour**, is drawn in your name colour, with the tip "Your name colour, from Name
    style". With no name colour set, it shows green and the tip adds "You haven't set one, so it's green."
- **Note.** A muted note under the controls: "With reduced motion on, everyone sees the still version of each ring.
  In big calls some rings show still to keep things smooth."
- **Containers.** The tab fits its pane at every width. The grid cells ellipsise, the swatches wrap, and the preview
  panel never shrinks below 200px. When there's no room beside it, the row stacks instead.

### 7.2 Voice settings: "Show everyone's rings as Classic"

- In `VoiceSettings.svelte`, add one row in the existing `.toggles` style, after the audio toggles:
  - **Show everyone's rings as Classic**
  - "Shows every speaking ring as the plain ring. Lighter on slower PCs, and calmer if motion distracts you."
- Off by default.
- **This device only.** It lives in localStorage under `pulse.classicRings`, read and written with try/catch, like
  `pulse.animateEmoji` in `animate.svelte.ts`. It doesn't go in `AudioConfig`, because that goes to the core and
  this is UI-only (doc's call).
- **On:**
  - Every ring renders as Classic. **Colours are kept** (doc's call): colour isn't motion, and it's still who they
    are.
  - No tile watches levels, so the core sends none.

## 8. Performance budget and tests

### 8.1 Rules (owner)
1. Ring animations move only `transform`, `opacity` and the `scale` property, each on its own small layer. No
   `@keyframes` or `transition` in ring CSS touches `box-shadow`, `filter`, `width`, `height`, colours,
   `background` or a custom property.
2. Blur is static. There's no `filter` in ring CSS at all: soft edges are radial gradients, painted once.
3. Animation runs only while that person is speaking (`data-ring="on"`). Off tiles have `animation: none`.
4. Reduced motion means every style shows its still version.
5. More than 8 speaking means the extras show still.

### 8.2 Budget
- **Animated elements per ring:**
  - Classic and Glow: 0
  - Pulse, Halo, Sweep, Orbit and Viewfinder: 1. Viewfinder's runs only for its 0.18s entry.
  - Ripple: 2
  - Liquid: 4
  - Wave of dots: 24 (12 below 48px)
  - Bars: 44 (22 below 48px)
  - Worst case: 8 Bars speakers is 352 tiny layers.
- **Paints.** After a ring starts there are no recurring paints. In a 10s performance trace with 8 speakers on Bars,
  there's no `Paint` event at frame rate for the tiles, and DevTools' Paint flashing shows no green on them.
- **Levels.** The core emits nothing in silence and nothing while no reactive tile is visible. At most 15 events a
  second otherwise.
- **Applying one levels event** at 8 speakers takes < 1ms on the main thread: ≤ 3 `setProperty` calls per person
  plus style recalc of ≤ 46 descendants each.
- **Acceptance target.** On the owner's Windows PC (WebView2, the primary platform), 8 simulated speakers on Bars
  add no visible jank. The whole app's CPU stays within about 3 percentage points of one core above the same call
  with Classic. Check it once with the harness in §8.4 and note the figure in the PR.

### 8.3 The cap
- `ringLevels.svelte.ts` (or `speakingRing.svelte.ts`) keeps a start counter. When an id enters `voice.speaking`, or
  the soundboard's playing set, it gets the next number. When it leaves, it's dropped.
- A ring animates only if its rank among those speaking now is < 8.
- When someone stops, the next in line starts animating. Rings that were already animating never drop to still
  because someone new started (doc's call: no flicker).

### 8.4 Tests
- **Core** (Rust unit tests):
  - `ring_level`: threshold → 0, −14 dBFS → 1, monotonic, 0 RMS → 0.
  - Smoothing and quantising: instant rise, 0.7 release, 0.05 steps.
  - The emitter:
    - only ids in the speaking set
    - nothing in silence
    - exactly one `{}` when the last speaker stops
    - nothing while unwatched
    - unchanged frames skipped
  - The serialisation shape test, beside the other `VoiceEvent` JSON tests in `voice/mod.rs`.
  - `Controls::watch_rings` and its reset on `reset_page_views`.
- **Server** (`server/tests/profiles.rs`, as #652):
  - set each key
  - absent keeps
  - `classic`/`green` store NULL, and `User` then omits the fields
  - unknown key → 400
  - `ProfileUpdated` carries them
  - a blocked viewer sees them emptied
  - the `RING_STYLES`/`RING_COLORS` literal-copy tests
- **UI** (vitest):
  - `speakingRing.test.ts`:
    - key lists (the literal copy)
    - resolving style and colour, including `profile` with and without a name colour, unknown keys, and blocked
    - the effective state (`off`/`on`/`still`) across reduced motion, streaming, Classic-for-everyone, the cap and
      the channel list
  - `ringLevels.svelte.test.ts`:
    - one `watch_voice_rings` invoke per 0 ↔ 1 transition
    - not watching for a non-reactive or off-screen tile
    - `apply` sets `--lvl` on the right elements without touching `$state`
    - an empty map zeroes them
    - speed buckets with hysteresis
  - `SpeakingRingCss.test.ts`, in the style of `FullProfileCss.test.ts`. It reads the component's `<style>` and
    asserts:
    - every `@keyframes` body sets only `transform`/`opacity`
    - every `transition` names only `opacity`/`scale`
    - no `filter`
    - `--lvl` appears only inside `scale:` declarations
  - Update `UserTile.ring.test.ts`, `ChannelList` and `CallBand` tests for `--speak` and the new component.
  - `ProfileSettings.test.ts`: the tab, the radiogroups and keyboard, the draft round-trip, and the profile swatch
    tip.
  - `VoiceSettings.test.ts`: the toggle persists and turns rings Classic.
  - A colour-token test: every theme declares every `--speak-*`, with the contrast floors from §3.4.
- **Harness** (doc's call). A dev-build-only `window.__ringStress(n, style)`, behind `import.meta.env.DEV`. It fakes
  `n` speakers in the current stage, with a synthetic `speaking_levels` feed, for the trace in §8.2. It isn't
  shipped in release builds.

## 9. Approaches considered

1. **Canvas or WebGL per tile (a real waveform).** Rejected by the owner. It repaints every frame for every speaker.
2. **Levels in Svelte `$state`, rendered into inline styles.** Rejected. It re-renders every tile 14 times a second.
   Chosen instead: write `--lvl` straight onto the element, and let a compositor transition on `scale` do the
   smoothing.
3. **Levels on the existing 100ms tick (10 Hz).** Rejected: the owner asked for 12–15 Hz. A second reader on the
   same 100ms task would split the read-and-reset meters. Chosen instead: move the meter readers and the speaking
   tracker together onto one 70ms task.
4. **The UI polling the core for levels.** Rejected: an invoke per tile per tick, and work in silence. Push only
   while someone speaks and someone watches, like the meters (`watch_voice_meters`) and link stats.
5. **The leveller's loudness as the ring level.** Rejected. It's per track, measured before personal volume and
   local mute, and lives in each receive task. The mixer meter is per person and is what you hear and what lights
   the ring.
6. **`filter: blur` for Halo and Liquid** (as on the canvas). Rejected. A blurred parent with moving children is
   re-blurred every frame, and the blur's spread overflows the box. Radial-gradient edges look the same and are
   painted once.
7. **Sweep's centre punched by a solid disc** (as on the canvas). Rejected. It shows over game backdrop art. A
   static CSS `mask` rotates with the layer for free.
8. **Changing `animation-duration` for "faster when loud".** Rejected: phase jumps. Chosen instead:
   `updatePlaybackRate` on three hysteresis buckets.
9. **Ring style as a device setting.** Rejected. Others must see your ring, so it's a profile field. Only the viewer
   override is per device.
10. **Not reserving space, and letting rings overlap the name.** Rejected under the owner's no-spill rule. Chosen
    instead: a fixed 1.32 D ring box, reserved by margin, with a deliberate tile clip as the last resort.

## 10. Left to the builder

- The exact light-theme shades of the palette (§3.4), within the contrast test.
- The final `tileScale` formula that reserves the ring box (§3.3), tuned against the stage layout tests.
- The fake speech envelope for the preview (§7.1). Any cheap random walk with pauses will do.
- File and test names where this doc only suggests them. Whether the start counter and cap live in
  `ringLevels.svelte.ts` or `speakingRing.svelte.ts`.
- The migration number, if 0085 is taken by then.
- Recording the §8.2 acceptance figure in the PR.

## 11. Builder check: questions settled

These came from reading this doc as a fresh builder. Each one is the doc's call, not an owner decision.

1. **What is "my profile colour"?** The first colour of your name style (`User.style.colors[0]`). It's the only
   colour on `User`, so every tile has it without fetching a profile. The card's theme colours aren't on `User`.
   Green if unset.
2. **What's the 8th colour?** Purple, `#b07cf0`. Red is out because it reads as the mute badge.
3. **White on the light theme?** It's drawn in the theme's text ink, and the swatch shows exactly that.
4. **Does "Show everyone's rings as Classic" also reset colours?** No. Only the style. Colour isn't motion.
5. **What colour is the 22px channel-list ring?** The person's colour, static.
6. **Rings while I'm live (streaming)?** Still versions, as animated emoji already do while you're live.
7. **Soundboard-lit rings on a reactive style?** The style runs at a fixed mid level (`--lvl` fallback `.5`).
8. **Does my own tile react?** Yes. Your level is `take_sent_level()` (what you send), already in the speaking list.
9. **Unfocused window?** Rings keep animating (it's a call you glance at). A hidden window stops watching levels, and
   the browser throttles hidden animations anyway.
10. **Which 8 rings animate under the cap?** The first 8 to start speaking. Animating rings never drop to still.
11. **What about Viewfinder's entry animation, and Ripple's scale?** Shrunk from the canvas (1.15 → 1.06,
    1.4 → 1.18) to fit the ring box.
12. **Who draws Classic?** It moves into `SpeakingRing` with today's exact shadow values, so the layout and look
    don't change for anyone who keeps Classic. The only global change is the reserved ring box (§3.3).
13. **Does the tile border follow the colour?** Yes, `var(--speak)`.
14. **Should reactive styles also have a looping animation?** Yes. The loop gives the motion and the level scales its
    size. A single level per person can't move 44 bars differently on its own.
15. **Is the watching state a counter or a flag in the core?** A flag. The UI counts and invokes only on 0 ↔ 1.
16. **Does a user with default settings send anything new?** No. Both fields are absent on `User` when they're
    Classic or green.
