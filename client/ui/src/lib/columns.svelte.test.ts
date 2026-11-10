// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { columns, reloadColumns, resetColumn, saveColumns, setColumn } from './columns.svelte'

beforeEach(() => {
  localStorage.clear()
  reloadColumns()
})

describe('saved column widths (#160)', () => {
  it('nothing stored: both default', () => {
    expect({ ...columns }).toEqual({ left: null, right: null, chat: null, strip: null })
  })

  it('a width is remembered on this device', () => {
    setColumn('right', 312.4)
    expect(JSON.parse(localStorage.getItem('pulse.columns')!)).toEqual({ left: null, right: 312, chat: null, strip: null })
    columns.right = null
    reloadColumns()
    expect(columns.right).toBe(312)
  })

  it('a drag moves the width live and writes it down once at the end', () => {
    setColumn('left', 300, false)
    expect(columns.left).toBe(300)
    expect(localStorage.getItem('pulse.columns')).toBeNull()
    saveColumns()
    expect(JSON.parse(localStorage.getItem('pulse.columns')!).left).toBe(300)
  })

  it("a voice channel's chat keeps its own width (#226): the members list's is left alone", () => {
    setColumn('right', 250)
    setColumn('chat', 420)
    reloadColumns()
    expect({ ...columns }).toEqual({ left: null, right: 250, chat: 420, strip: null })
    resetColumn('chat')
    reloadColumns()
    expect({ ...columns }).toEqual({ left: null, right: 250, chat: null, strip: null })
  })

  it('reset puts the default back and remembers that', () => {
    setColumn('left', 300)
    resetColumn('left')
    reloadColumns()
    expect(columns.left).toBeNull()
  })

  it('garbled or nonsense values are the defaults', () => {
    localStorage.setItem('pulse.columns', '{"left":"wide","right":-5}')
    reloadColumns()
    expect({ ...columns }).toEqual({ left: null, right: null, chat: null, strip: null })
    localStorage.setItem('pulse.columns', 'nope')
    reloadColumns()
    expect({ ...columns }).toEqual({ left: null, right: null, chat: null, strip: null })
  })

  it("the stream's chat strip (#63) keeps its own width, apart from the stage's chat", () => {
    setColumn('strip', 330)
    columns.strip = null
    reloadColumns()
    expect({ ...columns }).toEqual({ left: null, right: null, chat: null, strip: 330 })
  })
})
