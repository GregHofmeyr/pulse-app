// Measures channel switches, server switches and Settings opening in the perf harness (#238), at 1x and 4x CPU
// throttle, over the Chrome DevTools Protocol (Node's own WebSocket; no extra packages).
//
//   node perf/measure.mjs --chrome <chrome binary> --rounds 7 label=http://localhost:14238/perf/ [label2=url2 ...]
//
// Several builds can be compared at once (e.g. before=… after=…): each round runs every build in a fresh tab, in an
// order that alternates, so load on the machine falls on all of them alike. Medians per build, scenario and throttle.
// Per interaction:
//   shown   — the press to the first frame painted with the result (messages in the list, the Settings dialog)
//   longest — the longest main-thread task in the 1.5 s after the press
//   total   — all main-thread task time in those 1.5 s (work moved later still counts here)
import { spawn } from 'node:child_process'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const argv = process.argv.slice(2)
const opt = (k, d) => {
  const i = argv.indexOf(`--${k}`)
  return i >= 0 ? argv.splice(i, 2)[1] : d
}
const chromeBin = opt('chrome')
const rounds = Number(opt('rounds', 5))
const only = opt('only', '') // run just the scenarios whose name has this in it
const throttles = opt('throttle', '1,4').split(',').map(Number)
// --trace: also record a DevTools trace of each measured press and print where its main-thread time went
const tracing = argv.includes('--trace') && argv.splice(argv.indexOf('--trace'), 1)
const builds = argv.map((a) => { const i = a.indexOf('='); return { label: a.slice(0, i), url: a.slice(i + 1) } })
if (!chromeBin || !builds.length) throw new Error('usage: --chrome <bin> label=url …')

const port = 9300 + Math.floor(Math.random() * 500)
const chrome = spawn(chromeBin, [
  '--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), 'pulse-perf-'))}`,
  '--window-size=1600,1000', '--no-first-run', '--no-default-browser-check', '--disable-extensions', '--mute-audio',
  '--disable-background-timer-throttling', '--disable-renderer-backgrounding', 'about:blank',
], { stdio: 'ignore' })
process.on('exit', () => chrome.kill())

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
async function json(path, method = 'GET') {
  for (let i = 0; ; i++) {
    try { return await (await fetch(`http://127.0.0.1:${port}${path}`, { method })).json() } catch (e) { if (i > 50) throw e; await sleep(100) }
  }
}

async function openTab(url) {
  const t = await json(`/json/new?about:blank`, 'PUT')
  const ws = new WebSocket(t.webSocketDebuggerUrl)
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j })
  let id = 0
  const waiting = new Map()
  const traceEvents = []
  let traceDone = null
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data)
    if (m.method === 'Tracing.dataCollected') traceEvents.push(...m.params.value)
    if (m.method === 'Tracing.tracingComplete') traceDone?.()
    if (m.id && waiting.has(m.id)) { const w = waiting.get(m.id); waiting.delete(m.id); m.error ? w.j(new Error(JSON.stringify(m.error))) : w.r(m.result) }
  }
  const send = (method, params = {}) => new Promise((r, j) => { const i = ++id; waiting.set(i, { r, j }); ws.send(JSON.stringify({ id: i, method, params })) })
  const evaluate = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true })
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text)
    return r.result.value
  }
  await send('Page.enable')
  await send('Runtime.enable')
  await send('Performance.enable', { timeDomain: 'timeTicks' })
  await send('Page.navigate', { url })
  const trace = {
    start: () => { traceEvents.length = 0; return send('Tracing.start', { categories: 'devtools.timeline,disabled-by-default-devtools.timeline', transferMode: 'ReportEvents' }) },
    stop: () => new Promise((r) => { traceDone = () => r(traceEvents); void send('Tracing.end') }),
  }
  return { send, evaluate, trace, close: () => { ws.close(); return fetch(`http://127.0.0.1:${port}/json/close/${t.id}`).then((r) => r.text()) } }
}

// In the page: long tasks, and a probe for "the first frame painted after the press where `ready()` holds".
const PROBE = `(() => {
  window.__long = []
  new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__long.push([e.startTime, e.duration]) }).observe({ type: 'longtask' })
  window.__press = 0
  addEventListener('pointerdown', () => (window.__press = performance.now()), true)
  window.__shown = (ready) => new Promise((resolve) => {
    const frame = () => requestAnimationFrame(() => {
      if (!ready()) return frame()
      // after this frame's paint
      const ch = new MessageChannel()
      ch.port1.onmessage = () => resolve(performance.now() - window.__press)
      ch.port2.postMessage(0)
    })
    frame()
  })
})()`

async function click(tab, selector) {
  // `tab:Name` is the server tab with that name
  const find = selector.startsWith('tab:')
    ? `[...document.querySelectorAll('.tab')].find((t) => t.textContent.includes(${JSON.stringify(selector.slice(4))}))`
    : `document.querySelector(${JSON.stringify(selector)})`
  const r = await tab.evaluate(`(() => { const el = ${find}; if (!el) return null; el.scrollIntoView({ block: 'nearest' }); const b = el.getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2] })()`)
  if (!r) throw new Error(`no ${selector}`)
  const [x, y] = r
  await tab.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y })
  await tab.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 })
  await tab.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 })
}

const taskTime = async (tab) => (await tab.send('Performance.getMetrics')).metrics.find((m) => m.name === 'TaskDuration').value * 1000

async function interaction(tab, selector, ready) {
  await sleep(300)
  await tab.evaluate('window.__long = []')
  const t0 = await taskTime(tab)
  if (tracing) await tab.trace.start()
  const shownP = tab.evaluate(`window.__shown(() => ${ready})`)
  await click(tab, selector)
  const shown = await Promise.race([shownP, sleep(30000).then(async () => { throw new Error(`never shown: ${selector} / ${ready} / ${await tab.evaluate("[...document.querySelectorAll('.tab.active, .row.active')].map((e) => e.textContent.trim() + ' ' + (e.dataset.channel ?? '')).join(' | ')")}`) })])
  if (process.env.PERF_DEBUG) process.stderr.write(`${selector}: ${Math.round(shown)}\n`)
  await sleep(1500)
  const total = (await taskTime(tab)) - t0
  const longest = Math.max(0, ...(await tab.evaluate('window.__long.map((l) => l[1])')))
  if (tracing) summarise(selector, await tab.trace.stop())
  return { shown, longest, total }
}

/** Main-thread time by event, inside the tasks over 16 ms, of the renderer the press went to. */
function summarise(label, events) {
  const main = events.find((e) => e.name === 'thread_name' && e.args?.name === 'CrRendererMain' && events.some((x) => x.pid === e.pid && x.name === 'EventDispatch'))
  if (!main) return
  const on = events.filter((e) => e.pid === main.pid && e.tid === main.tid && e.ph === 'X' && e.dur)
  const tasks = on.filter((e) => e.name === 'RunTask' && e.dur > 16000)
  const by = {}
  for (const t of tasks) for (const e of on) {
    if (e === t || e.ts < t.ts || e.ts + e.dur > t.ts + t.dur) continue
    if (!['FunctionCall', 'EvaluateScript', 'v8.callFunction', 'TimerFire', 'FireAnimationFrame', 'EventDispatch', 'RunMicrotasks', 'UpdateLayoutTree', 'Layout', 'Paint', 'PrePaint', 'Layerize', 'ParseHTML', 'HitTest', 'Commit', 'Decode Image', 'IntersectionObserverController::computeIntersections', 'ScheduleStyleRecalculation', 'InvalidateLayout', 'MajorGC', 'MinorGC', 'V8.GC_SCAVENGER', 'BlinkGC.AtomicPhase'].includes(e.name)) continue
    by[e.name] = (by[e.name] ?? 0) + e.dur / 1000
  }
  process.stderr.write(`trace ${label}: tasks>16ms ${tasks.map((t) => Math.round(t.dur / 1000)).join('+')} ms; ` + Object.entries(by).sort((a, b) => b[1] - a[1]).map(([k, v]) => `${k} ${v.toFixed(1)}`).join(', ') + '\n')
}

const msgsIn = (ch) => `!!document.querySelector('.content .list [data-message-id^="${ch}-M"]')`
const SCENARIOS = [
  // the first visit to a channel: its history is fetched and drawn
  { name: 'channel first visit', run: (tab) => interaction(tab, '[data-channel="S1-C001"]', msgsIn('S1-C001')) },
  // back to a channel whose history is loaded: drawn at once
  { name: 'channel switch (loaded)', run: async (tab) => {
    await interaction(tab, '[data-channel="S1-C002"]', msgsIn('S1-C002'))
    return interaction(tab, '[data-channel="S1-C001"]', msgsIn('S1-C001'))
  } },
  // a server opens on its first channel
  { name: 'server switch (loaded)', run: async (tab) => {
    await interaction(tab, 'tab:Games', msgsIn('S2-C000'))
    await interaction(tab, 'tab:Main', msgsIn('S1-C000'))
    await interaction(tab, 'tab:Games', msgsIn('S2-C000'))
    return interaction(tab, 'tab:Main', msgsIn('S1-C000'))
  } },
  { name: 'settings open', run: async (tab) => {
    const r = await interaction(tab, 'button.me-btn[aria-label="Settings"]', `!!document.querySelector('[role=dialog][aria-label=Settings]')`)
    await tab.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 })
    await tab.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 })
    return r
  } },
]

const scenarios = SCENARIOS.filter((s) => s.name.includes(only))
const results = {} // label → scenario → throttle → metric → []
await json('/json/version')
for (let round = 0; round < rounds; round++) {
  const order = round % 2 ? [...builds].reverse() : builds
  for (const b of order) {
    for (const rate of throttles) {
      const tab = await openTab(b.url)
      const until = (cond) => tab.evaluate(`new Promise((r) => { const t = setInterval(() => { if (${cond}) { clearInterval(t); r() } }, 50) })`)
      await until(`document.querySelector('.tab')`)
      await tab.evaluate(PROBE)
      // into the server: it opens on its first channel
      await sleep(500)
      await click(tab, 'tab:Main')
      if (process.env.PERF_DEBUG) process.stderr.write('in Main\n')
      await until(msgsIn('S1-C000'))
      if (process.env.PERF_DEBUG) process.stderr.write('S1-C000 drawn\n')
      await sleep(800)
      await tab.send('Emulation.setCPUThrottlingRate', { rate })
      for (const s of scenarios) {
        const r = await s.run(tab)
        for (const [k, v] of Object.entries(r)) (((results[b.label] ??= {})[s.name] ??= {})[rate] ??= {})[k] ??= []
        for (const [k, v] of Object.entries(r)) results[b.label][s.name][rate][k].push(v)
      }
      await tab.send('Emulation.setCPUThrottlingRate', { rate: 1 })
      await tab.close()
    }
  }
  process.stderr.write(`round ${round + 1}/${rounds}\n`)
}

const median = (a) => { const s = [...a].sort((x, y) => x - y); const m = s.length >> 1; return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2 }
const rows = []
for (const s of scenarios) for (const rate of throttles) for (const b of builds) {
  const r = results[b.label][s.name][rate]
  rows.push({ scenario: s.name, cpu: `${rate}x`, build: b.label, 'shown ms': Math.round(median(r.shown)), 'longest task ms': Math.round(median(r.longest)), 'total task ms': Math.round(median(r.total)) })
}
console.table(rows)
console.log(JSON.stringify(results))
process.exit(0)
