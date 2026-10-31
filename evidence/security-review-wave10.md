# Security review, wave 10 (R2)

Date: 2026-10-31. Reviewer: the wave-10 implementation worker.
Scope: waves 7 (entertainment) and 9 (sensors) plus the whole admin surface
(`/bifrost/**`, setup mode, the websocket, the identity bundle, export and
reset). Read-only review, with the checks below made executable where they
were not already.

## Method

1. Enumerate every route under `/bifrost` from the sources and probe it
   without a session, for every method. The scan reads `.route(..)` from
   `src/routes`, so a route added in a new file is covered without anyone
   updating a list.
2. Probe the access guard's decisions: ingress trust by peer address,
   `Host` allow-list, session cookie, CSRF on mutations, `Origin` on
   mutations and websocket upgrades.
3. Read the entertainment and sensor paths for client-controlled allocation,
   secret handling and state changes that outlive a request.
4. Capture the z2m websocket handshake, which is where the z2m 2.x token
   leaves the bridge, and grep every fixture for the captured value.
5. Re-read the secret surfaces: configuration export, state export, log
   buffer, identity bundle.

## Verified controls

| Control | Evidence |
|---|---|
| Every non-public `/bifrost` route answers 401 without a session, for every method | `routes::bifrost::tests::every_bifrost_route_is_guarded` |
| An unknown `/bifrost/**` path is 401 without a session and 404 with one, never the SPA shell | `unknown_bifrost_paths_are_guarded_and_never_html`, `unknown_bifrost_paths_are_404_with_a_session` |
| `bifrost.auth` cannot be changed through the draft or import API | `routes::bifrost::config::tests::draft_refuses_auth_changes_and_keeps_auth_on_round_trip` |
| Ingress is believed only from a peer in `trusted_cidrs`; an untrusted `X-Ingress-Path` is 403 over a real socket | `tests/integration.rs::ingress_header_from_an_untrusted_peer_is_refused` |
| A trusted ingress peer skips the login; everyone else needs a session | `tests/integration.rs::ingress_header_from_a_trusted_peer_skips_the_login` |
| Mutations need the session's CSRF header and a same-origin `Origin` | `routes::bifrost::tests::mutation_needs_session_then_csrf_then_origin` |
| A session cookie is useless from another address | `routes::bifrost::tests::session_cookie_is_useless_from_another_address` |
| The z2m token travels on the websocket handshake and is in no fixture | `tests/security.rs::the_z2m_token_travels_on_the_websocket_handshake`, `no_fixture_contains_the_token_the_handshake_carried` |
| Configuration export omits `bifrost.auth` and every z2m token | `routes::bifrost::config::tests::export_omits_auth_and_z2m_tokens` |
| The identity bundle (private key included) is served only over HTTPS, with `no-store`, and holds no z2m token or password | `routes::bifrost::identity::get_backup` (code path), review read |
| Log lines are scrubbed of `token=`, `password:`, `Bearer …` before the buffer stores them | `logbuffer::tests::secrets_are_redacted_before_they_are_stored` |
| Setup mode's listener binds the default route's interface, never the wildcard | `routes::bifrost::setup::tests::the_setup_listener_is_not_a_wildcard` |
| State reset and identity changes require the bridge id typed as confirmation | `routes::bifrost::state::post_reset`, `routes::bifrost::identity::post_mac` (code path), review read |

## Findings

No new defect was found in waves 7, 9 or the admin surface. Three behaviours
are recorded here so a later reader does not re-litigate them as findings:

1. **The SSDP listener binds `0.0.0.0:1900`.** That is the discovery
   protocol's documented multicast socket, not an admin listener, and it
   predates this programme. The "no wildcard listener" rule in ticket 194 is
   about the setup listener that lets the Supervisor gateway reach the panel;
   that listener binds the default-route interface (test above). The SSDP
   socket is the one documented exception and is not changed here.
2. **A session holder can stop the HTTP service they are using.** The
   services API exposes start/stop for every registered service, including
   `http` and `https`. This is a self-lockout, not a privilege boundary: an
   authenticated owner can already restart the bridge from the same screen.
   No change; noted so a future report does not call it an escalation.
3. **A failing z2m connection can print its url, token included, to the
   process log.** The buffer that `/bifrost/logs` and the websocket serve is
   scrubbed (`token=` is redacted before storage), so the admin API does not
   expose it; standard output is the operator's own terminal. No change.

## Residual risk

- Ingress trust is an address check, as Home Assistant's own proxy model
  requires. A process that can spoof the Supervisor's address on the Docker
  network defeats it; that is a network-boundary assumption, not a bug here.
- The admin UI over plain HTTP exposes the password to anyone on the LAN.
  Documented in the plan's risks and in `doc/home-assistant.md`; the HTTPS
  listener is the mitigation.
- Hardware sign-offs from waves 6 and 8 are not re-taken by this review; no
  regression was found in their code paths, but the wave report names them.

## Follow-ups

None filed. The two behaviours above are documented, not deferred work.
