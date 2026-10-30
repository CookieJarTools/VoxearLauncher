import type { CSSProperties } from 'react'

/** Stroke icons on a 24px grid, in the same style as the other CookieJar apps. */
const PATHS = {
  folder: 'M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6z',
  folderOpen: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v1M3 7v10a2 2 0 0 0 2 2h12l4-8H7l-4 8',
  cube: 'M12 3l8 4.5v9L12 21l-8-4.5v-9zM12 12l8-4.5M12 12v9M12 12L4 7.5',
  grid: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
  settings: 'M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M10 15v4',
  plus: 'M12 5v14M5 12h14',
  search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-4.5-4.5',
  star: 'M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z',
  more: 'M5 12h.01M12 12h.01M19 12h.01',
  play: 'M7 4l13 8-13 8z',
  download: 'M12 4v11m0 0l-4-4m4 4l4-4M5 20h14',
  trash: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13',
  close: 'M6 6l12 12M18 6L6 18',
  check: 'M4 12l5 5L20 6',
  chevron: 'M9 6l6 6-6 6',
  chevronDown: 'M6 9l6 6 6-6',
  external: 'M14 5h5v5M19 5l-8 8M12 5H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-6',
  refresh: 'M20 11a8 8 0 1 0-1 5m1 4v-5h-5',
  alert: 'M12 4l9 16H3zM12 10v4M12 17h.01',
  info: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11v5M12 8h.01',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  file: 'M6 3h8l4 4v14H6zM14 3v4h4',
  spinner: 'M12 3a9 9 0 1 0 9 9',
  command: 'M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z',
  winMin: 'M5 12h14',
  winMax: 'M5 5h14v14H5z'
}

export type IconName = keyof typeof PATHS

export function Icon({
  name,
  size = 16,
  strokeWidth = 1.7,
  filled = false,
  className,
  style
}: {
  name: IconName
  size?: number
  strokeWidth?: number
  filled?: boolean
  className?: string
  style?: CSSProperties
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? 'currentColor' : 'none'}
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      style={style}
      aria-hidden
    >
      <path d={PATHS[name]} />
    </svg>
  )
}

/** The Voxear mark: one voxel, lit from above. */
export function Logo({ size = 20 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden>
      <path d="M512 236 L758 378 L512 520 L266 378 Z" fill="#b3b9f5" />
      <path d="M266 378 L512 520 L512 804 L266 662 Z" fill="#7480e0" />
      <path d="M512 520 L758 378 L758 662 L512 804 Z" fill="#4a539f" />
    </svg>
  )
}
