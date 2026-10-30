/**
 * The toast queue. `ToastHost` renders it; anything that finishes a mutation
 * pushes a line here instead of alerting.
 */
import { readonly, ref } from 'vue'

export type ToastKind = 'info' | 'ok' | 'crit'

export interface Toast {
  id: number
  kind: ToastKind
  message: string
}

/** How long a toast stays up; errors stay until dismissed. */
const LIFETIME_MS = 6000

const toasts = ref<Toast[]>([])
let nextId = 1

export function useToast() {
  function push(message: string, kind: ToastKind = 'info'): number {
    const id = nextId++
    toasts.value = [...toasts.value, { id, kind, message }]
    if (kind !== 'crit') {
      setTimeout(() => dismiss(id), LIFETIME_MS)
    }
    return id
  }

  function dismiss(id: number): void {
    toasts.value = toasts.value.filter(toast => toast.id !== id)
  }

  return {
    toasts: readonly(toasts),
    push,
    dismiss,
    ok: (message: string) => push(message, 'ok'),
    fail: (message: string) => push(message, 'crit'),
  }
}
