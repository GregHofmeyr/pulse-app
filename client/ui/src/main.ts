import { mount } from 'svelte'
// Bundled locally: the app makes no third-party requests (CSP blocks them anyway).
import './fonts.css'
import './nameFonts.css'
import './app.css'
import { markEngine } from './lib/engine'
import { markFinishedEntrances } from './lib/entrances'
import { applyTheme } from './lib/theme.svelte'
import { logUncaughtErrors } from './lib/uiErrors'
import { windowKind } from './lib/windowKind'

// The build pop-out (#620) is this page too: it may call only its own three commands (ui_log isn't one).
const popout = windowKind(location) === 'build'

// Uncaught page errors go to the log file too, from the start.
if (!popout) logUncaughtErrors()

// The saved theme before the first paint (no flash of Pulse Dark on Light).
applyTheme()
// The engine's class, before anything opens (#103).
markEngine()
// A finished entrance never replays when focus comes back (lib/entrances.ts).
markFinishedEntrances()

// The speaking rings' stress harness (#651), dev builds only: `__ringStress(8, 'bars')` in the console.
if (import.meta.env.DEV) void import('./lib/ringStress').then((m) => m.install())

// Loaded on demand so the pop-out never loads the app (or its store, which would start listening and calling).
const target = document.getElementById('app')!
if (popout) void import('./BuildPopout.svelte').then(({ default: BuildPopout }) => mount(BuildPopout, { target }))
else void import('./App.svelte').then(({ default: App }) => mount(App, { target }))
