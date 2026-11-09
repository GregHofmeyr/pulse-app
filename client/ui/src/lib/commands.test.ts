import { describe, expect, it, vi } from 'vitest'
import { applyCommand, commandQuery, COMMANDS, parseCommand, suggestCommands, typingCommand, type CommandContext } from './commands'

const ctx = (): CommandContext => ({ spinWheel: vi.fn(), openWheel: vi.fn(), openPoll: vi.fn() })

describe('the command registry', () => {
  it('has /wheel with its usage, description and tip', () => {
    const w = COMMANDS.find((c) => c.name === 'wheel')!
    expect(w.usage).toBe('/wheel option1, option2, option3')
    expect(w.description).toBe('Spin a quick wheel (or just /wheel to open it)')
    expect(w.hint).toBe('Enter to spin')
  })
  it('has /poll, which opens the poll composer filled with what was typed (never posts)', () => {
    const p = COMMANDS.find((c) => c.name === 'poll')!
    expect(p.usage).toBe('/poll Question? option1, option2')
    expect(p.description).toBe('Start a poll')
    expect(p.hint).toBe('Enter to set it up')
    const c = ctx()
    const parsed = parseCommand('/poll Where to eat? Pizza, Sushi')!
    parsed.command.run(parsed.args, c)
    expect(c.openPoll).toHaveBeenCalledWith({ question: 'Where to eat?', options: ['Pizza', 'Sushi'] })
    const d = ctx()
    parseCommand('/POLL')!.command.run('', d)
    expect(d.openPoll).toHaveBeenCalledWith({ question: '', options: [] })
  })
  it('names are unique, lower case words', () => {
    const names = COMMANDS.map((c) => c.name)
    expect(new Set(names).size).toBe(names.length)
    for (const n of names) expect(n).toMatch(/^[a-z0-9_]+$/)
  })
})

describe('parseCommand', () => {
  it('finds a registered command and what follows it', () => {
    expect(parseCommand('/wheel a, b')).toMatchObject({ command: { name: 'wheel' }, args: 'a, b' })
    expect(parseCommand('  /WHEEL  ')).toMatchObject({ command: { name: 'wheel' }, args: '' })
  })
  it('is nothing for unknown commands or ordinary text', () => {
    expect(parseCommand('/wheels a, b')).toBeNull()
    expect(parseCommand('/shrug')).toBeNull()
    expect(parseCommand('spin the /wheel a, b')).toBeNull()
  })
  it('/wheel reads comma-separated options; fewer than two (blanks dropped) open the popover', () => {
    const run = (text: string) => {
      const c = ctx()
      const p = parseCommand(text)!
      p.command.run(p.args, c)
      return c
    }
    expect(run('/wheel pizza, sushi,  braai ').spinWheel).toHaveBeenCalledWith(['pizza', 'sushi', 'braai'])
    expect(run('/WHEEL a,b').spinWheel).toHaveBeenCalledWith(['a', 'b'])
    expect(run('/wheel').openWheel).toHaveBeenCalledWith([])
    expect(run('  /wheel  ').openWheel).toHaveBeenCalledWith([])
    expect(run('/wheel , ,').openWheel).toHaveBeenCalledWith([])
    expect(parseCommand('wheel a, b')).toBeNull()
  })
  it('/wheel spins with two or more options, else opens the popover with what was typed', () => {
    const c = ctx()
    parseCommand('/wheel pizza, sushi, Pizza, pizza')!.command.run('pizza, sushi, Pizza, pizza', c)
    // exact repeats merge; case counts (#271)
    expect(c.spinWheel).toHaveBeenCalledWith(['pizza', 'sushi', 'Pizza'])
    const u = ctx()
    parseCommand('/wheel UPPER, upper')!.command.run('UPPER, upper', u)
    expect(u.spinWheel).toHaveBeenCalledWith(['UPPER', 'upper'])
    // the same option twice is one: the popover opens with it
    const twice = ctx()
    parseCommand('/wheel pizza, pizza')!.command.run('pizza, pizza', twice)
    expect(twice.spinWheel).not.toHaveBeenCalled()
    expect(twice.openWheel).toHaveBeenCalledWith(['pizza'])
    const d = ctx()
    parseCommand('/wheel pizza')!.command.run('pizza', d)
    expect(d.openWheel).toHaveBeenCalledWith(['pizza'])
    expect(d.spinWheel).not.toHaveBeenCalled()
  })
})

describe('suggesting', () => {
  it('opens on a / starting the message with the caret at the end of the word', () => {
    expect(commandQuery('/', 1)).toBe('')
    expect(commandQuery('/whe', 4)).toBe('whe')
    expect(commandQuery('/whe more', 4)).toBe('whe')
    expect(commandQuery('/wheel a', 8)).toBeNull()
    expect(commandQuery('hi /whe', 7)).toBeNull()
    expect(commandQuery('/whe', 2)).toBeNull()
    expect(commandQuery('/a/b', 4)).toBeNull()
  })
  it('matches by name prefix, any case', () => {
    expect(suggestCommands('').map((c) => c.name)).toContain('wheel')
    expect(suggestCommands('WH').map((c) => c.name)).toEqual(['wheel'])
    expect(suggestCommands('zz')).toEqual([])
  })
  it('completes to /name and one space, caret after it', () => {
    expect(applyCommand('/whe', 4, 'wheel')).toEqual({ text: '/wheel ', caret: 7 })
    expect(applyCommand('/whe  a, b', 4, 'wheel')).toEqual({ text: '/wheel a, b', caret: 7 })
  })
  it('knows the command whose arguments are being typed', () => {
    expect(typingCommand('/wheel ')?.name).toBe('wheel')
    expect(typingCommand('/wheel a, b')?.name).toBe('wheel')
    expect(typingCommand('/wheel')).toBeNull()
    expect(typingCommand('/nope a')).toBeNull()
    expect(typingCommand('hi /wheel a')).toBeNull()
  })
})
