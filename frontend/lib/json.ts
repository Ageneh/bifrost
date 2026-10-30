/**
 * Dotted-path helpers for comparing and merging configuration documents.
 *
 * The server reports changes as dotted paths (`z2m.garage.url`); resolving a
 * draft conflict per path needs the same addressing on the client.
 */

/** A JSON object, which is all a configuration document can be at the root. */
export type JsonObject = Record<string, unknown>

export function isObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Read the value at a dotted path, or `undefined` when it is absent. */
export function getPath(document: unknown, path: string): unknown {
  let current: unknown = document
  for (const key of splitPath(path)) {
    if (!isObject(current) || !(key in current)) {
      return undefined
    }
    current = current[key]
  }
  return current
}

/** Write a value at a dotted path, creating intermediate objects. */
export function setPath(document: JsonObject, path: string, value: unknown): void {
  const keys = splitPath(path)
  if (keys.length === 0) {
    return
  }
  let current: JsonObject = document
  for (const key of keys.slice(0, -1)) {
    const next = current[key]
    if (!isObject(next)) {
      const created: JsonObject = {}
      current[key] = created
      current = created
    }
    else {
      current = next
    }
  }
  current[keys[keys.length - 1] as string] = value
}

/** Remove the value at a dotted path; empty parents stay, so the shape holds. */
export function deletePath(document: JsonObject, path: string): void {
  const keys = splitPath(path)
  if (keys.length === 0) {
    return
  }
  let current: JsonObject = document
  for (const key of keys.slice(0, -1)) {
    const next = current[key]
    if (!isObject(next)) {
      return
    }
    current = next
  }
  Reflect.deleteProperty(current, keys[keys.length - 1] as string)
}

/** Every leaf path in a document, in a stable order. */
export function leafPaths(document: unknown, prefix = ''): string[] {
  if (!isObject(document)) {
    return prefix === '' ? [] : [prefix]
  }
  const paths: string[] = []
  for (const key of Object.keys(document).sort()) {
    const path = prefix === '' ? key : `${prefix}.${key}`
    const value = document[key]
    if (isObject(value) && Object.keys(value).length > 0) {
      paths.push(...leafPaths(value, path))
    }
    else {
      paths.push(path)
    }
  }
  return paths
}

/** Paths whose values differ between two documents. */
export function changedPaths(left: unknown, right: unknown): string[] {
  const paths = new Set([...leafPaths(left), ...leafPaths(right)])
  return [...paths]
    .filter(path => !deepEqual(getPath(left, path), getPath(right, path)))
    .sort()
}

/** Structural equality for JSON values. */
export function deepEqual(left: unknown, right: unknown): boolean {
  if (left === right) {
    return true
  }
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((item, index) => deepEqual(item, right[index]))
  }
  if (isObject(left) && isObject(right)) {
    const leftKeys = Object.keys(left)
    const rightKeys = Object.keys(right)
    return (
      leftKeys.length === rightKeys.length
      && leftKeys.every(key => key in right && deepEqual(left[key], right[key]))
    )
  }
  return false
}

/** Deep copy of a JSON document. */
export function clone<T>(value: T): T {
  return value === undefined ? value : (JSON.parse(JSON.stringify(value)) as T)
}

function splitPath(path: string): string[] {
  return path.split('.').filter(segment => segment !== '')
}
