# Features

- Keep the orchestration engine solution-agnostic. Sources, blocks, capabilities, route
  slots, lifecycles, probes, and deployments are generic concepts; Java, Python, UI,
  database, JAS, paths, ports, and environment variables are user configuration only.
- Provide versioned adapter interfaces for source providers, execution runtimes,
  supervisors, routing mechanisms, and health probes. Adapters publish schemas used by
  both CLI validation and automatically generated GUI controls.
- Support ordered deployment overlays that inject environment variables, explicit
  dotenv files, generated or copied configuration files, parameters, and route choices.
  Use overlays to create multiple variations of the same base product.
- Resolve overlays deterministically with validation, per-value origin tracking, and a
  complete preview of shadowed values, file changes, routes, resource claims, and whether
  applying each change requires a live reload, restart, or rebuild.
- Materialize injected files outside source worktrees and present them through adapter
  mounts or bindings. Support portable committed overlays, ignored machine-local
  overlays, and apply-time secret references without writing secrets into resolved
  manifests.
- Run three isolated containers in one private network.
- One container runs the shared PostgreSQL database. It is exposed to the two service
  containers, but not publicly exposed on the host.
- The other two containers run separate copies of the same service image and use the
  same shared database.
- Interact with both service copies through Vercel Labs Portless on the same proxy port
  using `http://app-one.localhost:1355` and `http://app-two.localhost:1355`.
- Keep the complete setup rootless: no hosts-file changes, external DNS, privileged
  ports, or local certificate-authority installation.
- Allow other devices on the same local network to reach both service copies through
  Portless LAN mode at `http://app-one.local:1355` and
  `http://app-two.local:1355`.
- Discover the Docker host automatically over mDNS, without configuring hosts files on
  each client device.
- Document and validate the LAN prerequisites: TCP port `1355` and mDNS UDP port `5353`
  must be allowed by the host firewall and network; Linux hosts require
  `avahi-publish-address` from `avahi-utils`.
- Treat LAN access as optional because mDNS normally works only on the same subnet and
  may not cross guest Wi-Fi isolation, VLANs, VPNs, or routed networks. Use normal DNS
  or a private networking solution such as Tailscale for cross-network access.
- Allow every startup block component to run either as a normal Docker image/Dockerfile
  service or as a declared script inside an isolated runner container. Containerized
  scripts may be long-running services or one-shot tasks.
- Support explicitly trusted host-command blocks for existing workflows such as
  `/zfs/projects/FR/jasBase/start-jas-service.sh` and `process-compose -f
  ai-services.process-compose.yaml up` with `/zfs/projects/FR/jasBase` as the working
  directory.
- Treat Process Compose as a first-class host adapter: preserve its child-process
  dependencies, readiness checks, logs, and ordered shutdown in the GUI and CLI.
- Require host commands to declare their working directory, environment, lifecycle,
  shutdown behavior, ports, and other exclusive resources. Reject deployments with
  collisions, including multiple copies of scripts that still use the same fixed ports.
- Use the parent workspace as the reference mixed-runtime deployment: databases managed
  through `./start-local-jas.sh`, Java through `start-jas-service.sh`, Python services
  through `ai-services.process-compose.yaml`, and independently selectable UI sources.
- Keep this JAS deployment under `examples/` as a generic integration fixture. It must
  not introduce JAS-specific modules or conditionals into the orchestrator.
- Represent startup, readiness, post-ready initialization, shutdown, and cleanup as
  separate lifecycle phases. The legacy `start-local-jas.sh` must be wrapped for a
  single instance or decomposed before multiple database instances are allowed because
  it currently recreates global Docker resources and waits for JAS itself.
- Route by typed dependency slots so consumers can select any compatible Java, Python,
  UI, or database instance regardless of how that instance was started. Route changes
  must declare whether they apply live or require a consumer restart or rebuild.
