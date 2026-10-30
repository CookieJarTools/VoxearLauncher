import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useStore } from '../store'
import { Icon } from './Icon'
import { Rich } from './Rich'

export function Menu() {
  const menu = useStore((s) => s.menu)
  const close = useStore((s) => s.closeMenu)
  const ref = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null)

  // Keep the menu on screen: open leftwards from the anchor, and upwards near the bottom.
  useLayoutEffect(() => {
    if (!menu || !ref.current) return setPosition(null)
    const { width, height } = ref.current.getBoundingClientRect()
    const left = Math.max(8, Math.min(menu.x - width, window.innerWidth - width - 8))
    const top = menu.y + height > window.innerHeight - 8 ? Math.max(8, menu.y - height - 8) : menu.y
    setPosition({ left, top })
  }, [menu])

  useEffect(() => {
    if (!menu) return
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && close()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [menu, close])

  if (!menu) return null
  return (
    <div className="menu-layer" onMouseDown={close} onContextMenu={(e) => (e.preventDefault(), close())}>
      <div
        ref={ref}
        className="menu"
        style={position ?? { left: menu.x, top: menu.y, visibility: 'hidden' }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {menu.items.map((item, index) =>
          item === 'divider' ? (
            <div key={index} className="menu-divider" />
          ) : (
            <button
              key={index}
              className={`menu-item ${item.danger ? 'danger' : ''}`}
              onClick={() => {
                close()
                item.run()
              }}
            >
              {item.icon && <Icon name={item.icon} size={15} />}
              {item.label}
            </button>
          )
        )}
      </div>
    </div>
  )
}

export function Toasts() {
  const toasts = useStore((s) => s.toasts)
  const dismiss = useStore((s) => s.dismissToast)
  return (
    <div className="toasts" role="status">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast toast-${toast.kind}`}>
          <Icon className="lead" name={toast.kind === 'ok' ? 'check' : toast.kind === 'error' ? 'alert' : 'info'} size={16} />
          <p>
            <Rich text={toast.message} />
          </p>
          {toast.action && (
            <button
              className="btn btn-sm"
              onClick={() => {
                dismiss(toast.id)
                toast.action?.run()
              }}
            >
              {toast.action.label}
            </button>
          )}
          <button className="icon-btn" title="Dismiss" onClick={() => dismiss(toast.id)}>
            <Icon name="close" size={13} />
          </button>
        </div>
      ))}
    </div>
  )
}

export function Confirm() {
  const request = useStore((s) => s.confirmRequest)
  const confirmRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!request) return
    confirmRef.current?.focus()
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && request.resolve(false)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [request])

  if (!request) return null
  return (
    <div className="overlay" onMouseDown={() => request.resolve(false)}>
      <div className="dialog confirm" onMouseDown={(e) => e.stopPropagation()} role="alertdialog">
        <div className="dialog-head">
          <div className="dialog-title">{request.title}</div>
        </div>
        <div className="dialog-body">
          <p>{request.message}</p>
        </div>
        <div className="dialog-foot">
          <div className="spacer" />
          <button className="btn btn-ghost" onClick={() => request.resolve(false)}>
            Cancel
          </button>
          <button
            ref={confirmRef}
            className={`btn ${request.danger ? 'btn-danger' : 'btn-primary'}`}
            onClick={() => request.resolve(true)}
          >
            {request.confirm}
          </button>
        </div>
      </div>
    </div>
  )
}
