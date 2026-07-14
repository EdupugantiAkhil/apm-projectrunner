# Features

- Run three isolated containers in one private network.
- One container runs the shared PostgreSQL database. It is exposed to the two service
  containers, but not publicly exposed on the host.
- The other two containers run separate copies of the same service image and use the
  same shared database.
- Interact with both service copies through Vercel Labs Portless on the same proxy port
  using `http://app-one.localhost:1355` and `http://app-two.localhost:1355`.
- Keep the complete setup rootless: no hosts-file changes, external DNS, privileged
  ports, or local certificate-authority installation.
