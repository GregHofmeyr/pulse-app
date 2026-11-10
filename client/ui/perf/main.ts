// Perf harness (#238): the real app in a browser, over a mocked Tauri core with realistic volumes, so channel
// switches and Settings can be traced with CPU throttling (perf/measure.mjs). Dev only: `pnpm dev`, then open
// http://localhost:1420/perf/?page=200 (`page`: messages in the first page of a channel's history, default 50).
import { emit } from '@tauri-apps/api/event'
import { mockConvertFileSrc, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { history, me, ready } from './fixtures'

const firstPage = Number(new URLSearchParams(location.search).get('page') ?? 50)
const unknown = new Set<string>()

mockWindows('main')
mockConvertFileSrc('linux')
mockIPC(
  (cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>
    switch (cmd) {
      case 'restore_session':
        // after App's listeners are in: Ready, then connected
        setTimeout(() => {
          void emit('pulse://ready', ready)
          void emit('pulse://conn', 'connected')
        }, 0)
        return { status: 'ready', user: me }
      case 'update_launch':
        return null
      case 'list_messages': {
        const all = history(a.channelId as string, 200)
        const before = a.before as string | null
        const end = before ? all.findIndex((m) => m.id === before) : all.length
        const size = before ? 50 : firstPage
        return all.slice(Math.max(0, end - size), end)
      }
      case 'list_messages_after':
      case 'list_pins':
        return []
      default:
        if (!unknown.has(cmd)) {
          unknown.add(cmd)
          console.debug('[perf] unmocked', cmd)
        }
        return null
    }
  },
  { shouldMockEvents: true },
)

await import('../src/main')
