# How Compass maps framework routes

Compass detects supported framework registrations during local structural extraction and publishes them in `compass.graph/1`. Detection stays local: route packs do not download grammars, invoke language servers, or require credentials. This reference lists the route shapes Compass recognizes, the graph records it emits, and the cases it leaves unresolved.

> **Who this page is for:** Compass users and integrators who need to trace application entry points.
>
> **You will learn:** how route nodes and `routes_to` edges work, which framework forms Compass recognizes, and why some dynamic routes remain unresolved.
>
> **Prerequisites:** [Graph model](../concepts/graph-model.md).
>
> **Reading time:** about 8 minutes.

## Route records and edges

Compass records a route when it can connect a framework registration to source evidence. The record keeps the path, operation, declaration, and resolution state together so you can inspect the result without guessing.

Each registration becomes a `route` node with these details:

- **Framework and operation**: the framework family and HTTP, page, hook, messaging, or subscription operation
- **Path**: the normalized path plus the original path expression
- **Declaration**: the source scope and anchor that declared the route
- **Stages**: ordered middleware and handler stages, with the handler last
- **Resolution**: the exact target, bounded candidates, or an explicit unresolved state

A `routes_to` edge points from the route to each exact middleware and handler target. Compass preserves declaration order. It does not create an exact edge when several targets remain valid.

Configuration and file-convention routes keep the rule and source that produced them. Equivalent inputs receive deterministic route identities and ordering.

## Read a route through callers

The `callers` command follows incoming `calls` and `routes_to` edges. Use it to see code callers and URL registrations for the same handler:

```bash
compass callers UsersController.show --graph compass-out/graph.json
```

The output stays tied to source anchors. Open the listed file and line when you need to verify a registration or investigate an unresolved target.

## Supported route shapes

Compass activates a framework pack only when the repository contains direct evidence, such as an import, exact receiver, framework configuration file, or dependency-backed file convention. The lists below describe the supported registration shapes.

### Python web frameworks

- **Django**: `path`, `re_path`, legacy `url`, and `include` in an activated URL module; positional or named `route` and `view` arguments; function views, dotted string handlers, and class-based `.as_view()` handlers
- **Flask**: `Flask` and `Blueprint` route decorators, constructor and registration-time `url_prefix`, positional or named `rule`, and literal `methods` lists
- **FastAPI**: `FastAPI` and `APIRouter` decorators for `get`, `post`, `put`, `patch`, `delete`, `options`, `head`, and `trace`; `api_route` and `route` method lists; literal `path`; constructor and `include_router(prefix=...)` prefixes; `Depends` stages

### JavaScript and TypeScript frameworks

- **Express**: `express()` and `Router()` receivers; `get`, `post`, `put`, `patch`, `delete`, `options`, `head`, and `all`; literal paths and ordered middleware chains; opaque inline callbacks remain unresolved
- **Fastify**: `fastify()` receivers; HTTP method calls and `route({ method, url, handler })` objects; literal `prefix` registrations, ordered hook stages such as `preHandler`, and opaque inline callbacks remain unresolved
- **Hono**: `new Hono()` receivers; HTTP method calls, `on([methods], path, handler)` arrays, `basePath` chains, and literal `route(prefix, child)` mounts with ordered middleware stages
- **Next.js**: App Router `page.*` and `route.*` files under `app` or `src/app`, Pages Router pages and `pages/api` handlers, dynamic segments, route groups, named HTTP exports, and project activation from the `next` dependency or `next.config.*`
- **Remix**: flat and nested route modules under `app/routes`, `routes`, or `src/routes`; `_index`, dotted nested segments, `$param` and splat names; and `PAGE`, `LOADER`, and `ACTION` operations from default, loader, and action exports. Project activation uses `@remix-run/*` dependencies or `remix.config.*`.
- **NestJS**: `Controller` HTTP method decorators and `RequestMapping`; GraphQL `Resolver` `Query` and `Mutation` operations at `/graphql`; typed GraphQL field details; `WebSocketGateway` `SubscribeMessage`; `MessagePattern` and `EventPattern` transport registrations
- **React Router**: JSX `Route` elements with `element={<Component />}` or `Component={Component}`, plus literal object route configs with `component`, `element`, or `Component` targets and loader or action stages
- **SvelteKit**: `src/routes` `+page.svelte` components and `+server` endpoints, including `[param]` and `[...rest]` segments and source-backed exported HTTP methods; `+page.ts` load modules are not pages
- **Vue Router and Nuxt**: Vue Router literal route objects; Nuxt `pages` components; `server/api` method-suffixed endpoints; dynamic segments; and route-middleware domain facts
- **Astro**: `src/pages` `.astro` pages and `.ts` or `.js` endpoints, including `[param]` and `[...rest]` segments, exported HTTP methods, `ALL`, and source-backed default handlers

Vite configuration is represented as a `configuration` node rather than a
route. The node retains bounded `resolve.alias`, plugin imports, and
configuration keys so build-time wiring remains inspectable without inventing
HTTP endpoints.

### PHP, Ruby, and JVM frameworks

- **Laravel**: exact `Illuminate\\Support\\Facades\\Route` receivers, including aliases; HTTP, `match`, `any`, `prefix(...)->group`, `resource`, `apiResource`, and `only` or `except` resource modifiers; `Controller@action` and controller or action tuple handlers
- **Drupal**: `*.routing.yml` and `*.routing.yaml` paths with controller, form, entity-form, entity-view, or entity-list handlers; pipe or comma-separated `_method` values; documented `hook_*` implementations and matching functions in `.module`, `.theme`, `.install`, and `.inc` files
- **Rails**: HTTP and `match` declarations inside `Rails.application.routes.draw`, `to:` and hash-rocket handlers, literal `scope` and `namespace` prefixes with namespaced controller owners, and `via` method lists
- **Spring**: Java and Kotlin controller mappings, including class and method composition, HTTP mapping annotations, `RequestMapping` methods, composed and inherited Java mappings, constants, packages, and overloaded handler signatures
- **Play**: literal `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, and `HEAD` entries in `conf/routes`, targeting Java, Scala, or injected controller actions

### Go, Rust, and native server frameworks

- **Gin, chi, and gorilla/mux**: imported router registrations, grouped or closure prefixes, Gin middleware chains, chi method calls, and gorilla `HandleFunc(...).Methods(...)`, including `PathPrefix(...).Subrouter()` chains
- **Axum, actix-web, and Rocket**: Axum nested `.nest(...).route(...)` chains, actix scoped resources and `.route(...).to(...)` handlers, plus Rocket and actix route attributes, including multiline attributes, guarded by framework imports or qualified macros
- **Vapor**: grouped literal and path-component prefixes, closure groups, `app.on(...)`, and HTTP registrations with explicit `use:` handlers; opaque closures remain visible as unresolved handlers
- **ASP.NET Core**: MVC controller and action templates, `[controller]` and `[action]` tokens, HTTP method attributes, and absolute `/` or `~/` action-template overrides

## Special route contracts

Some frameworks expose routes that are not HTTP URLs. Compass keeps their domain meaning instead of assigning a misleading path:

- NestJS messaging and WebSocket subscriptions retain their typed messaging or subscription contracts
- Nuxt middleware retains its middleware domain fact
- GraphQL fields retain the field name in route details while sharing the `/graphql` transport endpoint

These records still retain handler references and source anchors. File-route packs require matching project dependency or framework-configuration evidence during a normal repository build. Direct single-file extraction remains available for fixtures and tooling, but it does not activate a repository pack by itself.

## Why a route can remain unresolved

Compass publishes a route only when its evidence identifies a framework and a target within bounded limits. The following forms do not become exact route bindings:

- computed or concatenated paths
- dynamic method names and arbitrary metaprogramming
- opaque closures with no source-backed callable
- same-named handlers with more than one valid target
- targets selected only by repository-wide terminal-name similarity
- file-route conventions without matching project dependency evidence

An unresolved route remains visible on its `route` node. Compass keeps bounded candidates when available and omits a misleading `routes_to` edge. Limit errors remain explicit diagnostics, never empty successful results. Django include cycles stop without inventing a route.

## A practical route investigation workflow

Use this sequence when you need to explain how a request reaches a handler:

1. Run `compass update . --no-viz` to publish a local graph snapshot
2. Run `compass callers UsersController.show --graph compass-out/graph.json` to list code callers and route registrations
3. Open each source anchor and compare the declaration with the route path, stages, and resolution state
4. Treat a missing `routes_to` edge as unresolved evidence, not as proof that the route has no handler

## Related pages

- [Graph model](../concepts/graph-model.md)
- [Provenance](../concepts/provenance.md)
- [Outputs](outputs.md)

**Next step:** build a graph, then run `compass callers` for a known controller or view.
