const relative = new Intl.RelativeTimeFormat('en', { numeric: 'auto' })

/** "just now", "5 minutes ago", "yesterday", "3 weeks ago". */
export function ago(iso: string | null | undefined): string {
  if (!iso) return ''
  const seconds = (new Date(iso).getTime() - Date.now()) / 1000
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [
    ['year', 31_536_000],
    ['month', 2_592_000],
    ['week', 604_800],
    ['day', 86_400],
    ['hour', 3_600],
    ['minute', 60]
  ]
  for (const [unit, size] of steps) {
    if (Math.abs(seconds) >= size) return relative.format(Math.round(seconds / size), unit)
  }
  return 'just now'
}

export function bytes(count: number): string {
  if (count < 1024) return `${count} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = count / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

/** A path with the home folder shown as `~`. */
export function pretty(path: string, home: string | undefined): string {
  if (home && home !== '/' && (path === home || path.startsWith(home + '/') || path.startsWith(home + '\\'))) {
    return '~' + path.slice(home.length)
  }
  return path
}

export function joinPath(folder: string, name: string): string {
  const separator = folder.includes('\\') && !folder.includes('/') ? '\\' : '/'
  return folder.replace(/[\\/]+$/, '') + separator + name
}

/** A stable hue for a name, so each project keeps its colour. */
export function hueOf(name: string): number {
  let hash = 0
  for (const char of name) hash = (hash * 31 + char.charCodeAt(0)) >>> 0
  return hash % 360
}

/** Fuzzy match: every character of `query` in order. Returns a score, higher is better, or -1. */
export function fuzzy(query: string, text: string): number {
  const q = query.trim().toLowerCase()
  if (!q) return 0
  const t = text.toLowerCase()
  const at = t.indexOf(q)
  if (at >= 0) return 1000 - at
  let score = 0
  let from = 0
  let previous = -2
  for (const char of q) {
    const found = t.indexOf(char, from)
    if (found < 0) return -1
    score += found === previous + 1 ? 8 : 1
    if (found === 0 || ' -_/.'.includes(t[found - 1])) score += 6
    previous = found
    from = found + 1
  }
  return score
}
