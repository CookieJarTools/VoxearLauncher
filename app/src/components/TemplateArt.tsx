/** Line drawings for the templates, until the engine ships thumbnails of its own. */
export function TemplateArt({ id }: { id: string }) {
  const accent = 'var(--accent)'
  const common = { fill: 'none', stroke: 'currentColor', strokeWidth: 1.4, strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const }
  const floor = (
    <g {...common} opacity={0.45}>
      <path d="M20 86 L80 62 L140 86" />
      <path d="M40 92 L80 70 L120 92" opacity={0.7} />
      <path d="M50 78 L110 78" opacity={0.6} />
      <path d="M80 62 V92" opacity={0.5} />
    </g>
  )
  switch (id) {
    case 'empty':
      return (
        <svg viewBox="0 0 160 100" width="100%" height="100%">
          {floor}
          <g {...common}>
            <rect x="66" y="30" width="22" height="16" rx="3" />
            <path d="M88 35 L98 30 V46 L88 41" />
          </g>
        </svg>
      )
    case '3d':
      return (
        <svg viewBox="0 0 160 100" width="100%" height="100%">
          {floor}
          <g {...common}>
            <path d="M80 34 L98 44 L80 54 L62 44 Z" stroke={accent} />
            <path d="M62 44 V62 L80 72 V54 M98 44 V62 L80 72" stroke={accent} />
            <circle cx="122" cy="22" r="6" />
            <path d="M122 10 V13 M122 31 V34 M110 22 H113 M131 22 H134" opacity={0.7} />
          </g>
          <ellipse cx="82" cy="76" rx="20" ry="3.5" fill="currentColor" opacity={0.12} />
        </svg>
      )
    case '2d':
      return (
        <svg viewBox="0 0 160 100" width="100%" height="100%">
          <g {...common}>
            <path d="M14 82 H146" opacity={0.45} />
            <rect x="34" y="64" width="36" height="8" rx="1" opacity={0.7} />
            <rect x="92" y="48" width="40" height="8" rx="1" opacity={0.7} />
            <rect x="48" y="44" width="12" height="16" rx="2" stroke={accent} />
            <path d="M51 50 h2 M56 50 h2" stroke={accent} />
            <circle cx="104" cy="36" r="3" />
            <circle cx="116" cy="36" r="3" />
            <path d="M24 26 h14 M28 22 h8" opacity={0.4} />
          </g>
        </svg>
      )
    case 'first-person':
      return (
        <svg viewBox="0 0 160 100" width="100%" height="100%">
          <g {...common} opacity={0.45}>
            <path d="M10 10 L62 40 M150 10 L98 40 M10 92 L62 62 M150 92 L98 62" />
            <rect x="62" y="40" width="36" height="22" />
          </g>
          <g {...common} stroke={accent}>
            <circle cx="80" cy="51" r="6" />
            <path d="M80 40 V44 M80 58 V62 M69 51 H73 M87 51 H91" />
          </g>
          <path {...common} d="M112 96 L120 74 L134 70 L142 96" opacity={0.7} />
        </svg>
      )
    case 'third-person':
      return (
        <svg viewBox="0 0 160 100" width="100%" height="100%">
          {floor}
          <g {...common} stroke={accent}>
            <circle cx="80" cy="36" r="5" />
            <path d="M80 41 V60 M80 60 L73 72 M80 60 L87 72 M71 48 H89" />
          </g>
          <g {...common} opacity={0.7}>
            <rect x="24" y="18" width="16" height="11" rx="2" />
            <path d="M40 22 L48 19 V29 L40 26" />
            <path d="M48 24 L72 34" strokeDasharray="2 3" opacity={0.7} />
          </g>
        </svg>
      )
    case 'top-down':
      return (
        <svg viewBox="0 0 160 100" width="100%" height="100%">
          <g {...common} opacity={0.4}>
            <path d="M30 14 V86 M55 14 V86 M80 14 V86 M105 14 V86 M130 14 V86 M20 25 H140 M20 50 H140 M20 75 H140" />
          </g>
          <g {...common}>
            <rect x="105" y="25" width="25" height="25" opacity={0.7} />
            <path d="M50 70 Q 70 70 78 52 T 112 38" strokeDasharray="2 3" />
          </g>
          <circle cx="48" cy="70" r="5" fill="none" stroke={accent} strokeWidth={1.6} />
          <path {...common} d="M114 34 L118 38 L114 42 M118 38 H106" />
        </svg>
      )
    default:
      return (
        <svg viewBox="0 0 160 100" width="100%" height="100%">
          {floor}
          <path {...common} d="M80 30 L96 39 L80 48 L64 39 Z" stroke={accent} />
        </svg>
      )
  }
}
