# Framework-aware routes

Compass detects supported framework registrations during local structural
extraction and publishes them in `compass.graph/1`. No framework detector
downloads a grammar, invokes a language server, or requires credentials.

> **Who this page is for:** Compass users and integrators querying application
> entry points.
>
> **You will learn:** which route forms are recognized, how they appear in the
> graph, and which dynamic forms intentionally remain unresolved.
>
> **Prerequisites:** [Graph model](../concepts/graph-model.md).
>
> **Reading time:** about 8 minutes.

## Graph contract

Each HTTP, page, or hook registration becomes a `route` node with its
framework, operation, normalized path, original path, declaring scope, source
anchor, ordered stages, and resolution state. A `routes_to` edge points from
the route to each exact middleware and handler target. Middleware positions
preserve declaration order; the handler is the final stage.

An ambiguous or unresolved handler remains recorded on the route node, with
bounded candidates when available, but does not receive an invented exact
edge. Config- and convention-derived routes retain the rule and source that
produced them. Equivalent inputs have deterministic route identities and
ordering.

`callers` follows incoming `calls` and `routes_to` edges. After building a
graph, this surfaces both code callers and URL registrations for a handler:

```bash
compass callers UsersController.show --graph compass-out/graph.json
```

## Supported route shapes

| Framework | Recognized shapes |
| --- | --- |
| Django | `path`, `re_path`, legacy `url`, and `include` in an activated URL module; positional or named `route`/`view` arguments; function views, dotted string handlers, and class-based `.as_view()` handlers |
| Flask | `Flask` and `Blueprint` `@receiver.route` decorators, constructor and registration-time `url_prefix`, positional or named `rule`, and literal `methods` lists |
| FastAPI | `FastAPI` and `APIRouter` decorators for `get`, `post`, `put`, `patch`, `delete`, `options`, `head`, and `trace`; `api_route`/`route` literal method lists; positional or named `path`; constructor and `include_router(prefix=...)` prefixes and `Depends` stages |
| Express | `express()` and `Router()` receivers; `get`, `post`, `put`, `patch`, `delete`, `options`, `head`, and `all`; literal paths, ordered middleware chains, and opaque inline callbacks that remain unresolved |
| NestJS | `Controller` with HTTP method decorators and `RequestMapping`; GraphQL `Resolver` with `Query`/`Mutation` operations at the `/graphql` endpoint plus a typed field detail; `WebSocketGateway` `SubscribeMessage`; `MessagePattern` and `EventPattern` transport registrations |
| Laravel | Exact `Illuminate\\Support\\Facades\\Route` receivers, including aliases; HTTP, `match`, `any`, `prefix(...)->group`, `resource`/`apiResource`, and `only`/`except` resource modifiers; `Controller@action` and controller/action tuple handlers |
| Drupal | `*.routing.yml`/`*.routing.yaml` paths with controller, form, entity-form, entity-view, or entity-list handlers; pipe- or comma-separated `_method` values; documented `Implements hook_*().` functions and functions named `hook_*` in `.module`, `.theme`, `.install`, and `.inc` files |
| Rails | HTTP and `match` declarations inside `Rails.application.routes.draw`, `to:` and hash-rocket handlers, literal `scope`/`namespace` prefixes with namespaced controller owners, and `via` method lists |
| Spring | Java and Kotlin controller mappings, including class/method composition, HTTP mapping annotations, `RequestMapping` methods, composed and inherited Java mappings, constants, packages, and overloaded handler signatures |
| Play | Literal `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, and `HEAD` entries in `conf/routes`, targeting Java, Scala, or injected controller actions |
| Gin, chi, gorilla/mux | Imported router registrations, grouped/closure prefixes, Gin middleware chains, chi method calls, and gorilla `HandleFunc(...).Methods(...)` (including `PathPrefix(...).Subrouter()` chains) |
| Axum, actix-web, Rocket | Axum nested `.nest(...).route(...)` chains, actix scoped resources and `.route(...).to(...)` handlers, plus Rocket and actix route attributes (including multiline attributes), guarded by framework imports or qualified macros |
| ASP.NET Core | MVC controller/action templates, `[controller]` and `[action]` tokens, HTTP method attributes, and absolute `/` or `~/` action-template overrides |
| Vapor | Grouped literal/path-component prefixes, closure groups, `app.on(...)`, and HTTP registrations with explicit `use:` handlers; opaque closures remain visible as unresolved handlers |
| React Router | JSX `Route` elements using `element={<Component />}` or `Component={Component}`, and literal object route configs with `component`, `element`, or `Component` targets and loader/action stages |
| SvelteKit | `src/routes` `+page.svelte` components and `+server` endpoints, including `[param]` and `[...rest]` segments and source-backed exported HTTP methods; `+page.ts` load modules are not pages |
| Vue Router and Nuxt | Vue Router literal route objects; Nuxt `pages` components, `server/api` method-suffixed endpoints, dynamic segments, and route-middleware domain facts |
| Astro | `src/pages` `.astro` pages and `.ts`/`.js` endpoints, including `[param]` and `[...rest]` segments, exported HTTP methods, `ALL`, and source-backed default handlers |

File-route packs require matching project dependency evidence during a normal
repository build. Direct single-file extraction without project evidence is
available for fixtures and tooling, but does not weaken repository activation.

NestJS messaging, WebSocket subscriptions, and Nuxt middleware use their typed
messaging/domain contracts rather than being assigned fictitious HTTP URLs.
GraphQL fields retain their field name in route details while sharing the
transport endpoint path. They still retain handler references and source
anchors.

## Conservative boundaries

Compass publishes a route only when the framework is activated by direct
evidence such as an import, exact receiver, framework config filename, or
dependency-backed file convention. A filename or decorator name alone does not
activate most source packs.

The following forms intentionally do not become exact route bindings:

- computed or concatenated paths;
- dynamic method names and arbitrary metaprogramming;
- opaque closures when no source-backed callable can be identified;
- same-named handlers with more than one valid target;
- route targets selected only by repository-wide terminal-name similarity; and
- file-route conventions without matching project dependency evidence.

Framework resolution is bounded by per-file fact, candidate, alias-expansion,
and include-depth limits. Exceeding a limit is an explicit failure or
diagnostic, never an empty successful result. Django include cycles stop without
inventing a route, and ambiguous handlers preserve their candidate state
without publishing a misleading `routes_to` edge.

## Related pages

- [Graph model](../concepts/graph-model.md)
- [Provenance](../concepts/provenance.md)
- [Outputs](outputs.md)
- [Universal semantic evidence](universal-semantic-evidence.md)

**Next step:** build a graph and run `compass callers` for a known controller or
view to inspect its code callers and framework registrations together.
