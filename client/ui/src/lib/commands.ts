// Slash commands typed in the message box (#262): the one registry the composer suggests from (type `/`), shows
// the usage tip for, and TextChannel runs on send.
//
// To add a command, add one entry to COMMANDS below (`/poll` is the simplest to copy), and, if `run` needs something new from the chat (a popover to open, say), add it to CommandContext and pass it
// where TextChannel builds its context (`commandContext`).

import { parsePollArgs } from './poll'
import { addOptions, splitOptions, WHEEL_LIMITS } from './wheel'

/** What a command can do in the chat it was sent from. TextChannel provides it. */
export interface CommandContext {
  /** Spin a quick wheel with these options (two or more) straight away. */
  spinWheel: (options: string[]) => void
  /** Open the composer's wheel popover, starting a quick wheel with `prefill`. */
  openWheel: (prefill: string[]) => void
  /** Open the poll composer (#157), filled with what was typed after `/poll`. */
  openPoll: (prefill: { question: string; options: string[] }) => void
}

export interface SlashCommand {
  /** Typed after the `/`, lower case. */
  name: string
  /** How it's written, shown in the suggestion row and the tip, e.g. `/wheel option1, option2, option3`. */
  usage: string
  /** One line on what it does (the suggestion row). */
  description: string
  /** What Enter does, after the usage in the tip while its arguments are typed, e.g. `Enter to spin`. */
  hint: string
  /** Run it: `args` is everything typed after the name (trimmed, may be empty). */
  run: (args: string, ctx: CommandContext) => void
}

export const COMMANDS: SlashCommand[] = [
  {
    name: 'wheel',
    usage: '/wheel option1, option2, option3',
    description: 'Spin a quick wheel (or just /wheel to open it)',
    hint: 'Enter to spin',
    // two or more options spin at once (cut to the limit, repeats dropped, as the editor adds them); fewer open
    // the wheel popover with what was typed (#158)
    run: (args, ctx) => {
      const options = splitOptions(args)
      if (options.length >= WHEEL_LIMITS.minOptions) ctx.spinWheel(addOptions([], args).options)
      else ctx.openWheel(options)
    },
  },
  {
    name: 'poll',
    usage: '/poll Question? option1, option2',
    description: 'Start a poll',
    hint: 'Enter to set it up',
    // fills the poll composer (question up to the `?`, options after it) so the toggles can still be set (#157)
    run: (args, ctx) => ctx.openPoll(parsePollArgs(args)),
  },
]

/** The message `text` as a command from `commands`: the command and what's typed after its name. Null when it
 *  isn't one (a `/` word that isn't registered is sent as an ordinary message). */
export function parseCommand(text: string, commands: readonly SlashCommand[] = COMMANDS): { command: SlashCommand; args: string } | null {
  const m = /^\/(\w+)(?:\s+([\s\S]*))?$/.exec(text.trim())
  if (!m) return null
  const command = commands.find((c) => c.name === m[1].toLowerCase())
  return command ? { command, args: (m[2] ?? '').trim() } : null
}

/** The partial command name while a `/word` starting the message is typed with the caret at its end (`''` for a
 *  bare `/`), or null. Whatever follows the caret must start with a space (or be nothing). */
export function commandQuery(text: string, caret: number): string | null {
  const m = /^\/(\w*)$/.exec(text.slice(0, caret))
  if (!m) return null
  const after = text.slice(caret)
  return after === '' || /^\s/.test(after) ? m[1] : null
}

/** The commands whose name starts with `query`, in registry order. */
export function suggestCommands(query: string, commands: readonly SlashCommand[] = COMMANDS): SlashCommand[] {
  const q = query.toLowerCase()
  return commands.filter((c) => c.name.startsWith(q))
}

/** Complete the `/partial` before `caret` to `/name ` with the caret after the space (one space, whatever followed). */
export function applyCommand(text: string, caret: number, name: string): { text: string; caret: number } {
  const head = `/${name} `
  return { text: head + text.slice(caret).replace(/^[^\S\n]+/, ''), caret: head.length }
}

/** The command whose arguments are being typed (`/wheel ` and on), for the usage tip; null otherwise. */
export function typingCommand(text: string, commands: readonly SlashCommand[] = COMMANDS): SlashCommand | null {
  const m = /^\/(\w+)\s/.exec(text)
  return m ? (commands.find((c) => c.name === m[1].toLowerCase()) ?? null) : null
}
