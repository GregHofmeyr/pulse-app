// PNG byte fixtures for tests: chunks with a few bytes of data each (CRCs are not checked). Not used by the app.

const SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10]

/** One chunk: length, tag, three bytes of data, a placeholder CRC. */
export function pngChunk(tag: string): Uint8Array {
  return new Uint8Array([0, 0, 0, 3, ...[...tag].map((c) => c.charCodeAt(0)), 1, 2, 3, 0, 0, 0, 0])
}

/** A PNG made of these chunks. */
export const pngOf = (tags: string[]) => new Uint8Array([...SIGNATURE, ...tags.flatMap((t) => [...pngChunk(t)])])

/** The chunk tags of a PNG, in order. */
export function pngTags(png: Uint8Array): string[] {
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength)
  const tags: string[] = []
  for (let at = 8; at + 12 <= png.length; at += 12 + view.getUint32(at)) tags.push(String.fromCharCode(...png.subarray(at + 4, at + 8)))
  return tags
}
