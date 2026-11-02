/**
 * What the console's status column shows, when the page has something for
 * it.
 *
 * The column belongs to the shell, so a page cannot slot into it directly.
 * A page instead publishes the component and the props the column should
 * render, and clears it when nothing is selected. Listeners travel as
 * `on…` props, which is what a component bound with `v-bind` expects.
 */
import { ref, shallowRef } from 'vue'
import type { Component } from 'vue'

const component = shallowRef<Component | null>(null)
const panelProps = ref<Record<string, unknown>>({})

export function useStatusPanel() {
  /** Show `next` in the status column, with `props`. */
  function show(next: Component, props: Record<string, unknown> = {}): void {
    component.value = next
    panelProps.value = props
  }

  /** Update the props of what is already shown, keeping the component. */
  function update(props: Record<string, unknown>): void {
    panelProps.value = props
  }

  /** Put the column back to what the shell renders by itself. */
  function clear(): void {
    component.value = null
    panelProps.value = {}
  }

  return { component, panelProps, show, update, clear }
}
