import { describe, expect, it } from 'vitest'
import { classifyPaste, classifyText } from './buildPaste'

const data = (files: File[], text = ''): DataTransfer => ({ files, getData: (t: string) => (t === 'text/plain' ? text : '') }) as unknown as DataTransfer
const png = () => new File([new Uint8Array([137, 80, 78, 71])], 'shot.png', { type: 'image/png' })

describe('classifyPaste', () => {
  it('an image wins over text', () => {
    const f = png()
    expect(classifyPaste(data([f], 'https://youtu.be/x'))).toEqual({ kind: 'image', file: f })
  })

  it('a single http(s) URL is a link', () => {
    expect(classifyPaste(data([], '  https://www.youtube.com/watch?v=dQw4w9WgXcQ \n'))).toEqual({ kind: 'link', url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ' })
    expect(classifyPaste(data([], 'http://planner.example/raven'))).toEqual({ kind: 'link', url: 'http://planner.example/raven' })
  })

  it('other text is text, trimmed, first 120 characters', () => {
    expect(classifyPaste(data([], '  Raven crit build  '))).toEqual({ kind: 'text', text: 'Raven crit build' })
    expect(classifyPaste(data([], 'see https://a.example and https://b.example'))).toEqual({ kind: 'text', text: 'see https://a.example and https://b.example' })
    expect(classifyPaste(data([], 'ftp://files.example/x'))).toEqual({ kind: 'text', text: 'ftp://files.example/x' })
    expect((classifyPaste(data([], 'x'.repeat(200))) as { text: string }).text).toHaveLength(120)
  })

  it('a file that is not an image is ignored; nothing at all is null', () => {
    const zip = new File([new Uint8Array([1])], 'mods.zip', { type: 'application/zip' })
    expect(classifyPaste(data([zip], 'Raven'))).toEqual({ kind: 'text', text: 'Raven' })
    expect(classifyPaste(data([zip]))).toBeNull()
    expect(classifyPaste(data([], '   '))).toBeNull()
  })

  it('classifyText reads typed text the same way', () => {
    expect(classifyText('https://youtu.be/dQw4w9WgXcQ')).toEqual({ kind: 'link', url: 'https://youtu.be/dQw4w9WgXcQ' })
    expect(classifyText(' Raven ')).toEqual({ kind: 'text', text: 'Raven' })
    expect(classifyText('')).toBeNull()
  })
})
