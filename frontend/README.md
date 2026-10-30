# Bifrost web UI

The admin console for Bifrost: a Nuxt 3 single-page application with hash
routing, served by the Rust binary at `/` and, through Home Assistant, under
an ingress prefix.

## Commands

Run from the repository root:

```sh
pnpm --dir frontend install
pnpm --dir frontend lint
pnpm --dir frontend typecheck
pnpm --dir frontend build          # writes .output/public
pnpm --dir frontend test:e2e       # Playwright, 390 px, behind a prefix
```

`pnpm --dir frontend build` runs `nuxt generate` and then
`scripts/postbuild.mjs`. The script rewrites the generated HTML to relative
asset paths (`./_nuxt/...`) and the runtime `baseURL` to `./`, because Nuxt
normalises `app.baseURL: './'` back to `/`. Absolute asset paths break behind
the ingress prefix, where the browser would ask the proxy for `/_nuxt/...`
instead of `<prefix>/_nuxt/...`.

The server injects `<base href>` and `window.__BIFROST_BASE__` from
`X-Ingress-Path` at request time; both end in `/`. `lib/api.ts` reads the
variable and falls back to `./` for a static build opened from disk.

## TypeScript bindings

`lib/generated/` is generated from the Rust wire types, never edited by hand:

```sh
cargo test --features ts --test generated_types
```

The `ts` feature on `bifrost-api` and `svc` derives `ts-rs` bindings; the test
writes them into `lib/generated` and deletes files for types that no longer
exist. CI regenerates and fails on a diff, so the checked-in copy always
matches the server. `lib/generated/index.ts` is written by hand and
re-exports the set the app imports.

Types the UI does not consume are not generated. `Update::HueEvent` is one:
the event stream carries the whole Hue resource model, so `lib/types.ts`
hand-writes the block down to the fields the shell reads.

## Layout

- `components/` — the shell and its primitives (`AppShell`, `SideNav`,
  `StatusPill`, `StatTile`, `DataTable`, `AsyncState`, `AttentionRow`,
  `ImpactList`, `ConfirmDialog`, `ToastHost`, `ConfigField`) and the draft
  surface (`DraftBar`, `DraftPill`, `DiffDrawer`, `ChangeRow`,
  `ApplyProgress`, `RestartBanner`).
- `composables/` — `useApi` (session and client), `useDraft` (revision, 409
  conflicts, apply), `useLiveSocket` (the `/bifrost/ws` connection),
  `useToast`.
- `assets/css/` — the Flight Deck tokens and component classes. Every colour,
  size and radius is a token; components carry no raw hex values.
- `e2e/` — the Playwright cases; `scripts/serve.mjs` stands in for the Rust
  server and injects the same base elements, so the tests run behind a real
  path prefix.

## Narrow screens

Below 1000 px the status column stacks under the primary view and the draft
moves into a floating `DraftPill`. Below 600 px the rail becomes a bottom tab
bar and tables collapse to one card per row, with the column name repeated in
each cell.
