/**
 * Types the Rust side does not generate.
 *
 * `Update` and its `HueEvent` arm are not exported by `cargo test --features
 * ts`: the event stream carries the whole Hue resource model, which the UI
 * never renders, so the block is hand-written down to the fields the shell
 * actually reads.
 */
import type { AppConfig } from './generated/AppConfig'
import type { InventoryUpdate } from './generated/InventoryUpdate'
import type { LinkButtonStatus } from './generated/LinkButtonStatus'
import type { Service } from './generated/Service'

/**
 * One block of the Hue event stream, as `Update::HueEvent` serializes it:
 * `{ creationtime, id, type, data[] }`, each datum carrying its own `id`,
 * optional `id_v1`, `type` and the changed fields flattened beside them.
 */
export interface HueEventBlock {
  creationtime: string
  id: string
  type: 'add' | 'update' | 'delete' | 'error'
  data?: HueEventDatum[]
}

export interface HueEventDatum {
  id: string
  id_v1?: string
  type: string
  [field: string]: unknown
}

/** Everything `/bifrost/ws` sends. */
export type Update
  = | { AppConfig: AppConfig }
    | { HueEvent: HueEventBlock }
    | { ServiceUpdate: Service }
    | { Inventory: InventoryUpdate }
    | { LinkButton: LinkButtonStatus }

/**
 * One path of a draft conflict, resolved by the user.
 *
 * `keep` keeps the local value for the path and re-sends it on top of the
 * other side's revision; `take` drops the local value and accepts theirs.
 */
export interface ConflictResolution {
  path: string
  choice: 'keep' | 'take'
}

/** What the UI knows about an apply that may outlive the socket. */
export interface ApplyAttempt {
  /** Identifies this apply across a socket drop. */
  applyId: string
  /** Draft revision the apply was made against. */
  revision: number
  startedAt: string
}
