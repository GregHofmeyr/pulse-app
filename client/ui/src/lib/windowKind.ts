// Which window this page is (#620): the app, or the build pop-out (the core opens it on `index.html?window=build`).
export type WindowKind = 'main' | 'build'

export function windowKind(loc: Pick<Location, 'search'>): WindowKind {
  return new URLSearchParams(loc.search).get('window') === 'build' ? 'build' : 'main'
}
