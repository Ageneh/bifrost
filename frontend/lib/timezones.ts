/**
 * The timezones the bridge screen offers.
 *
 * The browser already carries the IANA database, so the list comes from
 * `Intl.supportedValuesOf('timeZone')` rather than a copy that would go
 * stale. An engine without that call (Safari before 15.4) gets a short list
 * of common zones; either way the value the bridge already holds is kept, so
 * saving never silently changes a timezone the list does not know.
 */

export interface TimezoneOption {
  value: string
  label: string
}

/** Used only when the engine cannot enumerate its own zone list. */
const FALLBACK = [
  'UTC',
  'Europe/Berlin',
  'Europe/London',
  'Europe/Amsterdam',
  'Europe/Vienna',
  'Europe/Zurich',
  'America/New_York',
  'America/Chicago',
  'America/Denver',
  'America/Los_Angeles',
  'Australia/Sydney',
  'Asia/Tokyo',
]

function supported(): string[] {
  const intl = Intl as unknown as { supportedValuesOf?: (key: string) => string[] }
  try {
    return intl.supportedValuesOf?.('timeZone') ?? FALLBACK
  }
  catch {
    return FALLBACK
  }
}

/** Every option, with `current` first when the list does not contain it. */
export function timezoneOptions(current: string): TimezoneOption[] {
  const zones = supported()
  const all = current !== '' && !zones.includes(current) ? [current, ...zones] : zones
  return all.map(zone => ({ value: zone, label: zone }))
}
